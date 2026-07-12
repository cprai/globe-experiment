//! [`PtzCamera`]: the pan/tilt/zoom orbital camera - rig math plus all input
//! and animation state (drag pan with flick inertia, drag tilt, smoothed
//! wheel zoom). World space is km; the rig lives in the inertial (star-fixed)
//! frame, so `longitude`/`latitude` select an inertial viewing direction, not
//! geography (see `.claude/rules/camera.md`). The orbit target is NOT stored
//! here: the scene owns it and passes it into every call that scales by or
//! centers on the orbited body, so rig state and orbit subject cannot drift
//! apart. Limits scale with the target's mean radius, so the feel is the
//! same fraction of whichever body is orbited.

use std::time::Instant;

use glam::{DMat3, DQuat, DVec3};

use super::{CameraControl, CameraView, CursorHint, PointerButton, ScrollDelta};
use crate::engine::scene::celestial_sphere::CelestialSphere;
use crate::engine::scene::{
    CameraTarget, CelestialBody, RenderState, Scene, SceneClock, SceneKinematicBodies,
    SceneOrbitalBodies,
};

/// Minimum release speed, in px/s, for a drag to keep coasting.
const FLICK_SPEED: f64 = 50.0;
/// Coasting stops once it decays below this speed, in px/s.
const STOP_SPEED: f64 = 15.0;
/// Coasting velocity halves every this many seconds.
const HALF_LIFE: f64 = 0.3;
/// A release later than this after the last cursor move is a hold, not a
/// flick, in seconds.
const FLICK_TIMEOUT: f64 = 0.1;
/// Bounds on the zoom glide's half-life, in seconds. The half-life tracks
/// the smoothed wheel-event gap: dense events zoom near-instantly, sparse
/// ones (momentum tails, single notches) interpolate across the gap.
const ZOOM_HALF_LIFE_MIN: f64 = 0.01;
const ZOOM_HALF_LIFE_MAX: f64 = 0.1;
/// Cap on a wheel-event gap sample, in seconds; longer pauses just mean
/// "fresh scroll", not an extremely slow cadence.
const WHEEL_GAP_CAP: f64 = 0.25;
/// The zoom coast velocity halves every this many seconds once wheel
/// events stop feeding it.
const ZOOM_COAST_HALF_LIFE: f64 = 0.15;
/// Coasting stops below this zoom rate, in natural-log distance per
/// second (0.1 ~ 10% of the camera distance per second).
const ZOOM_STOP_RATE: f64 = 0.1;

/// The interactive pan/tilt/zoom orbital camera: the rig pose plus the input
/// and animation state that moves it.
pub struct PtzCamera {
    /// Longitude of the inertial viewing direction, in degrees.
    pub longitude: f64,
    /// Latitude of the inertial viewing direction, in degrees. Clamped +/-89.
    pub latitude: f64,
    /// Distance from the camera to the look-at point, in kilometers.
    pub distance: f64,
    /// Angle off straight-down (nadir), in degrees. 0 looks straight down.
    pub tilt: f64,
    /// Last known pointer position; presses carry no position of their own.
    cursor: Option<(f64, f64)>,
    drag: Option<Drag>,
    inertia: Option<Inertia>,
    zoom: Option<Zoom>,
    last_wheel: Option<Instant>,
    /// Smoothed gap between recent wheel events, in seconds.
    wheel_gap: f64,
}

struct Drag {
    button: PointerButton,
    last: (f64, f64),
    /// Smoothed cursor velocity, in px/s.
    velocity: (f64, f64),
    moved_at: Instant,
}

struct Inertia {
    /// Remaining pan velocity, in px/s.
    velocity: (f64, f64),
    tick: Instant,
}

struct Zoom {
    /// Camera distance the glide is heading toward, in kilometers.
    target: f64,
    /// The glide's current half-life, in seconds; follows the wheel cadence.
    half_life: f64,
    /// Smoothed rate at which wheel events move the target, in natural-log
    /// distance per second. Keeps the target advancing between and after
    /// events, so motion never pauses while the device (or its momentum
    /// tail) is between deliveries.
    velocity: f64,
    /// Log-distance the target has been advanced by `velocity` since the
    /// last wheel event. The next event repays it, so bridged motion is
    /// never counted twice.
    bridged: f64,
    tick: Instant,
}

