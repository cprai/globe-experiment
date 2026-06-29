//! Simulation state and astronomical math: the simulation clock and the
//! ephemeris-driven celestial sphere. This module owns the shared simulation
//! core, defines the `Simulation` trait that every scenario implements, and
//! carries the `impl UIDrawable for SimulationState` that emits the shared-core
//! panel. It stays free of any windowing (winit) or GPU (wgpu) dependency and
//! never references the camera type (the camera lives in `application`); it
//! does depend on `ui` for the `UIDrawable`/`Instrument` types the shared-core
//! panel is built from.

pub mod body;
pub mod celestial_sphere;
pub mod clock;
pub mod satellite;

use glam::{Mat3, Mat4, Vec3};
use satkit::Instant;

pub use body::{BodyState, CelestialBody, TerraSystemEntity};
pub use clock::Clock;

use crate::terra;
use crate::ui::{
    Header, Instrument, InteractiveSlider, InteractiveToggle, PanelAnchor, Readout, Slider, Toggle,
    UIDrawable, UIDrawablePanel,
};
use celestial_sphere::CelestialSphere;

/// The body the orbital camera orbits, with its world-space center (km). Plain
/// astronomical data - no `Camera` or windowing dependency - so it can cross
/// the camera-agnostic `simulation` boundary the same way [`RenderState`] does:
/// it is defined here but consumed by the camera in `application`. The geometry
/// accessors delegate to the single-source-of-truth body modules (`terra` /
/// `luna`), so the camera rig never duplicates surface math.
///
/// Terra sits at the world origin; Luna's and the planets' centers come
/// from the ephemeris and are refreshed every frame (they move), so the target
/// carries the center alongside the body identity.
#[derive(Clone, Copy)]
pub struct CameraTarget {
    /// Which body the camera orbits. Geometry (radius, surface) is read through
    /// this identity (see [`CelestialBody`]).
    pub body: CelestialBody,
    /// The orbited body's center in the world frame (km). Terra is the origin
    /// (`ZERO`); Luna and the planets carry their live ephemeris center.
    /// Because a planet's center can be billions of km out - far past f32
    /// precision in world-km - a planet target renders with a floating origin
    /// (the scene is drawn relative to `render_origin`, which equals this
    /// center; see `RenderState::render_origin` and `Camera::view_proj`).
    /// Terra/Luna keep the origin at Terra.
    pub center_world: Vec3,
}

impl CameraTarget {
    /// The Terra target (the familiar default): identity Terra at the origin.
    pub fn terra() -> CameraTarget {
        CameraTarget {
            body: CelestialBody::TERRA,
            center_world: Vec3::ZERO,
        }
    }

    /// Whether two targets name the same body, ignoring the (per-frame) Luna/
    /// planet center. The camera uses this to detect a genuine body switch (and
    /// reframe) without treating a body's normal ephemeris drift as a change. A
    /// derived `PartialEq` would compare the centers and so fire every frame.
    /// Two planet targets match only when they are the *same* planet, so
    /// cycling Mars -> Jupiter reframes.
    pub fn same_kind(&self, other: &CameraTarget) -> bool {
        self.body == other.body
    }

    /// The body center in the world frame (km). Terra is the origin.
    pub fn center_world(&self) -> Vec3 {
        self.center_world
    }

    /// The world-space origin the scene is rendered relative to for this target
    /// (the "floating origin"). Terra and Luna are close enough to the
    /// Terra origin that f32 world-km is precise, so they keep the origin at
    /// Terra (`ZERO`) - which makes their render output bit-identical to the
    /// pre-planet renderer. A planet sits too far out for that, so the origin
    /// shifts to the planet's center, keeping the orbited body near the
    /// numerical origin where f32 precision is restored.
    pub fn render_origin(&self) -> Vec3 {
        match self.body {
            CelestialBody::TerraSystem(_) => Vec3::ZERO,
            // Any planet: too far out for the Terra origin, so the scene is
            // drawn relative to its own center.
            _ => self.center_world,
        }
    }

    /// Characteristic mean radius (km). The camera scales its distance/zoom
    /// limits, near plane, and pan rate by this so the interaction feel is the
    /// same fraction of the body whichever one is targeted.
    pub fn mean_radius_km(&self) -> f32 {
        self.body.mean_radius_km()
    }

