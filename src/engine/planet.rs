//! Physical constants and geometry for every non-Terra celestial body - the
//! seven classical planets AND Luna - in real-world units; the multi-body
//! sibling of [`crate::engine::terra`].
//!
//! The bodies are not a separate enum: they are variants of [`CelestialBody`]
//! (`Mercury`..`Neptune`, plus `TerraSystem(Luna)`). This module hangs their
//! body-specific data and geometry off those variants - the triaxial radii,
//! the simple IAU rotation constants (planets only), the texture file, and
//! the body-fixed surface points/normals - in the same parameterization
//! convention as Terra (**+Y is north** / the rotation pole, prime meridian
//! -> **+Z**, +X is 90 deg east). The orientation of each body frame in the
//! world is supplied separately by the ephemeris-driven IAU rotation (see
//! `celestial_sphere`); here we only define the geometry + the IAU constants
//! it consumes. It deliberately depends on neither satkit nor wgpu, exactly
//! like `terra`.
//!
//! Every body is modeled as a **triaxial ellipsoid** (independent semi-axes
//! on +X/+Y/+Z). Each planet has its +X and +Z semi-axes equal - its familiar
//! oblate spheroid of revolution (equatorial radius along +X/+Z, polar radius
//! along +Y; for the gas giants the flattening is large and visible, Saturn
//! ~10%, for the rocky planets tiny but modeled for uniformity). Luna is the
//! one genuinely triaxial body (long axis toward Terra); the shared
//! formulation is what lets one geometry pipeline and one impostor trace
//! serve all eight.

use glam::Vec3;

use crate::engine::simulation::body::{CelestialBody, TerraSystemEntity};

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

/// Per-body constants table entry.
struct Data {
    /// Triaxial semi-axes (km) in the body frame: `[+X (90 deg east),
    /// +Y (the rotation pole), +Z (the prime meridian)]`.
    radii_km: [f64; 3],
    /// `OUT_DIR` file name of the equirectangular albedo map (downloaded
    /// verbatim by build.rs). The renderer `include_bytes!`-es these in its
    /// impostor-slot order; kept here as the single source of the mapping.
    texture_file: &'static str,
    /// Simple pole + linear-spin IAU series; `None` for Luna, whose
    /// orientation is the full IAU lunar rotation with the 13 libration
    /// arguments (`celestial_sphere::lunar_body_to_gcrf`), not a table entry.
    rotation: Option<Rotation>,
}

/// Semi-axes of a spheroid of revolution (rx = rz = equatorial) - every
/// planet's shape. Keeps the equatorial value written once per planet so the
/// two equal axes cannot drift.
const fn spheroid_km(equatorial: f64, polar: f64) -> [f64; 3] {
    [equatorial, polar, equatorial]
}

impl CelestialBody {
    /// The body's constants. `const fn` so the table is evaluated at compile
    /// time and the accessors below stay zero-cost. Panics on Terra (the only
    /// body with its own geometry module, `terra` - it is a textured mesh
    /// with WGS84 dynamics, not an impostor); the accessors are only ever
    /// reached for non-Terra variants.
    const fn body_data(self) -> Data {
        match self {
            // Mercury: a near-perfect sphere; prograde, very slow spin.
            CelestialBody::Mercury => Data {
                radii_km: spheroid_km(2439.7, 2439.7),
                texture_file: "8k_mercury.jpg",
                rotation: Some(Rotation {
                    ra0_deg: 281.0103,
                    ra0_rate_per_century: -0.0328,
                    dec0_deg: 61.4155,
                    dec0_rate_per_century: -0.0049,
                    w0_deg: 329.5988,
                    w_rate_per_day: 6.1385108,
                }),
            },
            // Venus: sphere; retrograde spin (negative w_rate), nearly upright.
            CelestialBody::Venus => Data {
                radii_km: spheroid_km(6051.8, 6051.8),
                texture_file: "8k_venus_surface.jpg",
                rotation: Some(Rotation {
                    ra0_deg: 272.76,
                    ra0_rate_per_century: 0.0,
                    dec0_deg: 67.16,
                    dec0_rate_per_century: 0.0,
                    w0_deg: 160.20,
                    w_rate_per_day: -1.4813688,
                }),
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
                texture_file: "8k_moon.jpg",
                rotation: None,
            },
            // Mars: slightly oblate; ~24.6 h prograde day.
            CelestialBody::Mars => Data {
                radii_km: spheroid_km(3396.19, 3376.20),
                texture_file: "8k_mars.jpg",
                rotation: Some(Rotation {
                    ra0_deg: 317.269,
                    ra0_rate_per_century: -0.10,
                    dec0_deg: 54.432,
                    dec0_rate_per_century: -0.06,
                    w0_deg: 176.049863,
                    w_rate_per_day: 350.891982443297,
                }),
            },
            // Jupiter: strongly oblate; ~9.9 h System III rotation.
            CelestialBody::Jupiter => Data {
                radii_km: spheroid_km(71492.0, 66854.0),
                texture_file: "8k_jupiter.jpg",
                rotation: Some(Rotation {
                    ra0_deg: 268.056595,
                    ra0_rate_per_century: -0.006499,
                    dec0_deg: 64.495303,
                    dec0_rate_per_century: 0.002413,
                    w0_deg: 284.95,
                    w_rate_per_day: 870.5360000,
                }),
            },
            // Saturn: most oblate of all; ~10.7 h rotation.
            CelestialBody::Saturn => Data {
                radii_km: spheroid_km(60268.0, 54364.0),
                texture_file: "8k_saturn.jpg",
                rotation: Some(Rotation {
                    ra0_deg: 40.589,
                    ra0_rate_per_century: -0.036,
                    dec0_deg: 83.537,
                    dec0_rate_per_century: -0.004,
                    w0_deg: 38.90,
                    w_rate_per_day: 810.7939024,
                }),
            },
            // Uranus: oblate; retrograde spin about a nearly equator-on pole.
            CelestialBody::Uranus => Data {
                radii_km: spheroid_km(25559.0, 24973.0),
                texture_file: "2k_uranus.jpg",
                rotation: Some(Rotation {
                    ra0_deg: 257.311,
                    ra0_rate_per_century: 0.0,
                    dec0_deg: -15.175,
                    dec0_rate_per_century: 0.0,
                    w0_deg: 203.81,
                    w_rate_per_day: -501.1600928,
                }),
            },
            // Neptune: oblate; prograde (the N-libration trig terms dropped).
            CelestialBody::Neptune => Data {
                radii_km: spheroid_km(24764.0, 24341.0),
                texture_file: "2k_neptune.jpg",
                rotation: Some(Rotation {
                    ra0_deg: 299.36,
                    ra0_rate_per_century: 0.0,
                    dec0_deg: 43.46,
                    dec0_rate_per_century: 0.0,
                    w0_deg: 249.978,
                    w_rate_per_day: 541.1397757,
                }),
            },
            // Terra is the only body outside this table - it is a textured
            // mesh with its own WGS84 module (`terra`), never an impostor,
            // and never reaches these accessors.
            CelestialBody::TerraSystem(TerraSystemEntity::Terra) => {
                panic!("body data requested for Terra (see the terra module)")
            }
        }
    }

