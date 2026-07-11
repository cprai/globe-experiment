//! Simulation state and astronomical math: the simulation clock and the
//! ephemeris-driven celestial sphere. This module defines the `Scene`
//! trait that every scene implements; the clock lives directly in each
//! scene struct (the celestial sphere is not stored anywhere -
//! `CelestialSphere::at` is a pure function of time, evaluated on demand),
//! and each scene also builds its own panels - the Time panel and any
//! camera-target panel included -
//! (deliberately per-scene, so scenes can diverge). It stays free of
//! any windowing (winit), GPU (wgpu), or UI (`ui`/egui) dependency and never
//! references any camera type (the `CameraControl`/`CameraView` traits +
//! `PtzCamera` live in the shared `engine::camera` module, implemented/held
//! by each scene).

pub mod body;
pub mod celestial_sphere;
pub mod clock;
pub mod satellite;

use glam::DVec3;
use pyo3::prelude::*;
use satkit::Instant;

pub use body::CelestialBody;
// `SceneClock` is named by the `Scene` trait itself (the `tick_scene`
// bound), so that half of the import is used in both bin trees; `Clock` is
// named only by the main binary's scenes, so the headless tree (whose
// crate-level allow covers `dead_code`, not `unused_imports`) would warn on
// it without the allow.
#[allow(unused_imports)]
pub use clock::{Clock, SceneClock};

use crate::engine::planet;
use celestial_sphere::CelestialSphere;

/// Synthetic characteristic radius (km) for a free [`CameraTarget::Coordinate`]
/// target. A coordinate has no body, but the camera's distance/zoom/pan limits
/// and look-at anchor are all scaled by the target's radius, so the free-point
/// variant needs a fallback scale. Chosen as Terra's mean radius so the
/// interaction feel matches the familiar default view.
const COORDINATE_RADIUS_KM: f64 = planet::TERRA_MEAN_RADIUS_KM;

/// What the orbital camera orbits: a celestial body (by identity) or a free
/// world-space point. A pure **identity** - the body's moving world center is
/// NOT stored here; it is looked up from the [`CelestialSphere`] (time -> the
/// ephemeris) wherever it is needed, so there is a single source of truth for
/// every center. The position-dependent accessors ([`center_world`],
/// [`render_origin`]) take the sphere; the body-frame ones (radius, surface
/// anchor) are static and do not. Defined here in `simulation` but consumed by
/// the camera module (a sanctioned `simulation` -> `camera` data edge, like
/// `RenderState`).
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
    /// geometry. Future-proof scaffold - no scene constructs it yet, but the
    /// camera and renderer handle it, so a future scene can orbit an
    /// arbitrary point (e.g. a spacecraft, a Lagrange point) for free.
    #[allow(dead_code)]
    Coordinate(DVec3),
}

impl CameraTarget {
    /// The Terra target (the familiar default): identity Terra (at the origin).
    pub fn terra() -> CameraTarget {
        CameraTarget::Body(CelestialBody::TERRA)
    }

    /// Whether two targets name the same orbit subject. The scene (which owns
    /// its camera target) uses this to detect a genuine switch and have the
    /// camera reframe. Two `Body` targets match only when they are the *same*
    /// body (cycling Mars -> Jupiter reframes); two `Coordinate` targets
    /// always match (a free point never reframes once selected).
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
            CameraTarget::Coordinate(point) => *point,
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
            CameraTarget::Coordinate(point) => *point,
        }
    }

    /// Characteristic mean radius (km). The camera scales its distance/zoom
    /// limits, near plane, and pan rate by this so the interaction feel is the
    /// same fraction of the subject whichever one is targeted. Static (a body's
    /// radius does not move), so no sphere is needed.
    pub fn mean_radius_km(&self) -> f64 {
        match self {
            CameraTarget::Body(body) => body.mean_radius_km(),
            CameraTarget::Coordinate(_) => COORDINATE_RADIUS_KM,
        }
    }

    /// Look-at anchor at `(lat, lon)` (radians), in the body frame (km) - an
    /// offset from the target center. The camera treats this as an
    /// inertial-frame direction (see `crate::engine::camera`); the magnitude is
    /// what differs per body. A free coordinate has no surface, so the anchor
    /// is the center itself (zero offset). Body-frame, so no sphere is needed.
    pub fn surface_position(&self, latitude: f64, longitude: f64) -> DVec3 {
        match self {
            CameraTarget::Body(body) => body.surface_position(latitude, longitude),
            CameraTarget::Coordinate(_) => DVec3::ZERO,
        }
    }

    /// Outward unit normal at `(lat, lon)` (radians), in the body frame - the
    /// local "up" the eye offsets along. A free coordinate uses the standard
    /// spherical direction so lon/lat still orbit the point.
    pub fn geodetic_normal(&self, latitude: f64, longitude: f64) -> DVec3 {
        match self {
            CameraTarget::Body(body) => body.geodetic_normal(latitude, longitude),
            CameraTarget::Coordinate(_) => {
                CelestialBody::TERRA.geodetic_normal(latitude, longitude)
            }
        }
    }
}

