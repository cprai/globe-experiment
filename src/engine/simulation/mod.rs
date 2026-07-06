//! Simulation state and astronomical math: the simulation clock and the
//! ephemeris-driven celestial sphere. This module defines the `Simulation`
//! trait that every scenario implements; the clock + celestial sphere live
//! directly in each scenario struct, which also builds its own Time panel
//! (deliberately per-scenario, so scenarios can diverge). It stays free of
//! any windowing (winit) or GPU (wgpu) dependency and never references the
//! camera type (the camera lives in the shared top-level `camera` module,
//! driven by `application`); it does depend on `ui` for the
//! `UIDrawablePanel`/`Instrument` types the selector panels are built from.

pub mod body;
pub mod celestial_sphere;
pub mod clock;
pub mod satellite;

use glam::{DVec3, Vec3};
use satkit::Instant;

pub use body::CelestialBody;
// Only the main binary's scenarios name this re-export; the headless bin tree
// compiles this module with no `Clock` consumer, so the import would warn
// there (its crate-level allow covers `dead_code`, not `unused_imports`).
#[allow(unused_imports)]
pub use clock::Clock;

use crate::engine::planet;
use crate::engine::ui::{
    Header, Instrument, InteractiveToggle, PanelAnchor, Toggle, UIDrawablePanel,
};
use celestial_sphere::CelestialSphere;

/// Synthetic characteristic radius (km) for a free [`CameraTarget::Coordinate`]
/// target. A coordinate has no body, but the camera's distance/zoom/pan limits
/// and look-at anchor are all scaled by the target's radius, so the free-point
/// variant needs a fallback scale. Chosen as Terra's mean radius so the
/// interaction feel matches the familiar default view.
const COORDINATE_RADIUS_KM: f32 = planet::TERRA_MEAN_RADIUS_KM;

/// What the orbital camera orbits: a celestial body (by identity) or a free
/// world-space point. A pure **identity** - the body's moving world center is
/// NOT stored here; it is looked up from the [`CelestialSphere`] (time -> the
/// ephemeris) wherever it is needed, so there is a single source of truth for
/// every center. The position-dependent accessors ([`center_world`],
/// [`render_origin`]) take the sphere; the body-frame ones (radius, surface
/// anchor) are static and do not. Defined here in `simulation` but consumed by
/// the camera in `application`, which now also reads the `CelestialSphere` (see
/// the relaxed purity note in `architecture.md`).
///
/// [`center_world`]: CameraTarget::center_world
/// [`render_origin`]: CameraTarget::render_origin
#[derive(Clone, Copy, Debug)]
pub enum CameraTarget {
    /// Orbit a celestial body, named by identity. Its world center (the live
    /// heliocentric ephemeris center - `-sol_geo` for Terra, its own position
    /// for Luna/the planets) is resolved from the `CelestialSphere` on demand.
    /// Because those centers are ~1.5e8 km (Terra/Luna) to billions of km out -
    /// far past f32 precision in world-km - a target renders with a floating
    /// origin (the scene is drawn relative to `render_origin`; see
    /// [`CameraTarget::render_origin`]). Terra/Luna keep the origin at Terra's
    /// center, so their render frame stays Terra-local (unchanged output).
    Body(CelestialBody),
    /// Orbit a fixed world-frame point (km). The camera treats it like a planet
    /// target: its own floating origin sits at the point, with synthetic body
    /// geometry. Future-proof scaffold - no scenario constructs it yet, but the
    /// camera and renderer handle it, so a future scenario can orbit an
    /// arbitrary point (e.g. a spacecraft, a Lagrange point) for free.
    #[allow(dead_code)]
    Coordinate(Vec3),
}

impl CameraTarget {
    /// The Terra target (the familiar default): identity Terra (at the origin).
    pub fn terra() -> CameraTarget {
        CameraTarget::Body(CelestialBody::TERRA)
    }

    /// Whether two targets name the same orbit subject. The camera uses this to
    /// detect a genuine switch (and reframe). Two `Body` targets match only
    /// when they are the *same* body (cycling Mars -> Jupiter reframes);
    /// two `Coordinate` targets always match (a free point never reframes
    /// once selected).
    pub fn same_kind(&self, other: &CameraTarget) -> bool {
        match (self, other) {
            (CameraTarget::Body(a), CameraTarget::Body(b)) => a == b,
            (CameraTarget::Coordinate(_), CameraTarget::Coordinate(_)) => true,
            _ => false,
        }
    }