impl Default for PtzCamera {
    fn default() -> Self {
        Self {
            longitude: 0.0,
            latitude: 0.0,
            // ~2 Terra radii above the surface: the whole body in view.
            distance: Self::DEFAULT_DISTANCE_RADII * CelestialBody::TERRA.mean_radius_km(),
            tilt: 0.0,
            cursor: None,
            drag: None,
            inertia: None,
            zoom: None,
            last_wheel: None,
            wheel_gap: 0.0,
        }
    }
}

impl PtzCamera {
    // Distance limits as fractions of the target body's mean radius.
    const MIN_DISTANCE_RADII: f64 = 0.01;
    const MAX_DISTANCE_RADII: f64 = 10.0;
    const DEFAULT_DISTANCE_RADII: f64 = 2.0;
    const MAX_TILT: f64 = 80.0;

    /// A camera with an explicit pose, the distance clamped into `target`'s
    /// valid range (the input-state fields are private, so no struct literal
    /// works from outside this module).
    #[allow(dead_code)] // constructed only by the headless bin's tree
    pub fn new(
        target: &CameraTarget,
        longitude: f64,
        latitude: f64,
        distance: f64,
        tilt: f64,
    ) -> Self {
        let mut camera = Self {
            longitude,
            latitude,
            distance,
            tilt,
            ..Self::default()
        };
        camera.distance = camera.clamp_distance(target, distance);
        camera
    }

    /// Closest the eye may sit above the target surface, in km.
    fn min_distance(&self, target: &CameraTarget) -> f64 {
        Self::MIN_DISTANCE_RADII * target.mean_radius_km()
    }

    /// Farthest the eye may sit from the target surface, in km.
    fn max_distance(&self, target: &CameraTarget) -> f64 {
        Self::MAX_DISTANCE_RADII * target.mean_radius_km()
    }

    /// The full-body framing distance for the target, in km.
    fn default_distance(&self, target: &CameraTarget) -> f64 {
        Self::DEFAULT_DISTANCE_RADII * target.mean_radius_km()
    }

    /// Moves the look-at point by the given degrees, wrapping longitude
    /// across the dateline and clamping latitude short of the poles.
    fn pan(&mut self, dlon: f64, dlat: f64) {
        self.longitude = (self.longitude + dlon + 180.0).rem_euclid(360.0) - 180.0;
        self.latitude = (self.latitude + dlat).clamp(-89.0, 89.0);
    }

    /// Clamps a camera distance into the target's radius-scaled limits.
    fn clamp_distance(&self, target: &CameraTarget, distance: f64) -> f64 {
        distance.clamp(self.min_distance(target), self.max_distance(target))
    }

    /// Adjusts the tilt, clamped between straight-down and near-horizon.
    fn tilt_by(&mut self, degrees: f64) {
        self.tilt = (self.tilt + degrees).clamp(0.0, Self::MAX_TILT);
    }

    /// Degrees of arc panned per pixel of cursor movement, scaled so the
    /// ground under the cursor approximately follows it at any altitude.
    fn pan_degrees_per_pixel(&self, target: &CameraTarget, viewport_height: f64) -> f64 {
        // Ground arc length spanned by one pixel at the target plane, in km.
        let km_per_pixel = 2.0
            * self.distance
            * (crate::engine::renderer::FOV_Y_DEG / 2.0)
                .to_radians()
                .tan()
            / viewport_height.max(1.0);

        // One radian of arc subtends ~one mean radius of surface distance, so
        // scaling by the target's radius keeps the pan feel right on any body.
        (km_per_pixel / target.mean_radius_km()).to_degrees()
    }

