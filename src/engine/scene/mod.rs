//! Simulation state and astronomical math: the `Scene` trait, the simulation
//! clock, and the ephemeris-driven celestial sphere (`CelestialSphere::at` is
//! a pure function of time - never stored). Deliberately free of any
//! winit/wgpu/ui/camera dependency.

pub mod body;
pub mod celestial_sphere;
pub mod clock;
pub mod satellite;

use glam::DVec3;
use pyo3::prelude::*;
use satkit::Instant;

pub use body::CelestialBody;
// `SceneClock` is used by both bin trees; `Clock` only by the main tree's
// scenes, so the headless tree (whose crate-level allow covers `dead_code`,
// not `unused_imports`) would warn without the allow.
#[allow(unused_imports)]
pub use clock::{Clock, SceneClock};

use crate::engine::planet;
use celestial_sphere::CelestialSphere;

/// Fallback characteristic radius (km) for a free [`CameraTarget::Coordinate`]:
/// the camera's distance/zoom/pan limits scale by the target's radius, so a
/// body-less point needs one. Terra's mean radius keeps the familiar feel.
const COORDINATE_RADIUS_KM: f64 = planet::TERRA_MEAN_RADIUS_KM;

/// What the orbital camera orbits: a body identity or a free world point.
/// Pure identity - the moving center is NOT stored; it is resolved from the
/// [`CelestialSphere`] on demand (single source of truth). Body-frame
/// accessors (radius, surface anchor) are static and take no sphere.
#[derive(Clone, Copy, Debug)]
pub enum CameraTarget {
    /// Orbit a body by identity. Body centers are heliocentric - far past
    /// f32 precision in world-km - so a target renders with a floating
    /// origin (see [`CameraTarget::render_origin`]); Terra/Luna keep the
    /// origin at Terra's center, leaving their render frame Terra-local.
    Body(CelestialBody),
    /// Orbit a fixed world-frame point (km), treated like a planet target
    /// (own floating origin, synthetic body geometry). No scene constructs
    /// it yet - kept as scaffold for orbiting an arbitrary point.
    #[allow(dead_code)]
    Coordinate(DVec3),
}

impl CameraTarget {
    /// The default Terra target.
    pub fn terra() -> CameraTarget {
        CameraTarget::Body(CelestialBody::TERRA)
    }

    /// Whether two targets name the same orbit subject - how the scene
    /// detects a genuine switch that must reframe the camera. Two
    /// `Coordinate` targets always match (a free point never reframes).
    pub fn same_kind(&self, other: &CameraTarget) -> bool {
        match (self, other) {
            (CameraTarget::Body(a), CameraTarget::Body(b)) => a == b,
            (CameraTarget::Coordinate(_), CameraTarget::Coordinate(_)) => true,
            _ => false,
        }
    }

    /// The orbit center in the world frame (km), heliocentric (Terra at
    /// `-sol_geo`, not the origin). f64, like the placement.
    pub fn center_world(&self, celestial: &CelestialSphere) -> DVec3 {
        match self {
            CameraTarget::Body(body) => celestial.center_world(*body),
            CameraTarget::Coordinate(point) => *point,
        }
    }

    /// The floating-origin the scene is rendered relative to. Terra/Luna are
    /// close enough to Terra's center for f32 world-km, so they keep the
    /// origin there (bit-identical Terra render frame); a planet or free
    /// coordinate is too far out, so the origin shifts to its own center,
    /// restoring f32 precision near the subject. Terra's center is looked up
    /// from the sphere (not hard-coded `ZERO`) so this holds in the
    /// heliocentric frame.
    pub fn render_origin(&self, celestial: &CelestialSphere) -> DVec3 {
        match self {
            // Terra/Luna: origin stays at Terra's center.
            CameraTarget::Body(CelestialBody::TerraSystem(_)) => {
                celestial.center_world(CelestialBody::TERRA)
            }
            // Any planet: too far out for the Terra origin.
            CameraTarget::Body(body) => celestial.center_world(*body),
            CameraTarget::Coordinate(point) => *point,
        }
    }

    /// Characteristic mean radius (km); the camera scales its distance/zoom
    /// limits, near plane, and pan rate by it. Static - no sphere needed.
    pub fn mean_radius_km(&self) -> f64 {
        match self {
            CameraTarget::Body(body) => body.mean_radius_km(),
            CameraTarget::Coordinate(_) => COORDINATE_RADIUS_KM,
        }
    }