    /// Look-at anchor on the body surface at `(lat, lon)` (radians), in the
    /// body frame (km). The camera treats this as an inertial-frame direction
    /// (see `application::camera`); the magnitude is what differs per body.
    pub fn surface_position(&self, latitude: f32, longitude: f32) -> Vec3 {
        self.body.surface_position(latitude, longitude)
    }

    /// Outward unit normal of the body surface at `(lat, lon)` (radians), in
    /// the body frame - the local "up" the eye offsets along.
    pub fn geodetic_normal(&self, latitude: f32, longitude: f32) -> Vec3 {
        self.body.geodetic_normal(latitude, longitude)
    }
}

/// The interface every scenario implements. `ApplicationState` is generic over
/// `S: Simulation` and calls only these methods, so adding or swapping a
/// scenario requires no changes to the application layer.
///
/// This trait is UI-agnostic. The egui panel reads/drives a scenario through a
/// separate `crate::ui::UIDrawable` impl (no `clock_mut`, no UI snapshots from
/// `frame_state`); the rendering trait is kept distinct from `Simulation` even
/// though both the trait and the shared-core impl now live in this module.
pub trait Simulation {
    /// Advance the clock and re-evaluate the celestial sphere. Returns whether
    /// the clock is running, i.e. the app should keep requesting frames.
    fn advance(&mut self) -> bool;

    /// Rotation from the inertial (star-fixed) camera rig frame to the
    /// Earth-fixed world frame. The application uses this to resolve the camera
    /// before computing eye and view_proj for `frame_state`.
    fn celestial_to_world(&self) -> Mat3;

    /// Which body the orbital camera should center on this frame. Defaults to
    /// Terra, so every Terra-only scenario inherits it untouched; the
    /// eclipse scenarios override it to return the user-selected body (the
    /// Luna's center pulled from the ephemeris). The application reads this
    /// each frame and re-aims the camera (see `application::camera`).
    fn camera_target(&self) -> CameraTarget {
        CameraTarget::terra()
    }

    /// Produce this frame's render state from the application-resolved camera.
    /// Satellite propagation happens here, once per frame per satellite. The
    /// per-satellite readout produced by the same propagation is stashed on the
    /// scenario so the immediately-following
    /// `crate::ui::UIDrawable::get_drawables` call (the egui panel) reports
    /// values matching the rendered markers.
    fn frame_state(&mut self, eye: Vec3, view_proj: Mat4) -> RenderState;
}

/// The finished, render-ready positions/matrices for one frame: everything the
/// renderer needs, as plain `glam` data (no GPU types). Returned by
/// [`Simulation::frame_state`] from the application-resolved camera; the UI
/// readout for the same frame is pulled separately via `crate::ui::UIDrawable`.
#[derive(Clone, Debug)]
pub struct RenderState {
    /// View-projection matrix, built by the camera in the **floating-origin
    /// (render) frame** (`celestial_to_world` + aspect): the orbited body sits
    /// at the numerical origin so far planet targets stay f32-precise.
    pub view_proj: Mat4,
    /// Camera eye in the **floating-origin (render) frame** (km), i.e. relative
    /// to `render_origin` (= the absolute eye for Terra/Luna). The renderer's
    /// per-body positions are all expressed in this same frame.
    pub camera_pos: Vec3,
    /// The world-space point the render frame is centered on: the camera
    /// target's center (`camera_target.render_origin()`), `ZERO` for
    /// Terra/Luna. The renderer subtracts it (on the CPU) from each body's
    /// absolute world position below so the GPU only ever sees small,
    /// target-local coordinates; it is NOT uploaded to the shader.
    pub render_origin: Vec3,
    /// Sol position in the absolute world frame, km (true geocentric). The
    /// renderer expresses it relative to `render_origin` and every lit pass
    /// derives its Sol direction from it (`normalize(sol - surface)`); there is
    /// no Earth-fixed `sol_dir` in the render path.
    pub sol_pos_world: Vec3,
    /// World -> star-texture (galactic) rotation for the equirectangular
    /// star-map lookup (uploaded as `star_rot_inv`). This is the
    /// galactic->equatorial-corrected matrix (`star_tex_rot_inv`), distinct
    /// from the equatorial frame the camera rig uses.
    pub star_rot_inv: Mat3,
    /// The celestial bodies to draw this frame (identity + placement), as a
    /// flat list. The renderer pulls Luna's placement from here for the
    /// lunar mesh + the analytic eclipse-shadow geometry (its radius comes
    /// from the identity, `luna::MEAN_RADIUS_KM`), and routes each planet
    /// entry to the planet pipeline by identity. Terra/Luna scenarios carry
    /// only the Terra system (Terra + Luna), so no planet entry exists
    /// and the planet pipeline never runs; the solar-system scenario (and
    /// render mode) carry the whole list.
    pub celestial_bodies: Vec<BodyState>,
    /// One marker per tracked satellite, in the same order as the scenario's
    /// satellite list. The renderer draws them instanced.
    pub markers: Vec<SatelliteMarker>,
}

