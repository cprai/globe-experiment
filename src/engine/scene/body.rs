//! Tracked-body core: the shared state types, the flat per-frame render
//! payload ([`TrackedBody`]), the Terra occlusion test, and the frame
//! helpers both body kinds (`OrbitalBody`, `KinematicBody`) resolve
//! through. Scenes hold their bodies directly, expose them through the
//! derived `SceneOrbitalBodies`/`SceneKinematicBodies` accessors, and get
//! one [`TrackedBody`] per body from the provided `Scene::tracked_bodies`
//! in `frame_state`. Position is never stored - it is a pure function of
//! (elements/state, time), recomputed on demand, so nothing goes stale as
//! the clock advances.

use glam::DVec3;
use satkit::itrfcoord::ITRFCoord;
use satkit::{Duration, Instant, Vector3};

use crate::engine::planet;
use crate::engine::scene::celestial_body::CelestialBody;

/// Segments per predicted trail (one period). 1.4 deg per segment; the chord
/// sagitta at LEO radius (~0.5 km) is sub-pixel at whole-Terra zoom.
pub const TRAIL_SEGMENTS: usize = 256;

/// An instantaneous GCRF orbital state vector - numerical-propagation
/// initial conditions. Deliberately plain data (no satkit types) so a
/// manually-controlled body can construct one with no TLE behind it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitState {
    /// Position, GCRF, meters.
    pub pos_gcrf_m: DVec3,
    /// Velocity, GCRF, meters/second.
    pub vel_gcrf_m_s: DVec3,
}

/// The propagated state at one time, recomputed on demand.
pub struct BodyState {
    /// Position in the renderer's world frame: km, planet center at the
    /// origin, same axes as the Terra body frame.
    pub position_km: DVec3,
    /// Sub-body geodetic latitude, degrees.
    pub latitude_deg: f64,
    /// Sub-body geodetic longitude, degrees.
    pub longitude_deg: f64,
    /// Height above the WGS84 ellipsoid, kilometers.
    pub altitude_km: f64,
    /// The GCRF state vector at the propagated time - the bridge that lets
    /// an SGP4 sample seed a `KinematicBody`.
    pub orbit: OrbitState,
}

/// One tracked body's render payload for one frame: everything the renderer
/// needs to draw the dot and its predicted trail, as plain data. Built per
/// body by the provided `Scene::tracked_bodies` from `state_at`/`trail` +
/// [`body_occluded`].
#[derive(Clone, Debug)]
pub struct TrackedBody {
    /// Dot position in the world frame (km).
    pub position_km: DVec3,
    /// Whether the dot is visible (false when the solid Terra occludes it).
    pub visible: bool,
    /// Predicted trail: `TRAIL_SEGMENTS + 1` world-frame samples (km), the
    /// first at the current position, one period ahead. EMPTY for a
    /// non-elliptic (escape) state, which has no period.
    ///
    /// Trail frame treatment (both body kinds) deliberately differs from
    /// the dot: every inertial sample is rotated Earth-fixed with the
    /// SINGLE rotation at the current time, not each sample's own future
    /// rotation - rendering the star-fixed inertial ellipse (a closed curve
    /// Terra rotates under), not a ground track. The trail floats at
    /// altitude, so no geodetic round trip (that exists on the dot only);
    /// ITRF m -> world km is the plain permutation P (see `coordinates.md`).
    pub trail: Vec<DVec3>,
}

/// Whether the solid Terra blocks the line of sight from `eye` to `target`
/// (both world-space km). Approximates Terra as a mean-radius sphere -
/// slightly conservative vs the WGS84 ellipsoid, fine for dot hiding.
pub fn body_occluded(eye: DVec3, target: DVec3) -> bool {
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
    t > 0.0 && t < distance // Terra sits between the eye and the body
}

/// Shared Earth-fixed tail: ITRF meters -> geodetic -> a world point rebuilt
/// from our own WGS84 helpers (surface point + geodetic normal * altitude).
/// The geodetic round trip is deliberate: it lands the dot on the exact
/// ellipsoid the impostor traces. `orbit` passes through untouched.
pub(crate) fn state_from_itrf(itrf: &Vector3, orbit: OrbitState) -> BodyState {
    let coord = ITRFCoord::from_vector(itrf);
    let (lat_rad, lon_rad, hae_m) = coord.to_geodetic_rad();

    let altitude_km = hae_m / 1000.0;

    let position_km = planet::surface_position(CelestialBody::TERRA, lat_rad, lon_rad)
        + planet::geodetic_normal(CelestialBody::TERRA, lat_rad, lon_rad) * altitude_km;

    BodyState {
        position_km,
        latitude_deg: lat_rad.to_degrees(),
        longitude_deg: lon_rad.to_degrees(),
        altitude_km,
        orbit,
    }
}

/// The `segments + 1` sample instants spanning one period from `time`.
pub(crate) fn path_sample_times(time: &Instant, period_s: f64, segments: usize) -> Vec<Instant> {
    (0..=segments)
        .map(|i| *time + Duration::from_seconds(period_s * i as f64 / segments as f64))
        .collect()
}

/// ITRF meters -> world km: the permutation P (world (x,y,z) = ITRF (y,z,x))
/// plus the unit change.
pub(crate) fn world_km_from_itrf_m(itrf: &Vector3) -> DVec3 {
    DVec3::new(itrf[1] / 1000.0, itrf[2] / 1000.0, itrf[0] / 1000.0)
}