    /// The orbit center in the world frame (km), resolved from the celestial
    /// sphere this frame. Heliocentric (Terra is at `-sol_geo`, not the
    /// origin); a coordinate target is its own center. f64, like the
    /// placement it comes from.
    pub fn center_world(&self, celestial: &CelestialSphere) -> DVec3 {
        match self {
            CameraTarget::Body(body) => celestial.center_world(*body),
            CameraTarget::Coordinate(point) => point.as_dvec3(),
        }
    }

    /// The world-space origin the scene is rendered relative to for this target
    /// (the "floating origin"). Terra and Luna are close enough to Terra's
    /// center that f32 world-km is precise, so they keep the origin at Terra's
    /// center - which makes their render output bit-identical to the pre-planet
    /// renderer. A planet (or a free coordinate) sits too far out for that, so
    /// the origin shifts to its own center, keeping the orbited subject near
    /// the numerical origin where f32 precision is restored.
    ///
    /// Terra's center is looked up from the sphere (not hard-coded `ZERO`) so
    /// this is frame-agnostic: the `CelestialSphere` is heliocentric, so
    /// Terra's center is `-sol_geo`, and using it here is exactly what
    /// keeps the Terra render frame (and every `X - origin` subtraction
    /// downstream) unchanged.
    pub fn render_origin(&self, celestial: &CelestialSphere) -> DVec3 {
        match self {
            // Terra/Luna: origin stays at Terra's center.
            CameraTarget::Body(CelestialBody::TerraSystem(_)) => {
                celestial.center_world(CelestialBody::TERRA)
            }
            // Any planet: too far out for the Terra origin, so the scene is
            // drawn relative to its own center.
            CameraTarget::Body(body) => celestial.center_world(*body),
            CameraTarget::Coordinate(point) => point.as_dvec3(),
        }
    }

    /// Characteristic mean radius (km). The camera scales its distance/zoom
    /// limits, near plane, and pan rate by this so the interaction feel is the
    /// same fraction of the subject whichever one is targeted. Static (a body's
    /// radius does not move), so no sphere is needed.
    pub fn mean_radius_km(&self) -> f32 {
        match self {
            CameraTarget::Body(body) => body.mean_radius_km(),
            CameraTarget::Coordinate(_) => COORDINATE_RADIUS_KM,
        }
    }

    /// Look-at anchor at `(lat, lon)` (radians), in the body frame (km) - an
    /// offset from the target center. The camera treats this as an
    /// inertial-frame direction (see `application::camera`); the magnitude is
    /// what differs per body. A free coordinate has no surface, so the anchor
    /// is the center itself (zero offset). Body-frame, so no sphere is needed.
    pub fn surface_position(&self, latitude: f32, longitude: f32) -> Vec3 {
        match self {
            CameraTarget::Body(body) => body.surface_position(latitude, longitude),
            CameraTarget::Coordinate(_) => Vec3::ZERO,
        }
    }

    /// Outward unit normal at `(lat, lon)` (radians), in the body frame - the
    /// local "up" the eye offsets along. A free coordinate uses the standard
    /// spherical direction so lon/lat still orbit the point.
    pub fn geodetic_normal(&self, latitude: f32, longitude: f32) -> Vec3 {
        match self {
            CameraTarget::Body(body) => body.geodetic_normal(latitude, longitude),
            CameraTarget::Coordinate(_) => {
                CelestialBody::TERRA.geodetic_normal(latitude, longitude)
            }
        }
    }
}

/// The interface every scenario implements. `ApplicationState` is generic over
/// `S: Simulation` and calls only these methods, so adding or swapping a
/// scenario requires no changes to the application layer.
///
/// This trait is UI-agnostic. The egui panel reads/drives a scenario through a
/// separate `crate::engine::ui::UIDrawable` impl (no `clock_mut`, no UI
/// snapshots from `frame_state`); the rendering trait is kept distinct from
/// `Simulation`.
pub trait Simulation {
    /// Advance the clock and re-evaluate the celestial sphere. Returns whether
    /// the clock is running, i.e. the app should keep requesting frames.
    fn advance(&mut self) -> bool;

    /// The current celestial sphere (Sol, Luna, the planets, star matrices for
    /// this frame's time). The application reads it to resolve the camera rig
    /// into world space - it derives `celestial_to_world` from
    /// `star_rot_inv.transpose()` and looks up the camera target's center
    /// through it. (The renderer separately re-derives the sphere from
    /// `RenderState.time`; both agree because `CelestialSphere::at` is
    /// deterministic.)
    fn celestial(&self) -> &CelestialSphere;

