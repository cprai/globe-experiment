//! Physical constants and geometry for EVERY celestial body - Terra, Luna,
//! and the seven classical planets - in real-world units.
//!
//! The bodies are not a separate enum: they are variants of [`CelestialBody`]
//! (`Mercury`..`Neptune`, plus the `TerraSystem` pair). This module hangs
//! their body-specific data and geometry off those variants - the triaxial
//! radii, the simple IAU rotation constants (planets only), the texture
//! files, and the body-fixed surface points/normals - in one shared
//! parameterization convention (**+Y is north** / the rotation pole, prime
//! meridian -> **+Z**, +X is 90 deg east). The orientation of each body frame
//! in the world is supplied separately by the ephemeris-driven IAU rotation
//! (see `celestial_sphere`); here we only define the geometry + the IAU
//! constants it consumes. It deliberately depends on neither satkit nor wgpu.
//!
//! Every body is modeled as a **triaxial ellipsoid** (independent semi-axes
//! on +X/+Y/+Z). Terra and each planet have their +X and +Z semi-axes equal -
//! the familiar oblate spheroid of revolution (equatorial radius along +X/+Z,
//! polar radius along +Y; Terra is the WGS84 reference ellipsoid, the gas
//! giants' flattening is large and visible, Saturn ~10%, the rocky planets'
//! tiny but modeled for uniformity). Luna is the one genuinely triaxial body
//! (long axis toward Terra). The shared formulation is what lets one geometry
//! pipeline and one impostor trace serve all nine.
//!
//! **Latitude convention is shape-driven**: a spheroid of revolution (rx ==
//! rz) uses **geodetic** latitude (the WGS84 formulation - the standard for
//! Terra satellite geodesy and planetographic mapping), while the triaxial
//! body (Luna) uses parametric latitude (triaxial geodetic latitude has no
//! clean closed form, and Luna's ~1-3 km axis spread makes the difference
//! sub-texel). For a sphere the two coincide.

use glam::Vec3;

use crate::engine::simulation::body::{CelestialBody, TerraSystemEntity};

/// WGS84 semi-major (equatorial) axis, km - Terra's defining shape constant.
pub const SEMI_MAJOR_AXIS_KM: f64 = 6378.137;
/// WGS84 inverse flattening (dimensionless).
pub const INVERSE_FLATTENING: f64 = 298.257223563;
/// WGS84 flattening f = (a - b) / a.
pub const FLATTENING: f64 = 1.0 / INVERSE_FLATTENING;
/// WGS84 semi-minor (polar) axis, km: b = a * (1 - f) ~ 6356.752314 km.
pub const SEMI_MINOR_AXIS_KM: f64 = SEMI_MAJOR_AXIS_KM * (1.0 - FLATTENING);

/// IUGG mean radius R1 = (2a + b) / 3 ~ 6371.0088 km, as a `const` (the
/// non-const [`CelestialBody::mean_radius_km`] computes the same value from
/// the table). Kept for const contexts that need a Terra-scale fallback
/// (e.g. the free-coordinate camera target).
pub const TERRA_MEAN_RADIUS_KM: f32 =
    ((2.0 * SEMI_MAJOR_AXIS_KM + SEMI_MINOR_AXIS_KM) / 3.0) as f32;

// --- Terra dynamics constants, for orbital simulation built on this
// geometry. Not yet consumed anywhere; provided so satellites/orbits can be
// expressed in the same real-world km/second frame as everything else. ---
/// Terra's standard gravitational parameter GM (WGS84/EGM), km^3 / s^2.
#[allow(dead_code)]
pub const TERRA_GRAVITATIONAL_PARAMETER_KM3_S2: f64 = 398600.4418;
/// Terra's sidereal rotation rate, rad / s.
#[allow(dead_code)]
pub const TERRA_ANGULAR_VELOCITY_RAD_S: f64 = 7.292_115_146_7e-5;

/// Every planet, in increasing distance from Sol - the load/render order.
/// Indexed by the renderer's per-planet GPU resource arrays and used to filter
/// the planet entries out of the celestial-body render list. The array order is
/// also the order the renderer loads the planet textures and builds the
/// per-planet meshes/bind groups, so the two must not drift.
pub const ALL: [CelestialBody; 7] = [
    CelestialBody::Mercury,
    CelestialBody::Venus,
    CelestialBody::Mars,
    CelestialBody::Jupiter,
    CelestialBody::Saturn,
    CelestialBody::Uranus,
    CelestialBody::Neptune,
];

