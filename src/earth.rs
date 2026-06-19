//! Physical constants and geometry for planet Earth, in real-world units.
//!
//! This is the single source of truth for the WGS84 reference ellipsoid and
//! Earth's gross physical parameters. World space throughout the renderer is
//! **kilometers** with the planet center at the origin; lengths here are km
//! and the parameterization matches the rest of the project's convention:
//! **+Y is north**, longitude 0 deg / latitude 0 deg faces **+Z**, and +X is
//! east at the prime meridian.
//!
//! The mesh (`mesh::wgs84_ellipsoid`) and the camera (`camera`) both build
//! their geometry from `surface_position` / `geodetic_normal` here, so the
//! ellipsoid math lives in exactly one place.

use glam::Vec3;

/// WGS84 semi-major (equatorial) axis, km.
pub const SEMI_MAJOR_AXIS_KM: f64 = 6378.137;
/// WGS84 inverse flattening (dimensionless).
pub const INVERSE_FLATTENING: f64 = 298.257223563;
/// WGS84 flattening f = (a - b) / a.
pub const FLATTENING: f64 = 1.0 / INVERSE_FLATTENING;
/// WGS84 semi-minor (polar) axis, km: b = a * (1 - f) ~ 6356.752314 km.
pub const SEMI_MINOR_AXIS_KM: f64 = SEMI_MAJOR_AXIS_KM * (1.0 - FLATTENING);
/// First eccentricity squared, e^2 = f * (2 - f) ~ 0.00669438.
pub const ECCENTRICITY_SQ: f64 = FLATTENING * (2.0 - FLATTENING);

/// IUGG mean radius R1 = (2a + b) / 3 ~ 6371.0088 km. Used to convert the
/// camera/projection constants that were tuned in "globe radii" into km, so
/// the interaction feel is preserved exactly.
pub const MEAN_RADIUS_KM: f32 = ((2.0 * SEMI_MAJOR_AXIS_KM + SEMI_MINOR_AXIS_KM) / 3.0) as f32;

// --- Dynamics constants, for orbital simulation built on this geometry. ---
// Not yet consumed by the renderer; provided so satellites/orbits can be
// expressed in the same real-world km/second frame as everything else.
/// Earth's standard gravitational parameter GM (WGS84/EGM), km^3 / s^2.
#[allow(dead_code)]
pub const GRAVITATIONAL_PARAMETER_KM3_S2: f64 = 398600.4418;
/// Earth's sidereal rotation rate, rad / s.
#[allow(dead_code)]
pub const ANGULAR_VELOCITY_RAD_S: f64 = 7.292_115_146_7e-5;

/// Outward unit normal of the WGS84 ellipsoid at the given geodetic
/// latitude/longitude (radians). This is the local "up" used for lighting
/// and for the camera's radial direction. Because the ellipsoid's geodetic
/// normal has the same lat/lon structure as a sphere's radial, this vector
/// is identical to the old unit-sphere direction - which is why the shader's
/// analytic tangent frame and the surface-anchored city-light noise still
/// work unchanged.
pub fn geodetic_normal(latitude: f32, longitude: f32) -> Vec3 {
    let (sin_lat, cos_lat) = (latitude as f64).sin_cos();
    let (sin_lon, cos_lon) = (longitude as f64).sin_cos();

    Vec3::new(
        (cos_lat * sin_lon) as f32,
        sin_lat as f32,
        (cos_lat * cos_lon) as f32,
    )
}

/// Point on the WGS84 ellipsoid surface (height 0) at the given geodetic
/// latitude/longitude (radians), in km. `N` is the prime-vertical radius of
/// curvature; the polar (Y) coordinate carries the `(1 - e^2)` factor that
/// flattens the poles.
pub fn surface_position(latitude: f32, longitude: f32) -> Vec3 {
    let (sin_lat, cos_lat) = (latitude as f64).sin_cos();
    let (sin_lon, cos_lon) = (longitude as f64).sin_cos();

    let n = SEMI_MAJOR_AXIS_KM / (1.0 - ECCENTRICITY_SQ * sin_lat * sin_lat).sqrt();
    let horizontal = n * cos_lat;

    Vec3::new(
        (horizontal * sin_lon) as f32,
        (n * (1.0 - ECCENTRICITY_SQ) * sin_lat) as f32,
        (horizontal * cos_lon) as f32,
    )
}