/// A single satellite's on-screen marker for one frame: where to draw it and
/// whether it is visible. Element of [`RenderState::markers`].
#[derive(Clone, Copy, Debug)]
pub struct SatelliteMarker {
    /// Marker position in the world frame (km).
    pub position_km: Vec3,
    /// Whether the marker is visible (false when the solid Terra occludes it).
    pub visible: bool,
}

/// One tracked satellite's readout for the UI panel. A scenario stashes a
/// `Vec<SatelliteTelemetry>` each frame in [`Simulation::frame_state`], built
/// from the same propagation that fills [`RenderState::markers`] (so readout
/// and marker can never disagree, and the orbit is propagated once per frame),
/// and turns it into `crate::ui` instruments in its `crate::ui::UIDrawable`
/// impl.
#[derive(Clone, Debug)]
pub struct SatelliteTelemetry {
    /// Object name (e.g. "ISS (ZARYA)").
    pub name: String,
    /// Sub-satellite geodetic latitude, degrees.
    pub latitude_deg: f32,
    /// Sub-satellite geodetic longitude, degrees.
    pub longitude_deg: f32,
    /// Height above the WGS84 ellipsoid, kilometers.
    pub altitude_km: f32,
}

/// Seeds satkit's global state (embedded ephemeris + EOP table) for fully
/// offline, data-dir-free use. Must be called once at startup, before any
/// `CelestialSphere` is built. Thin wrapper over
/// `celestial_sphere::init_satkit` so callers (e.g. `main`) need not know
/// about the `celestial_sphere` submodule.
pub fn init() {
    celestial_sphere::init_satkit();
}

/// The shared simulation core: the clock (datetime + play/paused + speed) and
/// the ephemeris-driven celestial sphere (Sol direction + star-map
/// orientation). Held by composition inside each scenario's simulation struct,
/// which adds its own satellite list and implements [`Simulation`].
///
/// Does not own satellites - those live in the scenario struct so each scenario
/// can choose its own tracked objects without changing this shared core. This
/// struct owns the astronomical infrastructure; the `Simulation` trait owns the
/// per-scenario policy.
pub struct SimulationState {
    pub clock: Clock,
    pub celestial_sphere: CelestialSphere,
}

impl SimulationState {
    /// Builds the core state starting at `start_epoch`. `init` must already
    /// have run (the celestial sphere reads satkit globals).
    pub fn new(start_epoch: Instant) -> Self {
        let clock = Clock::new(start_epoch);
        let celestial_sphere = CelestialSphere::at(&clock.now());
        Self {
            clock,
            celestial_sphere,
        }
    }

    /// Advances the clock and, while it is running, re-evaluates the
    /// ephemeris-driven celestial sphere at the new time. Returns whether the
    /// clock is running - an "animating" source that keeps frames coming; when
    /// paused nothing advances and the app can go idle.
    pub fn advance(&mut self) -> bool {
        let running = self.clock.tick();
        if running {
            self.celestial_sphere = CelestialSphere::at(&self.clock.now());
        }
        running
    }

    /// Rotation from the inertial (star-fixed) frame the camera rig lives in
    /// to the Earth-fixed world frame the scene is drawn in - the inverse of
    /// the celestial sphere's world -> celestial rotation. (Orthonormal, so
    /// transpose = inverse.)
    pub fn celestial_to_world(&self) -> Mat3 {
        self.celestial_sphere.star_rot_inv.transpose()
    }
}