    /// The triaxial semi-axes in km, in the body frame (+X 90 deg east,
    /// +Y the rotation pole, +Z the prime meridian). The renderer traces the
    /// impostor ellipsoid from these (and sizes the quad by the largest
    /// axis). Every body except Terra.
    pub fn radii_km(self) -> Vec3 {
        let [rx, ry, rz] = self.body_data().radii_km;
        Vec3::new(rx as f32, ry as f32, rz as f32)
    }

    /// The simple IAU rotational elements (evaluated against time in
    /// `celestial_sphere`). Planet variants only - Luna's orientation is the
    /// full IAU lunar series with libration in `celestial_sphere`, so its
    /// table entry deliberately has none.
    pub fn rotation(self) -> Rotation {
        self.body_data()
            .rotation
            .expect("simple IAU rotation requested for Luna (use the lunar series)")
    }

    /// `OUT_DIR` file name of the body's albedo texture. Every body except
    /// Terra.
    pub fn texture_file(self) -> &'static str {
        self.body_data().texture_file
    }
}

/// Point on the triaxial body ellipsoid at the given planetographic
/// latitude/longitude (radians), in the body frame (km). Parametric form (the
/// sphere direction with each axis scaled by its semi-axis), matching the
/// equirectangular texture exactly as `terra` does. Every body except Terra.
pub fn surface_position(body: CelestialBody, latitude: f32, longitude: f32) -> Vec3 {
    let [rx, ry, rz] = body.body_data().radii_km;
    let (sin_lat, cos_lat) = (latitude as f64).sin_cos();
    let (sin_lon, cos_lon) = (longitude as f64).sin_cos();

    Vec3::new(
        (rx * cos_lat * sin_lon) as f32,
        (ry * sin_lat) as f32,
        (rz * cos_lat * cos_lon) as f32,
    )
}

/// Outward unit normal of the triaxial ellipsoid at the given
/// latitude/longitude (radians), in the body frame. The ellipsoid gradient
/// `(x/rx^2, y/ry^2, z/rz^2)`, not the radial direction - so the gas giants'
/// visible flattening lights correctly at the poles (for the near-spherical
/// Luna it is within ~0.04 deg of radial, but the true normal keeps the
/// lighting geometrically honest). Every body except Terra.
pub fn geodetic_normal(body: CelestialBody, latitude: f32, longitude: f32) -> Vec3 {
    let [rx, ry, rz] = body.body_data().radii_km;
    let (sin_lat, cos_lat) = (latitude as f64).sin_cos();
    let (sin_lon, cos_lon) = (longitude as f64).sin_cos();

    Vec3::new(
        (cos_lat * sin_lon / (rx * rx)) as f32,
        (sin_lat / (ry * ry)) as f32,
        (cos_lat * cos_lon / (rz * rz)) as f32,
    )
    .normalize()
}
