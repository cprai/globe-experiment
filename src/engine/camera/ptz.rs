//! [`PtzCamera`]: the pan/tilt/zoom orbital camera - the rig math *and* the
//! input response that drives it (drag pan with flick inertia, drag tilt,
//! smoothed wheel zoom). One struct owns both halves so all camera state,
//! including in-flight animation, lives behind the `CameraControl` +
//! `CameraView` traits a scene forwards to; the application passes
//! translated input through and keeps none of it.
//!
//! The camera lives in the **inertial (star-fixed) frame** and orbits a
//! chosen subject (a [`CameraTarget`] - Terra, Luna, a planet, or a free
//! point). The target is NOT stored here: the scene owns it (alongside its
//! clock) and passes it into every call that scales by
//! or centers on the orbited body, so the rig state and the orbit subject
//! cannot drift apart across the scene/camera boundary.
//!
//! The orbital math builds the rig (eye/target/up) around the *target's
//! center* exactly as a surface-anchored camera would, but that rig is
//! interpreted in the celestial frame and rotated into the Earth-fixed world
//! frame (see `world_rig`; the renderer then builds the projection). So the
//! camera does not rotate with the world: it holds still relative to the
//! stars while the scene spins beneath it. The longitude/latitude therefore
//! select an inertial viewing direction, not a fixed geographic point. With a
//! Luna target the same construction keeps the eye pinned to Luna (look axis
//! is always `-c2w * radial`, toward Luna center) while it tracks Luna's
//! moving ephemeris position.
//!
//! World space is in kilometers; the `CelestialSphere` frame is heliocentric
//! (Sol at the origin), but the rig is always built in the target-local
//! render frame (`world_frame_relative` subtracts `render_origin`), so its
//! absolute origin choice is invisible here. The surface anchor and the
//! distance/near/pan limits are scaled by the *target's* mean radius (see
//! [`CameraTarget::mean_radius_km`]), so the interaction feel is the same
//! fraction of whichever body is orbited. For a Terra target the render
//! origin is Terra's center, so the rig is identical to the original
//! Terra-only camera.

use std::time::Instant;

use glam::{DMat3, DQuat, DVec3};

use super::{CameraControl, CursorHint, PointerButton, ScrollDelta};
use crate::engine::scene::celestial_sphere::CelestialSphere;
use crate::engine::scene::{CameraTarget, CelestialBody};