/// Tracks which body the user has chosen to orbit (Terra or Luna) and builds
/// the TERRA / LUNA selector panel. Shared by the scenarios that offer a Luna
/// target (the eclipses); Terra-only scenarios never hold one.
///
/// Two radio toggles can't both `&mut` one selection field - a panel's element
/// callbacks all coexist, so each must capture a *disjoint* mutable field (the
/// same rule that makes the Run toggle and speed slider capture `paused` vs
/// `multiplier` separately). So a key press only sets a disjoint `request_*`
/// flag; [`apply_requests`](Self::apply_requests) reconciles it into
/// `luna_selected` once per frame. That is a one-frame latency, imperceptible
/// and identical to the existing clock-edit delay.
pub struct TargetSelector {
    luna_selected: bool,
    /// Set by the TERRA key; cleared in `apply_requests`.
    request_terra: bool,
    /// Set by the LUNA key; a field disjoint from `request_terra` so the two
    /// key callbacks can coexist.
    request_luna: bool,
}

impl TargetSelector {
    /// Builds a selector with the initial choice (`true` = Luna).
    pub fn new(luna_selected: bool) -> Self {
        Self {
            luna_selected,
            request_terra: false,
            request_luna: false,
        }
    }

    /// Applies any pending key press into the live selection, then clears the
    /// flags. Call once per frame *before* [`Simulation::camera_target`] is
    /// read (i.e. at the top of the scenario's `advance`). A simultaneous
    /// press of both keys in one frame is impossible from a mouse, but
    /// resolve it to Luna for determinism.
    pub fn apply_requests(&mut self) {
        if self.request_luna {
            self.luna_selected = true;
        } else if self.request_terra {
            self.luna_selected = false;
        }
        self.request_terra = false;
        self.request_luna = false;
    }

    /// Resolves the current choice into a [`CameraTarget`], filling Luna
    /// center from the live ephemeris.
    pub fn resolve(&self, luna_center: Vec3) -> CameraTarget {
        if self.luna_selected {
            CameraTarget {
                body: CelestialBody::LUNA,
                center_world: luna_center,
            }
        } else {
            CameraTarget::terra()
        }
    }

    /// The top-right TERRA / LUNA selector panel: a header plus two latching
    /// keys, the chosen body lit. The keys' callbacks set disjoint request
    /// flags (see the type docs); `luna_selected` is snapshotted up front so no
    /// shared borrow outlives into the callbacks.
    pub fn panel(&mut self) -> UIDrawablePanel<'_> {
        let luna_active = self.luna_selected;
        let elements: Vec<Box<dyn Instrument + '_>> = vec![
            Box::new(Header {
                position: [0.0, 0.0],
                title: "Camera Target".to_string(),
            }),
            Box::new(InteractiveToggle {
                toggle: Toggle {
                    position: [0.0, 26.0],
                    label: "Terra".to_string(),
                    active: !luna_active,
                },
                on_toggle: Box::new(|| self.request_terra = true),
            }),
            Box::new(InteractiveToggle {
                toggle: Toggle {
                    position: [104.0, 26.0],
                    label: "Luna".to_string(),
                    active: luna_active,
                },
                on_toggle: Box::new(|| self.request_luna = true),
            }),
        ];

        UIDrawablePanel {
            anchor: PanelAnchor::TopRight,
            offset: [10.0, 10.0],
            size: [212.0, 64.0],
            elements,
        }
    }
}

/// Every selectable body, ordered by distance from Sol, with Luna
/// placed right after its parent Terra. This is also the top-to-bottom order of
/// the selector panel's keys. The `request_*` fields of [`BodySelector`] and
/// the `apply_requests` branches mirror this order index-for-index; keep all
/// three in sync if the list changes.
const SELECTABLE_BODIES: [CelestialBody; 9] = [
    CelestialBody::Mercury,
    CelestialBody::Venus,
    CelestialBody::TERRA,
    CelestialBody::LUNA,
    CelestialBody::Mars,
    CelestialBody::Jupiter,
    CelestialBody::Saturn,
    CelestialBody::Uranus,
    CelestialBody::Neptune,
];

/// Index of Terra in [`SELECTABLE_BODIES`] - the scenario's start target.
const TERRA_INDEX: usize = 2;

