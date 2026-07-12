//! Physical constants and geometry for every celestial body - Terra, Luna,
//! and the seven planets - hung off the [`CelestialBody`] variants, in one
//! shared body-frame convention (+Y = the rotation pole/north, prime
//! meridian -> +Z, +X = 90 deg east).
//!
//! Every body is a triaxial ellipsoid. Terra and each planet have rx == rz
//! (a spheroid of revolution; Terra is the WGS84 reference ellipsoid); Luna
//! is the one genuinely triaxial body. **Latitude convention is
//! shape-driven**: spheroids use geodetic latitude (the WGS84 formulation),
//! triaxial Luna uses parametric (triaxial geodetic latitude has no clean
//! closed form, and Luna's ~1-3 km axis spread makes the difference
//! sub-texel).

use glam::DVec3;

use crate::engine::scene::celestial_body::{CelestialBody, TerraSystemEntity};

/// WGS84 semi-major (equatorial) axis, km - Terra's defining shape constant.
pub const SEMI_MAJOR_AXIS_KM: f64 = 6378.137;
/// WGS84 inverse flattening (dimensionless).
pub const INVERSE_FLATTENING: f64 = 298.257223563;
/// WGS84 flattening f = (a - b) / a.
pub const FLATTENING: f64 = 1.0 / INVERSE_FLATTENING;
/// WGS84 semi-minor (polar) axis, km: b = a * (1 - f) ~ 6356.752314 km.
pub const SEMI_MINOR_AXIS_KM: f64 = SEMI_MAJOR_AXIS_KM * (1.0 - FLATTENING);

/// IUGG mean radius R1 = (2a + b) / 3 ~ 6371.0088 km, as a `const` for const
/// contexts needing a Terra-scale fallback (e.g. the free-coordinate camera
/// target).
pub const TERRA_MEAN_RADIUS_KM: f64 = (2.0 * SEMI_MAJOR_AXIS_KM + SEMI_MINOR_AXIS_KM) / 3.0;

// Terra dynamics constants: not yet consumed; provided so orbital simulation
// can share the same real-world km/second frame.
/// Terra's standard gravitational parameter GM (WGS84/EGM), km^3 / s^2.
#[allow(dead_code)]
pub const TERRA_GRAVITATIONAL_PARAMETER_KM3_S2: f64 = 398600.4418;
/// Terra's sidereal rotation rate, rad / s.
#[allow(dead_code)]
pub const TERRA_ANGULAR_VELOCITY_RAD_S: f64 = 7.292_115_146_7e-5;

/// Every planet, in increasing distance from Sol - the load/render order.
/// Indexes the renderer's per-planet GPU resource arrays and its texture
/// load order; the orders must not drift.
pub const ALL: [CelestialBody; 7] = [
    CelestialBody::Mercury,
    CelestialBody::Venus,
    CelestialBody::Mars,
    CelestialBody::Jupiter,
    CelestialBody::Saturn,
    CelestialBody::Uranus,
    CelestialBody::Neptune,
];

/// IAU rotational elements (IAU/IAG Working Group on Cartographic
/// Coordinates and Rotational Elements, 2009/2015): pole RA/Dec as a
/// constant plus a rate per Julian century `T` of TT since J2000, and the
/// prime-meridian angle `W = w0 + w_rate * d` (degrees; `d` = TT days since
/// J2000; `w_rate` negative for the retrograde rotators Venus and Uranus).
/// The higher-order libration trig terms (Jupiter, Neptune) are deliberately
/// omitted - far below a rendered pixel. Evaluated in `celestial_sphere`.
#[derive(Clone, Copy)]
pub struct Rotation {
    pub ra0_deg: f64,
    pub ra0_rate_per_century: f64,
    pub dec0_deg: f64,
    pub dec0_rate_per_century: f64,
    pub w0_deg: f64,
    pub w_rate_per_day: f64,
}

/// The body's equirectangular texture maps: `OUT_DIR` file names (downloaded
/// verbatim by build.rs) - the single source of the file<->body mapping.
/// Only the albedo is mandatory; a body missing an optional map renders with
/// that shading feature off.
#[derive(Clone, Copy)]
pub struct Maps {
    /// Base color (sRGB).
    pub albedo: &'static str,
    /// Night-side emission source (sRGB; sampled as a luminance mask for the
    /// city-light glow).
    pub night: Option<&'static str>,
    /// Tangent-space normal/relief map (linear - data, not color).
    pub normal: Option<&'static str>,
    /// Specular mask (linear); `.r` = water/ocean.
    pub specular: Option<&'static str>,
}