    /// The (eye, look-at, up) rig in the floating-origin (render) frame.
    /// `celestial_to_world` rotates the inertial (star) frame the rig lives
    /// in into the Earth-fixed world frame. The look direction is carried as
    /// the look-at *point*, not a normalized forward: re-normalizing and
    /// re-projecting drifts by sub-ULP and speckles antialiased edges.
    pub fn world_rig(
        &self,
        target: &CameraTarget,
        celestial: &CelestialSphere,
        celestial_to_world: DMat3,
    ) -> (DVec3, DVec3, DVec3) {
        self.world_frame_relative(target, celestial, celestial_to_world)
    }

    /// An inertial camera orbiting `target` whose look axis points along
    /// `world_look` (an Earth-fixed world-frame direction), viewed from
    /// `distance` km with no tilt. `star_rot_inv` is the world->celestial
    /// rotation. Seeds a launch framing; the camera stays interactive after.
    pub fn looking_toward(
        target: &CameraTarget,
        star_rot_inv: DMat3,
        world_look: DVec3,
        distance: f64,
    ) -> Self {
        // The rig's look axis resolved into the world is
        // `-star_rot_inv^T * radial`; setting it equal to `world_look` gives
        // `radial = -(star_rot_inv * world_look)`.
        let radial = -(star_rot_inv * world_look.normalize_or_zero());
        let mut camera = Self {
            longitude: radial.x.atan2(radial.z).to_degrees(),
            latitude: radial.y.clamp(-1.0, 1.0).asin().to_degrees(),
            distance,
            tilt: 0.0,
            ..Self::default()
        };
        camera.distance = camera.clamp_distance(target, distance);
        camera
    }

    /// Reframes the camera onto a just-switched orbit target: full-frame
    /// distance, zero tilt, re-aimed at an off-Terra body's center, in-flight
    /// animation dropped (it still targets the old body's scale). Called by
    /// the scene only on a genuine body switch.
    pub fn reframe(
        &mut self,
        target: &CameraTarget,
        celestial: &CelestialSphere,
        celestial_to_world: DMat3,
    ) {
        self.distance = self.default_distance(target);
        self.tilt = 0.0;
        self.reset_animation();

        // Aim along the GEOCENTRIC (Terra-relative) direction, not the raw
        // heliocentric center, so it is frame-agnostic: Terra -> zero (skip,
        // any inertial direction frames Terra), Luna/planets -> Terra->body.
        let terra = celestial.center_world(CelestialBody::TERRA);
        let aim = target.center_world(celestial) - terra;
        if aim != DVec3::ZERO {
            let star_rot_inv = celestial_to_world.transpose();
            let radial = -(star_rot_inv * aim.normalize_or_zero());
            self.longitude = radial.x.atan2(radial.z).to_degrees();
            self.latitude = radial.y.clamp(-1.0, 1.0).asin().to_degrees();
        }
    }

    /// The rig in the floating-origin (render) frame: inertial offsets
    /// rotated into the world, shifted by `(center - render_origin)`.
    /// Computed this way - never as `absolute_eye - render_origin` - so a far
    /// planet's billions-of-km absolute position is never formed and cannot
    /// cancel catastrophically in f32.
    fn world_frame_relative(
        &self,
        target: &CameraTarget,
        celestial: &CelestialSphere,
        celestial_to_world: DMat3,
    ) -> (DVec3, DVec3, DVec3) {
        let (eye, look_at, up) = self.frame(target);
        // f64 subtraction cancels the ~1.5e8 km heliocentric Sol offset
        // exactly (zero for the orbited body).
        let shift = target.center_world(celestial) - target.render_origin(celestial);
        (
            shift + celestial_to_world * eye,
            shift + celestial_to_world * look_at,
            celestial_to_world * up,
        )
    }

    /// The camera's (eye, target, up) in the inertial (star) frame, as
    /// offsets from the target body's center.
    fn frame(&self, target: &CameraTarget) -> (DVec3, DVec3, DVec3) {
        let lat = self.latitude.clamp(-89.0, 89.0).to_radians();
        let lon = self.longitude.to_radians();

        // Look-at point on the target surface (km) and the geodetic normal
        // there, which the eye offsets along.
        let look_at = target.surface_position(lat, lon);
        let radial = target.geodetic_normal(lat, lon);

        // Local tangent frame at the look-at point.
        let east = DVec3::Y.cross(radial).normalize();
        let north = radial.cross(east);

        // Tilt swings the camera off straight-down around the local east
        // axis, so increasing tilt reveals the horizon to the north.
        let tilt = DQuat::from_axis_angle(east, self.tilt.to_radians());
        let eye = look_at + tilt * radial * self.distance;
        let up = tilt * north;

        (eye, look_at, up)
    }