/// Tracks which solar-system body the camera orbits and builds the selector
/// panel: one always-visible latching key per body (the chosen one lit), so the
/// whole solar system is selectable at a glance.
///
/// The panel's element callbacks all coexist, so each must capture a *disjoint*
/// mutable field (the same rule the clock's Run toggle and speed slider
/// follow); hence one `request_*` flag per body rather than a single shared
/// selection a key could write. A click sets that body's flag;
/// [`apply_requests`] folds it into `selected` once per frame, before
/// [`Simulation::camera_target`] reads it
/// - the same one-frame latency as [`TargetSelector`].
pub struct BodySelector {
    /// Index into [`SELECTABLE_BODIES`].
    selected: usize,
    /// One per body, set by that body's key and cleared in `apply_requests`.
    /// Disjoint fields (not an array) so the key callbacks can each capture one
    /// without borrowing a shared place. In [`SELECTABLE_BODIES`] order.
    request_mercury: bool,
    request_venus: bool,
    request_terra: bool,
    request_luna: bool,
    request_mars: bool,
    request_jupiter: bool,
    request_saturn: bool,
    request_uranus: bool,
    request_neptune: bool,
}

impl Default for BodySelector {
    fn default() -> Self {
        // Start on Terra (the familiar default view).
        Self {
            selected: TERRA_INDEX,
            request_mercury: false,
            request_venus: false,
            request_terra: false,
            request_luna: false,
            request_mars: false,
            request_jupiter: false,
            request_saturn: false,
            request_uranus: false,
            request_neptune: false,
        }
    }
}

impl BodySelector {
    /// Applies any pending key press into the live selection, then clears every
    /// flag. Call once per frame *before* [`Simulation::camera_target`] is read
    /// (at the top of the scenario's `advance`). At most one key can be pressed
    /// per frame from a mouse; the branch order only breaks an impossible tie.
    /// The indices match [`SELECTABLE_BODIES`].
    pub fn apply_requests(&mut self) {
        if self.request_mercury {
            self.selected = 0;
        } else if self.request_venus {
            self.selected = 1;
        } else if self.request_terra {
            self.selected = 2;
        } else if self.request_luna {
            self.selected = 3;
        } else if self.request_mars {
            self.selected = 4;
        } else if self.request_jupiter {
            self.selected = 5;
        } else if self.request_saturn {
            self.selected = 6;
        } else if self.request_uranus {
            self.selected = 7;
        } else if self.request_neptune {
            self.selected = 8;
        }
        self.request_mercury = false;
        self.request_venus = false;
        self.request_terra = false;
        self.request_luna = false;
        self.request_mars = false;
        self.request_jupiter = false;
        self.request_saturn = false;
        self.request_uranus = false;
        self.request_neptune = false;
    }

    /// Resolves the current choice into a [`CameraTarget`], filling the body
    /// center from the live ephemeris (`celestial` carries every selectable
    /// body, so the lookup always succeeds).
    pub fn resolve(&self, celestial: &CelestialSphere) -> CameraTarget {
        let body = SELECTABLE_BODIES[self.selected];
        CameraTarget {
            body,
            center_world: celestial.center_world(body),
        }
    }

    /// The top-right selector panel: a header plus one latching key per body,
    /// in a single column ordered by distance from Sol (the chosen body
    /// lit). `selected` is snapshotted up front so no shared borrow
    /// outlives into the per-key callbacks, which each set a disjoint
    /// `request_*` flag.
    pub fn panel(&mut self) -> UIDrawablePanel<'_> {
        let selected = self.selected;
        // One key per body, in its own row; index i lines up with
        // SELECTABLE_BODIES so the label + `active` reflect the live selection.
        let key = |i: usize| Toggle {
            position: [0.0, 26.0 + i as f32 * 28.0],
            label: SELECTABLE_BODIES[i].name().to_string(),
            active: selected == i,
        };
        let elements: Vec<Box<dyn Instrument + '_>> = vec![
            Box::new(Header {
                position: [0.0, 0.0],
                title: "Camera Target".to_string(),
            }),
            Box::new(InteractiveToggle {
                toggle: key(0),
                on_toggle: Box::new(|| self.request_mercury = true),
            }),
            Box::new(InteractiveToggle {
                toggle: key(1),
                on_toggle: Box::new(|| self.request_venus = true),
            }),
            Box::new(InteractiveToggle {
                toggle: key(2),
                on_toggle: Box::new(|| self.request_terra = true),
            }),
            Box::new(InteractiveToggle {
                toggle: key(3),
                on_toggle: Box::new(|| self.request_luna = true),
            }),
            Box::new(InteractiveToggle {
                toggle: key(4),
                on_toggle: Box::new(|| self.request_mars = true),
            }),
            Box::new(InteractiveToggle {
                toggle: key(5),
                on_toggle: Box::new(|| self.request_jupiter = true),
            }),
            Box::new(InteractiveToggle {
                toggle: key(6),
                on_toggle: Box::new(|| self.request_saturn = true),
            }),
            Box::new(InteractiveToggle {
                toggle: key(7),
                on_toggle: Box::new(|| self.request_uranus = true),
            }),
            Box::new(InteractiveToggle {
                toggle: key(8),
                on_toggle: Box::new(|| self.request_neptune = true),
            }),
        ];

        UIDrawablePanel {
            anchor: PanelAnchor::TopRight,
            offset: [10.0, 10.0],
            // Tall single column: header + 9 rows at 28px pitch.
            size: [150.0, 282.0],
            elements,
        }
    }
}