/// The simulation half of the interface every scene implements.
/// `ApplicationState` bounds `S: Scene + CameraControl + CameraView +
/// UIDrawable` and calls only those traits' methods, so adding or swapping a
/// scene requires no changes to the application layer.
///
/// This trait is UI- and camera-agnostic. The egui panel reads/drives a
/// scene through a separate `crate::engine::ui::UIDrawable` impl, and the
/// frame's `RenderState` (camera rig included) comes from the scene's
/// `crate::engine::camera::CameraView` impl - each concern is a distinct
/// trait on the same scene struct.
pub trait Scene {
    /// Scene-specific per-frame work (e.g. `manual_control`'s orbit
    /// re-anchor; most scenes have none). Called by [`Scene::tick_scene`]
    /// AFTER the clock has ticked; a scene that cares whether it advanced
    /// reads `clock_paused()` through its own [`SceneClock`] impl (a paused
    /// clock also yields dt = 0, which is what pause-sensitive work keys on).
    fn advance(&mut self);

    /// The per-frame entry point the application calls: tick the clock, then
    /// run the scene's own [`Scene::advance`]. Provided for every scene that
    /// implements [`SceneClock`] (all of them), so no scene hand-writes the
    /// clock tick.
    fn tick_scene(&mut self)
    where
        Self: SceneClock,
    {
        self.tick_clock();
        self.advance();
    }
}

/// The minimal render contract for one frame: the simulation time plus the
/// camera rig, as plain `glam` data (no GPU types). The renderer derives every
/// body's position and orientation from `time` itself (via
/// `CelestialSphere::at`), so this carries no body list, Sol position, or star
/// matrices - only what the renderer cannot recompute: the time, the camera,
/// and the satellite markers. Returned by the scene's
/// `crate::engine::camera::CameraView::frame_state` impl (which resolves its
/// own camera rig); the UI readout for the same frame is pulled separately
/// via `crate::engine::ui::UIDrawable`.
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
    /// `PtzCamera::world_rig`). f64; the renderer builds the view matrix in f64
    /// and casts to f32 only at uniform upload.
    pub camera_pos: DVec3,
    /// The look-at point in the **render frame** (km) - the camera's view
    /// direction, carried as a point rather than a unit vector so the
    /// renderer's `look_at_rh` reproduces the camera's implied view exactly
    /// (reconstructing a normalized forward and re-projecting would drift by
    /// sub-ULP and speckle every antialiased edge).
    pub camera_look_at: DVec3,
    /// Camera up direction in the world frame (unit).
    pub camera_up: DVec3,
    /// One marker per tracked satellite, in the same order as the scene's
    /// satellite list. The renderer draws them instanced, and propagates each
    /// marker's `Propagation` ahead to draw its predicted orbit path. This is
    /// the one piece of frame state not derivable from `time` (it depends on
    /// the scene's tracked objects).
    pub markers: Vec<SatelliteMarker>,
}

/// A single satellite's on-screen marker for one frame: where to draw it,
/// whether it is visible, and how to predict its future path. Element of
/// [`RenderState::markers`].
#[derive(Clone, Debug)]
pub struct SatelliteMarker {
    /// Marker position in the world frame (km).
    pub position_km: DVec3,
    /// Whether the marker is visible (false when the solid Terra occludes it).
    pub visible: bool,
    /// How the renderer predicts this object's orbit path about one orbit
    /// ahead (`satellite::orbit_path_inertial`): analytic SGP4 from a cloned
    /// element set, or numerical propagation from a GCRF state vector (no TLE
    /// needed). Chosen per object by the scene; a scene may mix both. The
    /// path, like the marker position, is the render input that is not
    /// derivable from `time` alone.
    pub propagation: satellite::Propagation,
}

/// One tracked satellite's readout for the UI panel. A scene builds a
/// `Vec<SatelliteTelemetry>` on demand in its `crate::engine::ui::UIDrawable`
/// impl by re-propagating each satellite at the frame's clock instant - the
/// same instant its `crate::engine::camera::CameraView::frame_state` impl
/// filled [`RenderState::markers`] from, and propagation is deterministic, so
/// the readout matches the rendered markers with no stashed state.
///
/// `pyclass` (fields readable via `get_all`; `skip_from_py_object` - Python
/// only ever receives one, never hands one back) so a `*_py` scene's script
/// reads the same readout its Rust sibling formats.
#[derive(Clone, Debug)]
#[pyclass(module = "globe", get_all, skip_from_py_object)]
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

/// Whether the solid Terra blocks the line of sight from `eye` to `target`
/// (both world-space km). Approximates the planet as a sphere of mean Terra
/// radius - slightly conservative against the WGS84 ellipsoid, which is fine
/// for deciding whether to hide the marker.
pub(crate) fn marker_occluded(eye: DVec3, target: DVec3) -> bool {
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