/// Per-body constants table entry.
struct Data {
    /// Triaxial semi-axes (km) in the body frame: `[+X (90 deg east),
    /// +Y (the rotation pole), +Z (the prime meridian)]`.
    radii_km: [f64; 3],
    maps: Maps,
    /// Simple pole + linear-spin IAU series; `None` for the Terra system:
    /// Terra's body frame IS the world frame (identity placement), and Luna
    /// needs the full IAU lunar series with libration
    /// (`celestial_sphere::lunar_body_to_gcrf`).
    rotation: Option<Rotation>,
    /// Whether the body carries the rendered atmosphere (Terra only today).
    has_atmosphere: bool,
}

/// Semi-axes of a spheroid of revolution (rx = rz = equatorial). Writes the
/// equatorial value once per body so the two equal axes cannot drift.
const fn spheroid_km(equatorial: f64, polar: f64) -> [f64; 3] {
    [equatorial, polar, equatorial]
}

/// Maps entry for a body with only an albedo texture (every body but Terra).
const fn albedo_only(albedo: &'static str) -> Maps {
    Maps {
        albedo,
        night: None,
        normal: None,
        specular: None,
    }
}

impl CelestialBody {
    /// The body's constants; `const fn` so the accessors stay zero-cost.
    const fn body_data(self) -> Data {
        match self {
            CelestialBody::Mercury => Data {
                radii_km: spheroid_km(2439.7, 2439.7),
                maps: albedo_only("8k_mercury.jpg"),
                rotation: Some(Rotation {
                    ra0_deg: 281.0103,
                    ra0_rate_per_century: -0.0328,
                    dec0_deg: 61.4155,
                    dec0_rate_per_century: -0.0049,
                    w0_deg: 329.5988,
                    w_rate_per_day: 6.1385108,
                }),
                has_atmosphere: false,
            },
            CelestialBody::Venus => Data {
                radii_km: spheroid_km(6051.8, 6051.8),
                maps: albedo_only("8k_venus_surface.jpg"),
                rotation: Some(Rotation {
                    ra0_deg: 272.76,
                    ra0_rate_per_century: 0.0,
                    dec0_deg: 67.16,
                    dec0_rate_per_century: 0.0,
                    w0_deg: 160.20,
                    w_rate_per_day: -1.4813688,
                }),
                has_atmosphere: false,
            },
            // Luna: genuinely triaxial - long axis (+Z) toward Terra (the
            // tidal bulge), short axis the rotation pole (+Y). IAU/IAG
            // principal-axis mean radii; the ~1-3 km spread is imperceptible
            // at render scale but the geometry honors it.
            CelestialBody::TerraSystem(TerraSystemEntity::Luna) => Data {
                radii_km: [1735.7, 1734.5, 1737.4],
                maps: albedo_only("8k_moon.jpg"),
                rotation: None,
                has_atmosphere: false,
            },
            CelestialBody::Mars => Data {
                radii_km: spheroid_km(3396.19, 3376.20),
                maps: albedo_only("8k_mars.jpg"),
                rotation: Some(Rotation {
                    ra0_deg: 317.269,
                    ra0_rate_per_century: -0.10,
                    dec0_deg: 54.432,
                    dec0_rate_per_century: -0.06,
                    w0_deg: 176.049863,
                    w_rate_per_day: 350.891982443297,
                }),
                has_atmosphere: false,
            },
            CelestialBody::Jupiter => Data {
                radii_km: spheroid_km(71492.0, 66854.0),
                maps: albedo_only("8k_jupiter.jpg"),
                rotation: Some(Rotation {
                    ra0_deg: 268.056595,
                    ra0_rate_per_century: -0.006499,
                    dec0_deg: 64.495303,
                    dec0_rate_per_century: 0.002413,
                    w0_deg: 284.95,
                    w_rate_per_day: 870.5360000,
                }),
                has_atmosphere: false,
            },
            CelestialBody::Saturn => Data {
                radii_km: spheroid_km(60268.0, 54364.0),
                maps: albedo_only("8k_saturn.jpg"),
                rotation: Some(Rotation {
                    ra0_deg: 40.589,
                    ra0_rate_per_century: -0.036,
                    dec0_deg: 83.537,
                    dec0_rate_per_century: -0.004,
                    w0_deg: 38.90,
                    w_rate_per_day: 810.7939024,
                }),
                has_atmosphere: false,
            },
            CelestialBody::Uranus => Data {
                radii_km: spheroid_km(25559.0, 24973.0),
                maps: albedo_only("2k_uranus.jpg"),
                rotation: Some(Rotation {
                    ra0_deg: 257.311,
                    ra0_rate_per_century: 0.0,
                    dec0_deg: -15.175,
                    dec0_rate_per_century: 0.0,
                    w0_deg: 203.81,
                    w_rate_per_day: -501.1600928,
                }),
                has_atmosphere: false,
            },
            CelestialBody::Neptune => Data {
                radii_km: spheroid_km(24764.0, 24341.0),
                maps: albedo_only("2k_neptune.jpg"),
                rotation: Some(Rotation {
                    ra0_deg: 299.36,
                    ra0_rate_per_century: 0.0,
                    dec0_deg: 43.46,
                    dec0_rate_per_century: 0.0,
                    w0_deg: 249.978,
                    w_rate_per_day: 541.1397757,
                }),
                has_atmosphere: false,
            },
            CelestialBody::TerraSystem(TerraSystemEntity::Terra) => Data {
                radii_km: spheroid_km(SEMI_MAJOR_AXIS_KM, SEMI_MINOR_AXIS_KM),
                maps: Maps {
                    albedo: "8k_earth_daymap.jpg",
                    night: Some("8k_earth_nightmap.jpg"),
                    normal: Some("8k_earth_normal_map.tif"),
                    specular: Some("8k_earth_specular_map.tif"),
                },
                rotation: None,
                has_atmosphere: true,
            },
        }
    }

