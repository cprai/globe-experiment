//! Physical constants and geometry for Luna, in real-world units - the
//! lunar twin of [`crate::terra`].
//!
//! World space throughout the renderer is **kilometers**; this module gives the
//! Luna's body-fixed (selenographic) surface points and normals in the same
//! parameterization convention as `terra`: **+Y is north** (the lunar rotation
//! pole), selenographic longitude 0 deg / latitude 0 deg (the mean sub-Terra
//! point) faces **+Z**, and +X is 90 deg east. The orientation of this body
//! frame in the world is supplied separately by the ephemeris-driven lunar
//! rotation (see `celestial_sphere`); here we only define the body geometry.
//!
//! Unlike Terra's WGS84 (an oblate spheroid of revolution), Luna is modeled
//! as a **triaxial** ellipsoid - its three principal axes differ. The long axis
//! points at Terra (tidal bulge), the short axis is the rotation pole. The
//! differences are only ~1-3 km out of ~1737 km, so they are imperceptible at
//! render scale, but the body is genuinely not a sphere of revolution, so the
//! geometry honors that.

use glam::Vec3;

/// IAU/IAG mean radius along the principal axis pointing at Terra (the
/// sub-Terra/prime-meridian direction, +Z here), km. The longest axis.
pub const RADIUS_SUBTERRA_KM: f64 = 1737.4;
/// Mean radius along the along-orbit principal axis (90 deg east, +X here), km.
pub const RADIUS_ALONGORBIT_KM: f64 = 1735.7;
/// Mean radius along the polar (rotation) axis (north, +Y here), km. The
/// shortest axis.
pub const RADIUS_POLAR_KM: f64 = 1734.5;

/// IAU mean volumetric radius, km. Used for the analytic eclipse-shadow test
/// (Luna as a shadow caster is treated as a sphere of this radius - the
/// triaxial difference is far below the penumbra softness).
pub const MEAN_RADIUS_KM: f32 = 1737.4;

/// Semi-axis along world +X (90 deg east).
const RX_KM: f64 = RADIUS_ALONGORBIT_KM;
/// Semi-axis along world +Y (north pole).
const RY_KM: f64 = RADIUS_POLAR_KM;
/// Semi-axis along world +Z (sub-Terra / prime meridian).
const RZ_KM: f64 = RADIUS_SUBTERRA_KM;

/// Point on the triaxial lunar ellipsoid surface at the given selenographic
/// latitude/longitude (radians), in the body frame (km). Parametric form: the
/// sphere direction with each axis scaled by its semi-axis. The flattening is
/// tiny, so parametric vs geodetic latitude differ negligibly; the
/// parameterization matches the equirectangular texture exactly as Terra's
/// does.
pub fn surface_position(latitude: f32, longitude: f32) -> Vec3 {
    let (sin_lat, cos_lat) = (latitude as f64).sin_cos();
    let (sin_lon, cos_lon) = (longitude as f64).sin_cos();

    Vec3::new(
        (RX_KM * cos_lat * sin_lon) as f32,
        (RY_KM * sin_lat) as f32,
        (RZ_KM * cos_lat * cos_lon) as f32,
    )
}

/// Outward unit normal of the triaxial ellipsoid at the given selenographic
/// latitude/longitude (radians), in the body frame. For an ellipsoid the normal
/// is the gradient `(x/rx^2, y/ry^2, z/rz^2)` of the implicit surface, not the
/// radial direction; with Luna's near-sphericity it is within ~0.04 deg of
/// radial, but using the true normal keeps the lighting geometrically honest.
pub fn geodetic_normal(latitude: f32, longitude: f32) -> Vec3 {
    let (sin_lat, cos_lat) = (latitude as f64).sin_cos();
    let (sin_lon, cos_lon) = (longitude as f64).sin_cos();

    Vec3::new(
        (cos_lat * sin_lon / (RX_KM * RX_KM)) as f32,
        (sin_lat / (RY_KM * RY_KM)) as f32,
        (cos_lat * cos_lon / (RZ_KM * RZ_KM)) as f32,
    )
    .normalize()
}