/// Minimum release speed, in px/s, for a drag to keep coasting.
const FLICK_SPEED: f64 = 50.0;
/// Coasting stops once it decays below this speed, in px/s.
const STOP_SPEED: f64 = 15.0;
/// Coasting velocity halves every this many seconds.
const HALF_LIFE: f64 = 0.3;
/// A release later than this after the last cursor move is a hold, not a
/// flick, in seconds.
const FLICK_TIMEOUT: f64 = 0.1;
/// Bounds on the zoom glide's half-life, in seconds. The glide adapts to
/// the wheel-event cadence: its half-life tracks the smoothed gap between
/// recent events, so dense events (active trackpad scrolling) zoom
/// near-instantly while sparse ones (a trackpad's synthesized momentum
/// tail, single mouse-wheel notches) are interpolated across the gap that
/// would otherwise show as a step.
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
/// and animation state that moves it (left-drag pan with flick inertia,
/// right-drag tilt, wheel zoom glide).
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
    /// The glide's current half-life, in seconds; follows the wheel
    /// cadence at the time of the last event.
    half_life: f64,
    /// Smoothed rate at which wheel events move the target, in
    /// natural-log distance per second. Keeps the target advancing
    /// between and after events, so the motion never pauses while the
    /// device (or its momentum tail) is between deliveries.
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
    // Distance/default limits as fractions of the *target body's* mean radius
    // (was a hard-coded Terra radius). ~0.01 radii above the surface up to ~10
    // radii out; the default view sits ~2 radii above the surface. The FOV, near
    // plane, and far plane now live with the projection in `renderer` (the
    // renderer rebuilds view_proj from the rig this camera emits).
    const MIN_DISTANCE_RADII: f64 = 0.01;
    const MAX_DISTANCE_RADII: f64 = 10.0;
    const DEFAULT_DISTANCE_RADII: f64 = 2.0;
    const MAX_TILT: f64 = 80.0;

    /// A camera with an explicit pose, the distance clamped into `target`'s
    /// valid range. The headless binary's constructor (it builds the pose from
    /// its `--scene` JSON; a struct literal no longer works from outside this
    /// module because the input-state fields are private).
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

    /// The full-body framing distance for the target, in km. Used when a body
    /// switch reframes the camera.
    fn default_distance(&self, target: &CameraTarget) -> f64 {
        Self::DEFAULT_DISTANCE_RADII * target.mean_radius_km()
    }

    /// Moves the look-at point by the given degrees, wrapping longitude
    /// across the dateline and clamping latitude short of the poles.
    fn pan(&mut self, dlon: f64, dlat: f64) {
        self.longitude = (self.longitude + dlon + 180.0).rem_euclid(360.0) - 180.0;
        self.latitude = (self.latitude + dlat).clamp(-89.0, 89.0);
    }

    /// Clamps a camera distance to lie between just above the target surface
    /// and a full-body view; the limits scale with the target's radius.
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

        // Convert that arc length to an angle on the body: one radian of arc
        // subtends ~one mean radius of surface distance, so scaling by the
        // target's radius keeps the pan feel right on Luna as on Terra.
        (km_per_pixel / target.mean_radius_km()).to_degrees()
    }

    /// The camera rig for the renderer: the eye, the look-at point, and the up
    /// vector, all in the floating-origin (render) frame.
    /// `celestial_to_world` is the rotation from the inertial (star) frame the
    /// rig lives in to the Earth-fixed world frame the scene is drawn in (the
    /// inverse of the celestial sphere's world->celestial rotation); applying
    /// it keeps the camera fixed relative to the stars while the world
    /// rotates beneath it.
    ///
    /// The renderer rebuilds the view-projection matrix from these (see
    /// `renderer::view_proj_reversed_z`). The look direction is carried as the
    /// look-at *point* rather than a normalized forward vector so the
    /// renderer's `look_at_rh` reproduces the old `view_proj` bit-for-bit
    /// (re-normalizing a forward and re-projecting would drift by sub-ULP
    /// and speckle every antialiased edge). The eye is kept in the render
    /// frame - computed WITHOUT ever forming the absolute eye - because a
    /// far planet's absolute position is billions of km, so `absolute_eye -
    /// render_origin` in f32 would cancel catastrophically (the view
    /// translation snaps to ~hundreds of km and the camera jitters). For
    /// Terra/Luna (render_origin 0) the render frame is the absolute frame.
    pub fn world_rig(
        &self,
        target: &CameraTarget,
        celestial: &CelestialSphere,
        celestial_to_world: DMat3,
    ) -> (DVec3, DVec3, DVec3) {
        self.world_frame_relative(target, celestial, celestial_to_world)
    }

    /// An inertial camera that orbits `target` and whose look axis points along
    /// `world_look` - a direction in the Earth-fixed world frame (e.g. toward
    /// Sol's day side or toward Luna) - viewed from `distance` km with
    /// no tilt. `star_rot_inv` is the celestial sphere's world->celestial
    /// (equatorial) rotation, mapping the world direction back into the
    /// inertial frame the rig is built in. Used by the eclipse scenes to
    /// frame their event on launch; the camera stays fully interactive
    /// afterward.
    pub fn looking_toward(
        target: &CameraTarget,
        star_rot_inv: DMat3,
        world_look: DVec3,
        distance: f64,
    ) -> Self {
        // The rig's look axis (toward the target) is `-radial`; resolved into
        // the world it is `-celestial_to_world * radial = -star_rot_inv^T *
        // radial`. Setting that equal to `world_look` gives
        // `radial = -(star_rot_inv * world_look)` - the inertial direction the
        // eye sits along.
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

    /// Reframes the camera onto a just-switched orbit target: resets to the
    /// body's full-frame distance and zero tilt, re-aims at an off-Terra
    /// body's center, and drops any in-flight zoom or flick (which still
    /// targets the old body's scale). The scene owns the [`CameraTarget`]
    /// and calls this only on a genuine **body switch** (`same_kind`); no
    /// per-frame call is needed - the target's moving center is resolved
    /// from the sphere inside every `world_rig` call. Keeping the existing
    /// longitude/latitude for a Terra switch is fine: any inertial direction
    /// frames Terra at the origin.
    pub fn reframe(
        &mut self,
        target: &CameraTarget,
        celestial: &CelestialSphere,
        celestial_to_world: DMat3,
    ) {
        self.distance = self.default_distance(target);
        self.tilt = 0.0;
        self.reset_animation();

        // Aim at the body: for an off-Terra target (Luna or a planet) look
        // toward it along the GEOCENTRIC (Terra-relative) direction, the
        // same mapping `looking_toward` uses. A Terra target keeps the
        // existing longitude/latitude (any inertial direction frames Terra
        // at its own center). The direction is Terra-relative, not the raw
        // heliocentric `center_world`, so it is frame-agnostic: Terra ->
        // zero (skip), Luna/planets -> the Terra->body direction.
        let terra = celestial.center_world(CelestialBody::TERRA);
        let aim = target.center_world(celestial) - terra;
        if aim != DVec3::ZERO {
            let star_rot_inv = celestial_to_world.transpose();
            let radial = -(star_rot_inv * aim.normalize_or_zero());
            self.longitude = radial.x.atan2(radial.z).to_degrees();
            self.latitude = radial.y.clamp(-1.0, 1.0).asin().to_degrees();
        }
    }

    /// The rig in the **floating-origin (render) frame**: the inertial offsets
    /// rotated into the world and shifted by `(center - render_origin)`, so the
    /// orbited body sits at/near the numerical origin. Computed this way
    /// (rather than forming the absolute rig and subtracting
    /// `render_origin`) so a far planet never goes through a billions-of-km
    /// f32 value: for the orbited body `center == render_origin`, so the
    /// shift is exactly zero and the rig is just `celestial_to_world *
    /// offset` (local, precise). For Terra/Luna (render_origin 0) the shift
    /// is the body center, so this equals the absolute rig - bit-identical
    /// to the pre-planet renderer.
    fn world_frame_relative(
        &self,
        target: &CameraTarget,
        celestial: &CelestialSphere,
        celestial_to_world: DMat3,
    ) -> (DVec3, DVec3, DVec3) {
        let (eye, look_at, up) = self.frame(target);
        // The f64 subtraction cancels the ~1.5e8 km heliocentric Sol offset
        // exactly (zero for the orbited body).
        let shift = target.center_world(celestial) - target.render_origin(celestial);
        (
            shift + celestial_to_world * eye,
            shift + celestial_to_world * look_at,
            celestial_to_world * up,
        )
    }

    /// Computes the camera's (eye, target, up) in the inertial (star) frame,
    /// as offsets from the target body's center.
    fn frame(&self, target: &CameraTarget) -> (DVec3, DVec3, DVec3) {
        let lat = self.latitude.clamp(-89.0, 89.0).to_radians();
        let lon = self.longitude.to_radians();

        // Look-at point on the target surface (km) and the local "up" - the
        // geodetic normal there, which the eye offsets along.
        let look_at = target.surface_position(lat, lon);
        let radial = target.geodetic_normal(lat, lon);

        // Local tangent frame at the look-at point.
        let east = DVec3::Y.cross(radial).normalize();
        let north = radial.cross(east);

        // Tilt swings the camera away from straight-down, around the local
        // east axis, so increasing tilt reveals the horizon to the north.
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
        // live across the `self.pan`/`self.tilt_by` calls below now that the
        // rig and the input state share one struct.
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

        // Exponential moving average of cursor velocity; the blend
        // weight is time-based so the smoothing is frame-rate
        // independent.
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

    /// A scroll event: move the zoom glide's target (see the `Zoom` docs; the
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

        // Events only move the target; `tick` glides the camera
        // toward it each frame, paced to the event cadence. An
        // in-flight glide keeps its clock: resetting it per event
        // would stall the glide between dense events.
        let delta = ticks * 0.9f64.ln();

        // Taken out and put back: the zoom borrow cannot live across the
        // `self.clamp_distance` calls below now that the rig and the input
        // state share one struct.
        match self.zoom.take() {
            Some(mut zoom) => {
                // Reversing direction kills the coast outright.
                if delta * zoom.velocity < 0.0 {
                    zoom.velocity = 0.0;
                    zoom.bridged = 0.0;
                }

                // Repay target motion the coast already applied
                // since the last event; only the remainder moves
                // the target now. An over-bridged surplus carries
                // forward against the next event.
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

                // Velocity follows the event rate; time-based
                // blend, like the drag velocity EMA.
                let alpha = 1.0 - (-gap * 20.0).exp();
                let rate = delta / gap.max(1e-3);
                zoom.velocity += (rate - zoom.velocity) * alpha;

                self.zoom = Some(zoom);
            }
            None => {
                // A first event carries no rate information, so
                // the glide starts without coast velocity.
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
    /// zoom glide. Called from the redraw handler.
    pub fn tick(&mut self, target: &CameraTarget, viewport_height: f64) {
        self.tick_coast(target, viewport_height);
        self.tick_zoom(target);
    }

    /// Integrates one frame of flick coasting.
    fn tick_coast(&mut self, target: &CameraTarget, viewport_height: f64) {
        // Taken out and put back unless it stopped: the inertia borrow cannot
        // live across the `self.pan` call now that the rig and the input
        // state share one struct.
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
        // live across the `self.clamp_distance` call now that the rig and
        // the input state share one struct.
        let Some(mut zoom) = self.zoom.take() else {
            return;
        };

        let now = Instant::now();
        let dt = now
            .saturating_duration_since(zoom.tick)
            .as_secs_f64()
            .min(0.1);
        zoom.tick = now;

        // Coast: keep the target moving at the rate the wheel events
        // established, decaying once they stop. This is what carries the
        // motion across the silence between a finger lift and the first
        // momentum-tail event - without it the glide drains its target
        // and visibly stalls there. The advance is logged in `bridged`
        // and repaid by the next event, so nothing is counted twice.
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

    /// Drops any in-flight flick inertia and zoom glide. Called on an orbit
    /// target switch (from `reframe`): both animations target the old body's
    /// distance scale, so left running they would fight the reframe.
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

/// The camera hookup for every scene that flies a [`PtzCamera`]: implement
/// the three accessors saying where the camera and its scene-owned orbit
/// target live in the struct, and the blanket [`CameraControl`] impl below
/// supplies the whole input surface, forwarding each event into the embedded
/// camera - the `SceneClock` pattern (one required hook, shared logic lives
/// with the type it drives), replacing the forwarding block every scene used
/// to duplicate.
///
/// Three accessors, not one, because the forwarding needs three shapes:
/// `camera_mut` for the mutating input calls, `camera` for
/// [`CameraControl::cursor_hint`]'s `&self`, and `camera_target` because
/// `pointer_move`/`scroll`/`tick` scale by the orbited body - the target
/// stays scene-owned (see the module doc), so the blanket impl fetches it
/// per call and the rig can never drift from the orbit subject.
///
/// Every scene implements this, the `*_py` wrappers included: they keep
/// their camera as a plain wrapper field OUTSIDE their scene pyclass
/// (a script has no camera surface, and a pyclass cell's borrow guard
/// could never hand out the `&mut PtzCamera` this trait requires). A scene
/// that must diverge (gate input, fly a different camera kind) implements
/// [`CameraControl`] directly instead; a future non-interactive scene
/// simply implements neither and keeps `CameraControl`'s no-op defaults.
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