    /// Triaxial semi-axes in km, body frame (+X 90 deg east, +Y the rotation
    /// pole, +Z the prime meridian). f64; cast to f32 only at the impostor
    /// uniform.
    pub fn radii_km(self) -> DVec3 {
        let [rx, ry, rz] = self.body_data().radii_km;
        DVec3::new(rx, ry, rz)
    }

    /// Axis-mean radius (km; for rx = rz this is algebraically the classic
    /// IUGG (2a + b) / 3). Scales the camera's distance/zoom limits, near
    /// plane, and pan rate so interaction feel is the same fraction of any
    /// targeted body; for Luna it also sets the eclipse caster/occlusion
    /// sphere, where the ~1.5 km triaxial spread is far below the penumbra
    /// softness.
    pub fn mean_radius_km(self) -> f64 {
        let [rx, ry, rz] = self.body_data().radii_km;
        (rx + ry + rz) / 3.0
    }

    /// The simple IAU rotational elements - planet variants only; the
    /// Terra-system bodies deliberately have none (see `Data::rotation`).
    pub fn rotation(self) -> Rotation {
        self.body_data()
            .rotation
            .expect("simple IAU rotation requested for a Terra-system body")
    }

    /// The body's texture maps (`OUT_DIR` file names).
    pub fn maps(self) -> Maps {
        self.body_data().maps
    }

    /// Whether the body carries the rendered atmosphere (Terra only today).
    pub fn has_atmosphere(self) -> bool {
        self.body_data().has_atmosphere
    }
}

/// Point on the body ellipsoid at latitude/longitude (radians), body frame
/// (km). A spheroid (rx == rz) treats latitude as **geodetic** via the WGS84
/// prime-vertical formulation - for Terra this is bit-for-bit the WGS84
/// math, so satellite geodetic coordinates land on the exact same ellipsoid;
/// triaxial Luna uses the parametric form.
pub fn surface_position(body: CelestialBody, latitude: f64, longitude: f64) -> DVec3 {
    let [rx, ry, rz] = body.body_data().radii_km;
    let (sin_lat, cos_lat) = latitude.sin_cos();
    let (sin_lon, cos_lon) = longitude.sin_cos();

    if rx == rz {
        // Spheroid: geodetic latitude. e^2 from the table radii equals the
        // classic f * (2 - f) to f64 rounding.
        let e_sq = 1.0 - (ry * ry) / (rx * rx);
        let n = rx / (1.0 - e_sq * sin_lat * sin_lat).sqrt();
        let horizontal = n * cos_lat;

        DVec3::new(
            horizontal * sin_lon,
            n * (1.0 - e_sq) * sin_lat,
            horizontal * cos_lon,
        )
    } else {
        // Triaxial: parametric latitude.
        DVec3::new(rx * cos_lat * sin_lon, ry * sin_lat, rz * cos_lat * cos_lon)
    }
}

/// Outward unit normal at latitude/longitude (radians), body frame. For a
/// spheroid the geodetic normal is by definition the plain sphere direction -
/// why sphere-based tangent frames and surface-anchored noise work unchanged
/// on the flattened body. The triaxial body uses the ellipsoid gradient
/// `(x/rx^2, y/ry^2, z/rz^2)`, not the radial direction (within ~0.04 deg on
/// near-spherical Luna, but geometrically honest).
pub fn geodetic_normal(body: CelestialBody, latitude: f64, longitude: f64) -> DVec3 {
    let [rx, ry, rz] = body.body_data().radii_km;
    let (sin_lat, cos_lat) = latitude.sin_cos();
    let (sin_lon, cos_lon) = longitude.sin_cos();

    if rx == rz {
        DVec3::new(cos_lat * sin_lon, sin_lat, cos_lat * cos_lon)
    } else {
        DVec3::new(
            cos_lat * sin_lon / (rx * rx),
            sin_lat / (ry * ry),
            cos_lat * cos_lon / (rz * rz),
        )
        .normalize()
    }
}