    /// A pointer button went down at the last known cursor position.
    pub fn pointer_press(&mut self, button: PointerButton) {
        let Some(position) = self.cursor else {
            return;
        };
        // Grabbing the body stops any coasting.
        self.inertia = None;
        self.drag = Some(Drag {
            button,
            last: position,
            velocity: (0.0, 0.0),
            moved_at: Instant::now(),
        });
    }

    /// A pointer button was released; a fast left release becomes a flick.
    pub fn pointer_release(&mut self, button: PointerButton) {
        let releases_drag = self.drag.as_ref().is_some_and(|drag| drag.button == button);

        if !releases_drag {
            return;
        }

        let Some(drag) = self.drag.take() else {
            return;
        };

        if drag.button == PointerButton::Left {
            let (vx, vy) = drag.velocity;
            let speed = vx.hypot(vy);
            let held = Instant::now()
                .saturating_duration_since(drag.moved_at)
                .as_secs_f64();

            if speed > FLICK_SPEED && held < FLICK_TIMEOUT {
                self.inertia = Some(Inertia {
                    velocity: drag.velocity,
                    tick: Instant::now(),
                });
            }
        }
    }

    /// The pointer moved: track it, and while a drag is active pan (left) or
    /// tilt (right) the rig with it.
    pub fn pointer_move(
        &mut self,
        target: &CameraTarget,
        position: (f64, f64),
        viewport_height: f64,
    ) {
        self.cursor = Some(position);

        // Taken out and put back (not held as `&mut`): the drag borrow cannot
        // live across the `self.pan`/`self.tilt_by` calls below.
        let Some(mut drag) = self.drag.take() else {
            return;
        };

        let dx = position.0 - drag.last.0;
        let dy = position.1 - drag.last.1;
        drag.last = position;

        let now = Instant::now();
        let dt = now
            .saturating_duration_since(drag.moved_at)
            .as_secs_f64()
            .max(1e-4);
        drag.moved_at = now;

        // EMA of cursor velocity; the blend weight is time-based so the
        // smoothing is frame-rate independent.
        let alpha = 1.0 - (-dt * 20.0).exp();
        let (vx, vy) = drag.velocity;
        drag.velocity = (vx + (dx / dt - vx) * alpha, vy + (dy / dt - vy) * alpha);

        match drag.button {
            // Drag moves the body with the cursor: dragging right
            // pulls the view west, dragging down pulls it north.
            PointerButton::Left => {
                let scale = self.pan_degrees_per_pixel(target, viewport_height);
                self.pan(-dx * scale, dy * scale);
            }
            // Dragging up tilts toward the horizon.
            PointerButton::Right => self.tilt_by(-dy * 0.25),
        }

        self.drag = Some(drag);
    }