    /// Which subject the orbital camera should orbit this frame, by identity.
    /// Defaults to Terra, so every Terra-only scenario inherits it untouched;
    /// the eclipse / solar-system scenarios override it with the user-selected
    /// body. The application reads this each frame and re-aims the camera (see
    /// `application::camera`).
    fn camera_target(&self) -> CameraTarget {
        CameraTarget::terra()
    }

    /// Produce this frame's render state from the application-resolved camera
    /// rig: the eye in the floating-origin (render) frame and the world-frame
    /// look direction + up. Satellite propagation happens here, once per frame
    /// per satellite. The per-satellite readout produced by the same
    /// propagation is stashed on the scenario so the immediately-following
    /// `crate::engine::ui::UIDrawable::get_drawables` call (the egui panel)
    /// reports values matching the rendered markers.
    fn frame_state(&mut self, camera_pos: Vec3, look_at: Vec3, up: Vec3) -> RenderState;
}

/// The minimal render contract for one frame: the simulation time plus the
/// camera rig, as plain `glam` data (no GPU types). The renderer derives every
/// body's position and orientation from `time` itself (via
/// `CelestialSphere::at`), so this carries no body list, Sol position, or star
/// matrices - only what the renderer cannot recompute: the time, the camera,
/// and the satellite markers. Returned by [`Simulation::frame_state`] from the
/// application-resolved camera; the UI readout for the same frame is pulled
/// separately via `crate::engine::ui::UIDrawable`.
#[derive(Clone)]
pub struct RenderState {
    /// The instant the frame depicts. The renderer evaluates the ephemeris at
    /// this time to place Sol, Luna, and the planets - the same
    /// `CelestialSphere::at` the simulation core uses, so the two agree exactly
    /// (which is what keeps the orbited body's render-frame position a
    /// bit-exact zero).
    pub time: Instant,
    /// What the camera orbits this frame. The renderer reads its
    /// `render_origin()` (the floating-origin center it subtracts from every
    /// absolute body position) and its identity (to gate the Terra-system
    /// passes and scale the projection's near plane).
    pub camera_target: CameraTarget,
    /// Camera eye in the **floating-origin (render) frame** (km), i.e. relative
    /// to `camera_target.render_origin()` (= the absolute eye for Terra/Luna).
    /// Computed without ever forming the absolute eye for far targets (see
    /// `Camera::world_rig`), so it stays f32-precise.
    pub camera_pos: Vec3,
    /// The look-at point in the **render frame** (km) - the camera's view
    /// direction, carried as a point rather than a unit vector so the
    /// renderer's `look_at_rh` is bit-identical to the camera's old
    /// `view_proj` for Terra/Luna (reconstructing a normalized forward and
    /// re-projecting would drift by sub-ULP and speckle every antialiased
    /// edge).
    pub camera_look_at: Vec3,
    /// Camera up direction in the world frame (unit).
    pub camera_up: Vec3,
    /// One marker per tracked satellite, in the same order as the scenario's
    /// satellite list. The renderer draws them instanced, and propagates each
    /// marker's `Propagation` ahead to draw its predicted orbit path. This is
    /// the one piece of frame state not derivable from `time` (it depends on
    /// the scenario's tracked objects).
    pub markers: Vec<SatelliteMarker>,
}

/// A single satellite's on-screen marker for one frame: where to draw it,
/// whether it is visible, and how to predict its future path. Element of
/// [`RenderState::markers`].
#[derive(Clone, Debug)]
pub struct SatelliteMarker {
    /// Marker position in the world frame (km).
    pub position_km: Vec3,
    /// Whether the marker is visible (false when the solid Terra occludes it).
    pub visible: bool,
    /// How the renderer predicts this object's orbit path about one orbit
    /// ahead (`satellite::orbit_path_inertial`): analytic SGP4 from a cloned
    /// element set, or numerical propagation from a GCRF state vector (no TLE
    /// needed). Chosen per object by the scenario; a scene may mix both. The
    /// path, like the marker position, is the render input that is not
    /// derivable from `time` alone.
    pub propagation: satellite::Propagation,
}

/// One tracked satellite's readout for the UI panel. A scenario stashes a
/// `Vec<SatelliteTelemetry>` each frame in [`Simulation::frame_state`], built
/// from the same propagation that fills [`RenderState::markers`] (so readout
/// and marker can never disagree, and the orbit is propagated once per frame),
/// and turns it into `crate::engine::ui` instruments in its
/// `crate::engine::ui::UIDrawable` impl.
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