impl UIDrawable for SimulationState {
    /// The shared-core panel, read from live state: the clock datetime, and
    /// the play/pause + speed controls whose
    /// callbacks mutate the live clock. The two control callbacks capture
    /// disjoint clock fields (`paused` vs `multiplier`) via direct field
    /// assignment - a `Clock` method would borrow the whole clock and collide.
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        // Snapshot the displayed values up front (owned `String`/`f32`/`bool`),
        // so no shared borrow of the clock outlives into the mutable callback
        // captures below.
        let datetime = self.clock.datetime_label();
        let speed = format!("{:.1}x", self.clock.multiplier);
        let running = !self.clock.paused;

        // Exponential (base e) speed: the slider edits the exponent, so
        // multiplier = e^exp - real time (e^0 = 1x) at the left, 100x at the
        // right, 10x at the midpoint. The mapping lives here, not in the panel.
        let speed_exp = self.clock.multiplier.ln();
        let exp_range = Clock::MIN_MULTIPLIER.ln()..=Clock::MAX_MULTIPLIER.ln();

        // Instrument positions are relative to this panel's content origin. The
        // producer picks instruments + content only; all styling is in the
        // instrument modules.
        let elements: Vec<Box<dyn Instrument + '_>> = vec![
            Box::new(Header {
                position: [0.0, 0.0],
                title: "Time".to_string(),
            }),
            Box::new(Readout {
                position: [0.0, 26.0],
                label: "UTC".to_string(),
                value: datetime,
            }),
            Box::new(InteractiveToggle {
                toggle: Toggle {
                    position: [0.0, 52.0],
                    label: "Run".to_string(),
                    active: running,
                },
                on_toggle: Box::new(|| self.clock.paused = !self.clock.paused),
            }),
            Box::new(Readout {
                position: [104.0, 54.0],
                label: "Speed".to_string(),
                value: speed,
            }),
            Box::new(InteractiveSlider {
                slider: Slider {
                    position: [0.0, 82.0],
                    value: speed_exp,
                    range: exp_range,
                },
                on_change: Box::new(|exp| self.clock.multiplier = exp.exp()),
            }),
        ];

        vec![UIDrawablePanel {
            anchor: PanelAnchor::TopLeft,
            offset: [10.0, 10.0],
            size: [340.0, 116.0],
            elements,
        }]
    }
}

/// Whether the solid Terra blocks the line of sight from `eye` to `target`
/// (both world-space km). Approximates the planet as a sphere of mean Terra
/// radius - slightly conservative against the WGS84 ellipsoid, which is fine
/// for deciding whether to hide the marker.
pub(crate) fn marker_occluded(eye: Vec3, target: Vec3) -> bool {
    let to_target = target - eye;
    let distance = to_target.length();
    if distance <= 1e-3 {
        return false;
    }
    let dir = to_target / distance;

    // Ray-sphere intersection of the line of sight with the Terra sphere.
    let b = dir.dot(eye);
    let c = eye.length_squared() - terra::MEAN_RADIUS_KM * terra::MEAN_RADIUS_KM;
    let disc = b * b - c;
    if disc < 0.0 {
        return false; // line of sight misses Terra entirely
    }
    let t = -b - disc.sqrt(); // nearest intersection along the ray
    t > 0.0 && t < distance // Terra sits between the eye and the station
}