    /// A scroll event: move the zoom glide's target (see the `Zoom` docs;
    /// per-frame motion happens in `tick`).
    pub fn scroll(&mut self, target: &CameraTarget, delta: ScrollDelta) {
        let ticks = match delta {
            ScrollDelta::Lines(y) => y,
            ScrollDelta::Pixels(y) => y / 60.0,
        };

        let now = Instant::now();
        let gap = self.last_wheel.map_or(WHEEL_GAP_CAP, |last| {
            now.saturating_duration_since(last)
                .as_secs_f64()
                .min(WHEEL_GAP_CAP)
        });
        self.last_wheel = Some(now);

        // Track the event cadence; the 0.5 blend adapts within a
        // few events when a scroll speeds up or trails off.
        self.wheel_gap += (gap - self.wheel_gap) * 0.5;

        let half_life = self.wheel_gap.clamp(ZOOM_HALF_LIFE_MIN, ZOOM_HALF_LIFE_MAX);

        // Events only move the target; `tick` glides the camera toward it.
        // An in-flight glide keeps its clock: resetting it per event would
        // stall the glide between dense events.
        let delta = ticks * 0.9f64.ln();

        // Taken out and put back: the zoom borrow cannot live across the
        // `self.clamp_distance` calls below.
        match self.zoom.take() {
            Some(mut zoom) => {
                // Reversing direction kills the coast outright.
                if delta * zoom.velocity < 0.0 {
                    zoom.velocity = 0.0;
                    zoom.bridged = 0.0;
                }

                // Repay target motion the coast already applied since the
                // last event; only the remainder moves the target now. An
                // over-bridged surplus carries forward against the next event.
                let outstanding = if delta * zoom.bridged > 0.0 {
                    let remaining = delta - zoom.bridged;
                    if remaining * delta > 0.0 {
                        zoom.bridged = 0.0;
                        remaining
                    } else {
                        zoom.bridged -= delta;
                        0.0
                    }
                } else {
                    delta
                };

                zoom.target = self.clamp_distance(target, zoom.target * outstanding.exp());
                zoom.half_life = half_life;

                // Velocity follows the event rate; time-based blend, like
                // the drag velocity EMA.
                let alpha = 1.0 - (-gap * 20.0).exp();
                let rate = delta / gap.max(1e-3);
                zoom.velocity += (rate - zoom.velocity) * alpha;

                self.zoom = Some(zoom);
            }
            None => {
                // A first event carries no rate information, so the glide
                // starts without coast velocity.
                self.zoom = Some(Zoom {
                    target: self.clamp_distance(target, self.distance * delta.exp()),
                    half_life,
                    velocity: 0.0,
                    bridged: 0.0,
                    tick: now,
                });
            }
        }
    }

    /// Advances one frame of camera animation: flick coasting and the
    /// zoom glide.
    pub fn tick(&mut self, target: &CameraTarget, viewport_height: f64) {
        self.tick_coast(target, viewport_height);
        self.tick_zoom(target);
    }

    /// Integrates one frame of flick coasting.
    fn tick_coast(&mut self, target: &CameraTarget, viewport_height: f64) {
        // Taken out and put back unless it stopped: the inertia borrow
        // cannot live across the `self.pan` call.
        let Some(mut inertia) = self.inertia.take() else {
            return;
        };

        let now = Instant::now();
        let dt = now
            .saturating_duration_since(inertia.tick)
            .as_secs_f64()
            .min(0.1);
        inertia.tick = now;

        let scale = self.pan_degrees_per_pixel(target, viewport_height);
        let (vx, vy) = inertia.velocity;
        self.pan(-vx * dt * scale, vy * dt * scale);

        let decay = 0.5f64.powf(dt / HALF_LIFE);
        inertia.velocity = (vx * decay, vy * decay);

        if vx.hypot(vy) * decay >= STOP_SPEED {
            self.inertia = Some(inertia);
        }
    }

    /// Moves the camera one frame closer to the zoom target.
    fn tick_zoom(&mut self, target: &CameraTarget) {
        // Taken out and put back unless it settled: the zoom borrow cannot
        // live across the `self.clamp_distance` call.
        let Some(mut zoom) = self.zoom.take() else {
            return;
        };

        let now = Instant::now();
        let dt = now
            .saturating_duration_since(zoom.tick)
            .as_secs_f64()
            .min(0.1);
        zoom.tick = now;

        // Coast: keep the target moving at the wheel-established rate,
        // decaying once events stop - this carries motion across the silence
        // between a finger lift and the first momentum-tail event (without
        // it the glide drains its target and visibly stalls). The advance is
        // logged in `bridged` and repaid by the next event.
        zoom.velocity *= 0.5f64.powf(dt / ZOOM_COAST_HALF_LIFE);
        if zoom.velocity.abs() > ZOOM_STOP_RATE {
            let step = zoom.velocity * dt;
            zoom.target = self.clamp_distance(target, zoom.target * step.exp());
            zoom.bridged += step;
        } else {
            zoom.velocity = 0.0;
        }

        // Exponential approach in log space - zoom is multiplicative, so
        // this keeps the glide's perceived speed uniform at any altitude.
        let blend = 1.0 - 0.5f64.powf(dt / zoom.half_life);
        let ratio = zoom.target / self.distance;
        self.distance *= ratio.powf(blend);

        if (ratio - 1.0).abs() < 1e-3 && zoom.velocity == 0.0 {
            self.distance = zoom.target;
        } else {
            self.zoom = Some(zoom);
        }
    }