/// IAU rotational elements for a planet (IAU/IAG Working Group on Cartographic
/// Coordinates and Rotational Elements, 2009/2015): the north-pole right
/// ascension `alpha0` and declination `delta0` (each a constant plus a linear
/// rate in Julian centuries `T` of TT since J2000), and the prime-meridian
/// angle `W = w0 + w_rate * d` (degrees; `d` = days of TT since J2000;
/// `w_rate` is the sidereal spin, negative for the retrograde rotators Venus
/// and Uranus). The small higher-order libration trig terms (Jupiter, Neptune)
/// are omitted - they are far below a rendered pixel for a viewer - so this
/// captures the axial tilt and the spin phase. Evaluated in `celestial_sphere`.
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
/// verbatim by build.rs). The renderer `include_bytes!`-es these in its
/// impostor-slot order; kept here as the single source of the mapping. Only
/// the albedo is mandatory; the optional maps drive the richer shading
/// features (a body without one renders with that feature off).
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
    /// The body's texture maps (albedo + the optional feature maps).
    maps: Maps,
    /// Simple pole + linear-spin IAU series; `None` for the Terra-system
    /// bodies, whose orientation is not a table entry: Terra's body frame IS
    /// the world frame (identity placement), and Luna's orientation is the
    /// full IAU lunar rotation with the 13 libration arguments
    /// (`celestial_sphere::lunar_body_to_gcrf`).
    rotation: Option<Rotation>,
    /// Whether the body carries the rendered atmosphere (the Hillaire LUT
    /// set). Terra only today; the atmosphere pass draws for a
    /// `has_atmosphere` body sitting at the render origin.
    has_atmosphere: bool,
}