/// Tracks which body the user has chosen to orbit (Terra or Luna) and builds
/// the Terra / Luna selector panel. Shared by the scenarios that offer a Luna
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
    /// Set by the Terra key; cleared in `apply_requests`.
    request_terra: bool,
    /// Set by the Luna key; a field disjoint from `request_terra` so the two
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

    /// Resolves the current choice into a [`CameraTarget`] identity (the
    /// center is looked up from the ephemeris where it is needed).
    pub fn resolve(&self) -> CameraTarget {
        if self.luna_selected {
            CameraTarget::Body(CelestialBody::LUNA)
        } else {
            CameraTarget::terra()
        }
    }

    /// The top-right Terra / Luna selector panel: a header row plus one row of
    /// two latching keys (side by side, splitting the row), the chosen body
    /// lit. The keys' callbacks set disjoint request flags (see the type
    /// docs); `luna_selected` is snapshotted up front so no shared borrow
    /// outlives into the callbacks.
    pub fn panel(&mut self) -> UIDrawablePanel<'_> {
        let luna_active = self.luna_selected;
        let rows: Vec<Vec<Box<dyn Instrument + '_>>> = vec![
            vec![Box::new(Header {
                title: "Camera Target".to_string(),
            })],
            vec![
                Box::new(InteractiveToggle {
                    toggle: Toggle {
                        label: "Terra".to_string(),
                        active: !luna_active,
                    },
                    on_toggle: Box::new(|| self.request_terra = true),
                }),
                Box::new(InteractiveToggle {
                    toggle: Toggle {
                        label: "Luna".to_string(),
                        active: luna_active,
                    },
                    on_toggle: Box::new(|| self.request_luna = true),
                }),
            ],
        ];

        UIDrawablePanel {
            anchor: PanelAnchor::TopRight,
            rows,
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

    /// Resolves the current choice into a [`CameraTarget`] identity (the
    /// center is looked up from the ephemeris where it is needed).
    pub fn resolve(&self) -> CameraTarget {
        CameraTarget::Body(SELECTABLE_BODIES[self.selected])
    }

    /// The top-right selector panel: a header row plus one latching key per
    /// row, a single column ordered by distance from Sol (the chosen body
    /// lit; each lone key fills its row). `selected` is snapshotted up front
    /// so no shared borrow outlives into the per-key callbacks, which each set
    /// a disjoint `request_*` flag.
    pub fn panel(&mut self) -> UIDrawablePanel<'_> {
        let selected = self.selected;
        // One key per body, in its own row; index i lines up with
        // SELECTABLE_BODIES so the label + `active` reflect the live selection.
        let key = |i: usize| Toggle {
            label: SELECTABLE_BODIES[i].name().to_string(),
            active: selected == i,
        };
        let rows: Vec<Vec<Box<dyn Instrument + '_>>> = vec![
            vec![Box::new(Header {
                title: "Camera Target".to_string(),
            })],
            vec![Box::new(InteractiveToggle {
                toggle: key(0),
                on_toggle: Box::new(|| self.request_mercury = true),
            })],
            vec![Box::new(InteractiveToggle {
                toggle: key(1),
                on_toggle: Box::new(|| self.request_venus = true),
            })],
            vec![Box::new(InteractiveToggle {
                toggle: key(2),
                on_toggle: Box::new(|| self.request_terra = true),
            })],
            vec![Box::new(InteractiveToggle {
                toggle: key(3),
                on_toggle: Box::new(|| self.request_luna = true),
            })],
            vec![Box::new(InteractiveToggle {
                toggle: key(4),
                on_toggle: Box::new(|| self.request_mars = true),
            })],
            vec![Box::new(InteractiveToggle {
                toggle: key(5),
                on_toggle: Box::new(|| self.request_jupiter = true),
            })],
            vec![Box::new(InteractiveToggle {
                toggle: key(6),
                on_toggle: Box::new(|| self.request_saturn = true),
            })],
            vec![Box::new(InteractiveToggle {
                toggle: key(7),
                on_toggle: Box::new(|| self.request_uranus = true),
            })],
            vec![Box::new(InteractiveToggle {
                toggle: key(8),
                on_toggle: Box::new(|| self.request_neptune = true),
            })],
        ];

        UIDrawablePanel {
            anchor: PanelAnchor::TopRight,
            rows,
        }
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
    let c = eye.length_squared() - planet::TERRA_MEAN_RADIUS_KM * planet::TERRA_MEAN_RADIUS_KM;
    let disc = b * b - c;
    if disc < 0.0 {
        return false; // line of sight misses Terra entirely
    }
    let t = -b - disc.sqrt(); // nearest intersection along the ray
    t > 0.0 && t < distance // Terra sits between the eye and the station
}