    /// Drops any in-flight flick inertia and zoom glide (they target the old
    /// body's distance scale, so left running they would fight a reframe).
    fn reset_animation(&mut self) {
        self.inertia = None;
        self.zoom = None;
    }

    /// The scene cursor for the current drag state.
    pub fn cursor_hint(&self) -> CursorHint {
        if self.drag.is_some() {
            CursorHint::Grabbing
        } else {
            CursorHint::Grab
        }
    }
}

/// The camera hookup for a scene that flies a [`PtzCamera`]: implement the
/// three accessors (normally `#[derive(ScenePtzCamera)]` over the scene's
/// `camera` + `camera_target` fields) and the blanket [`CameraControl`] impl
/// below supplies the whole input surface. Three, not one, because the
/// forwarding needs three shapes: `camera_mut` for the mutating calls,
/// `camera` for `cursor_hint`'s `&self`, and `camera_target` because
/// `pointer_move`/`scroll`/`tick` scale by the orbited body (the target
/// stays scene-owned). A scene that must diverge implements
/// [`CameraControl`] directly instead.
pub trait ScenePtzCamera {
    /// Where the camera lives in the scene struct (shared view).
    fn camera(&self) -> &PtzCamera;

    /// Where the camera lives in the scene struct (mutating view).
    fn camera_mut(&mut self) -> &mut PtzCamera;

    /// The scene-owned body the camera orbits this frame.
    fn camera_target(&self) -> &CameraTarget;
}

impl<S: ScenePtzCamera> CameraControl for S {
    fn pointer_press(&mut self, button: PointerButton) {
        self.camera_mut().pointer_press(button)
    }

    fn pointer_release(&mut self, button: PointerButton) {
        self.camera_mut().pointer_release(button)
    }

    fn pointer_move(&mut self, position: (f64, f64), viewport_height: f64) {
        // Copied out (`CameraTarget` is a small Copy identity) so the target
        // read does not hold `self` borrowed against `camera_mut`.
        let target = *self.camera_target();
        self.camera_mut()
            .pointer_move(&target, position, viewport_height)
    }

    fn scroll(&mut self, delta: ScrollDelta) {
        let target = *self.camera_target();
        self.camera_mut().scroll(&target, delta)
    }

    fn tick(&mut self, viewport_height: f64) {
        let target = *self.camera_target();
        self.camera_mut().tick(&target, viewport_height)
    }

    fn cursor_hint(&self) -> CursorHint {
        self.camera().cursor_hint()
    }
}

/// Every scene flying a [`PtzCamera`] produces its frame identically, so the
/// standard trait set also supplies [`CameraView`]. A scripted or fixed
/// camera implements [`CameraView`] directly instead of `ScenePtzCamera`.
impl<S> CameraView for S
where
    S: Scene + SceneClock + ScenePtzCamera + SceneOrbitalBodies + SceneKinematicBodies,
{
    fn frame_state(&mut self) -> RenderState {
        let now = self.clock_now();
        let sphere = CelestialSphere::at(&now);

        let celestial_to_world = sphere.star_rot_inv.transpose();
        let target = *self.camera_target();
        let (eye, look_at, up) = self
            .camera()
            .world_rig(&target, &sphere, celestial_to_world);

        // The render-frame eye doubles as `tracked_bodies`' geocentric eye:
        // exact for a Terra target, and every body-tracking scene orbits
        // Terra (body-less scenes never use it).
        let tracked_bodies = self.tracked_bodies(&now, eye);

        RenderState {
            time: now,
            camera_target: target,
            camera_pos: eye,
            camera_look_at: look_at,
            camera_up: up,
            tracked_bodies,
        }
    }
}