/// Semi-axes of a spheroid of revolution (rx = rz = equatorial) - the shape
/// of Terra and every planet. Keeps the equatorial value written once per
/// body so the two equal axes cannot drift.
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
    /// The body's constants. `const fn` so the table is evaluated at compile
    /// time and the accessors below stay zero-cost.
    const fn body_data(self) -> Data {
        match self {
            // Mercury: a near-perfect sphere; prograde, very slow spin.
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
            // Venus: sphere; retrograde spin (negative w_rate), nearly upright.
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
            // Luna: the one genuinely triaxial body - the long axis points at
            // Terra (the tidal bulge, +Z, the mean sub-Terra direction), the
            // short axis is the rotation pole (+Y), and the along-orbit axis
            // (+X, 90 deg east) sits between. The differences are only ~1-3 km
            // of ~1737 km - imperceptible at render scale - but the body is
            // genuinely not a sphere of revolution, so the geometry honors
            // that. IAU/IAG principal-axis mean radii. Its rotation is None:
            // Luna's orientation needs the full IAU lunar series with
            // libration, implemented in `celestial_sphere`, not the simple
            // pole + linear-spin form.
            CelestialBody::TerraSystem(TerraSystemEntity::Luna) => Data {
                radii_km: [1735.7, 1734.5, 1737.4],
                maps: albedo_only("8k_moon.jpg"),
                rotation: None,
                has_atmosphere: false,
            },
            // Mars: slightly oblate; ~24.6 h prograde day.
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
            // Jupiter: strongly oblate; ~9.9 h System III rotation.
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
            // Saturn: most oblate of all; ~10.7 h rotation.
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
            // Uranus: oblate; retrograde spin about a nearly equator-on pole.
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
            // Neptune: oblate; prograde (the N-libration trig terms dropped).
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
            // Terra: the WGS84 reference ellipsoid, with the full texture-map
            // set (night lights, relief, ocean mask) and the rendered
            // atmosphere. Its rotation is None: the world frame IS the
            // Terra-fixed body frame (identity placement, see
            // `celestial_sphere`), so there is no series to evaluate.
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

    /// The triaxial semi-axes in km, in the body frame (+X 90 deg east,
    /// +Y the rotation pole, +Z the prime meridian). The renderer traces the
    /// impostor ellipsoid from these (and sizes the quad by the largest
    /// axis).
    pub fn radii_km(self) -> Vec3 {
        let [rx, ry, rz] = self.body_data().radii_km;
        Vec3::new(rx as f32, ry as f32, rz as f32)
    }

    /// Characteristic mean radius (km): the axis mean over the triaxial
    /// semi-axes, computed in f64 so Terra's value stays bit-equal to the
    /// classic IUGG (2a + b) / 3 (rx = rz makes the two algebraically
    /// identical; an f32 sum could drift a ULP and micro-shift every camera
    /// distance). The camera scales its distance/zoom limits, near plane, and
    /// pan rate by this so the interaction feel is the same fraction of the
    /// body whichever one is targeted; for Luna it also sets the eclipse
    /// caster/occlusion sphere, where the ~1.5 km triaxial spread is far
    /// below the penumbra softness.
    pub fn mean_radius_km(self) -> f32 {
        let [rx, ry, rz] = self.body_data().radii_km;
        ((rx + ry + rz) / 3.0) as f32
    }

    /// The simple IAU rotational elements (evaluated against time in
    /// `celestial_sphere`). Planet variants only - the Terra-system bodies'
    /// orientation is not a table entry (Terra's body frame is the world
    /// frame; Luna needs the full IAU lunar series with libration), so their
    /// entries deliberately have none.
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

/// Point on the body ellipsoid surface at the given latitude/longitude
/// (radians), in the body frame (km). Shape-driven latitude convention (see
/// the module docs): a spheroid of revolution (rx == rz - Terra and every
/// planet) treats the latitude as **geodetic** and uses the WGS84
/// prime-vertical formulation (`N` is the prime-vertical radius of curvature;
/// the polar (Y) coordinate carries the `(1 - e^2)` factor that flattens the
/// poles - for Terra this is bit-for-bit the WGS84 math, and satellite
/// geodetic coordinates land on the exact same ellipsoid). The triaxial body
/// (Luna) uses the parametric form (the sphere direction with each axis
/// scaled by its semi-axis).
pub fn surface_position(body: CelestialBody, latitude: f32, longitude: f32) -> Vec3 {
    let [rx, ry, rz] = body.body_data().radii_km;
    let (sin_lat, cos_lat) = (latitude as f64).sin_cos();
    let (sin_lon, cos_lon) = (longitude as f64).sin_cos();

    if rx == rz {
        // Spheroid: geodetic latitude. e^2 from the table radii equals the
        // classic f * (2 - f) to f64 rounding, which the f32 cast absorbs.
        let e_sq = 1.0 - (ry * ry) / (rx * rx);
        let n = rx / (1.0 - e_sq * sin_lat * sin_lat).sqrt();
        let horizontal = n * cos_lat;

        Vec3::new(
            (horizontal * sin_lon) as f32,
            (n * (1.0 - e_sq) * sin_lat) as f32,
            (horizontal * cos_lon) as f32,
        )
    } else {
        // Triaxial: parametric latitude.
        Vec3::new(
            (rx * cos_lat * sin_lon) as f32,
            (ry * sin_lat) as f32,
            (rz * cos_lat * cos_lon) as f32,
        )
    }
}

/// Outward unit normal of the body ellipsoid at the given latitude/longitude
/// (radians), in the body frame - the local "up" used for lighting and the
/// camera's radial direction. For a spheroid the latitude is geodetic, and
/// the geodetic normal is by definition the plain sphere direction
/// `(cos_lat*sin_lon, sin_lat, cos_lat*cos_lon)` - the same lat/lon structure
/// as a sphere's radial, which is why the analytic tangent frames and the
/// surface-anchored noise work unchanged on the flattened body. For the
/// triaxial body (parametric latitude) it is the ellipsoid gradient
/// `(x/rx^2, y/ry^2, z/rz^2)`, not the radial direction (for the
/// near-spherical Luna it is within ~0.04 deg of radial, but the true normal
/// keeps the lighting geometrically honest).
pub fn geodetic_normal(body: CelestialBody, latitude: f32, longitude: f32) -> Vec3 {
    let [rx, ry, rz] = body.body_data().radii_km;
    let (sin_lat, cos_lat) = (latitude as f64).sin_cos();
    let (sin_lon, cos_lon) = (longitude as f64).sin_cos();

    if rx == rz {
        Vec3::new(
            (cos_lat * sin_lon) as f32,
            sin_lat as f32,
            (cos_lat * cos_lon) as f32,
        )
    } else {
        Vec3::new(
            (cos_lat * sin_lon / (rx * rx)) as f32,
            (sin_lat / (ry * ry)) as f32,
            (cos_lat * cos_lon / (rz * rz)) as f32,
        )
        .normalize()
    }
}