    /// Look-at anchor at `(lat, lon)` (radians), body frame (km) - an offset
    /// from the target center. A free coordinate has no surface, so its
    /// anchor is the center itself (zero offset).
    pub fn surface_position(&self, latitude: f64, longitude: f64) -> DVec3 {
        match self {
            CameraTarget::Body(body) => body.surface_position(latitude, longitude),
            CameraTarget::Coordinate(_) => DVec3::ZERO,
        }
    }

    /// Outward unit normal at `(lat, lon)` (radians), body frame - the local
    /// "up". A free coordinate uses the standard spherical direction so
    /// lat/lon still orbit the point.
    pub fn geodetic_normal(&self, latitude: f64, longitude: f64) -> DVec3 {
        match self {
            CameraTarget::Body(body) => body.geodetic_normal(latitude, longitude),
            CameraTarget::Coordinate(_) => {
                CelestialBody::TERRA.geodetic_normal(latitude, longitude)
            }
        }
    }
}

/// The simulation half of every scene. UI- and camera-agnostic: panels go
/// through `ui::UIDrawable`, the frame's `RenderState` through
/// `camera::CameraView` - distinct traits on the same scene struct.
pub trait Scene {
    /// Scene-specific per-frame work (most scenes: none). Runs after the
    /// clock tick; a paused clock yields dt = 0, which pause-sensitive work
    /// keys on (or read `clock_paused()`).
    fn advance(&mut self);

    /// The per-frame entry point: tick the clock, then [`Scene::advance`].
    /// Provided via [`SceneClock`], so no scene hand-writes the clock tick.
    fn tick_scene(&mut self)
    where
        Self: SceneClock,
    {
        self.tick_clock();
        self.advance();
    }
}

/// The minimal render contract for one frame, as plain `glam` data (no GPU
/// types). The renderer re-derives every body's placement from `time` via
/// `CelestialSphere::at`, so this carries only what it cannot recompute:
/// time, camera rig, target, and satellite markers.
#[derive(Clone)]
pub struct RenderState {
    /// The instant the frame depicts. Renderer and simulation evaluate the
    /// same `CelestialSphere::at` here, keeping the orbited body's
    /// render-frame position a bit-exact zero.
    pub time: Instant,
    /// What the camera orbits this frame. MUST be the target the rig was
    /// built for; the renderer reads its `render_origin()` and identity.
    pub camera_target: CameraTarget,
    /// Camera eye in the floating-origin (render) frame (km), relative to
    /// `camera_target.render_origin()`. f64; cast to f32 only at uniform
    /// upload.
    pub camera_pos: DVec3,
    /// Look-at point in the render frame (km) - carried as a point, not a
    /// unit forward: reconstructing a normalized forward and re-projecting
    /// would drift by sub-ULP and speckle every antialiased edge.
    pub camera_look_at: DVec3,
    /// Camera up direction in the world frame (unit).
    pub camera_up: DVec3,
    /// One marker per tracked satellite, in scene order - the one piece of
    /// frame state not derivable from `time`.
    pub markers: Vec<SatelliteMarker>,
}

/// One satellite's on-screen marker for one frame.
#[derive(Clone, Debug)]
pub struct SatelliteMarker {
    /// Marker position in the world frame (km).
    pub position_km: DVec3,
    /// Whether the marker is visible (false when the solid Terra occludes it).
    pub visible: bool,
    /// How the renderer predicts the orbit path about one period ahead:
    /// analytic SGP4 from a cloned element set, or numerical propagation
    /// from a GCRF state (no TLE). Per object; a scene may mix both.
    pub propagation: satellite::Propagation,
}

/// One tracked satellite's UI readout, rebuilt on demand by re-propagating
/// at the frame's clock instant - deterministic, so it matches the rendered
/// markers with no stashed state.
///
/// `pyclass` (`get_all`; `skip_from_py_object` - Python only ever receives
/// one, never hands one back).
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

/// Seeds satkit's global state (embedded ephemeris + EOP + tables). Call
/// once at startup, before any `CelestialSphere` is built.
pub fn init() {
    celestial_sphere::init_satkit();
}

/// Whether the solid Terra blocks the line of sight from `eye` to `target`
/// (both world-space km). Approximates Terra as a mean-radius sphere -
/// slightly conservative vs the WGS84 ellipsoid, fine for marker hiding.
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
