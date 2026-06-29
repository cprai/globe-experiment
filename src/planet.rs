//! Physical constants and geometry for the seven classical planets, in
//! real-world units - the multi-body sibling of [`crate::terra`] and
//! [`crate::luna`].
//!
//! The planets are no longer a separate enum: they are variants of
//! [`CelestialBody`] (`Mercury`..`Neptune`). This module hangs their
//! planet-specific data and geometry off those variants - the oblate radii, the
//! IAU rotation constants, the texture file, and the body-fixed surface
//! points/normals - in the same parameterization convention as the Terra/Luna
//! (**+Y is north** / the rotation pole, prime meridian -> **+Z**, +X is 90 deg
//! east). The orientation of each body frame in the world is supplied
//! separately by the ephemeris-driven IAU rotation (see `celestial_sphere`);
//! here we only define the geometry + the IAU constants it consumes. It
//! deliberately depends on neither satkit nor wgpu, exactly like
//! `terra`/`luna`.
//!
//! Each planet is modeled as an **oblate ellipsoid of revolution** (equatorial
//! radius along +X/+Z, polar radius along +Y). For the gas giants the
//! flattening is large and visible (Saturn ~10%); for the rocky planets it is
//! tiny but modeled for uniformity.

use glam::Vec3;

use crate::simulation::body::CelestialBody;

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

/// Per-planet constants table entry.
struct Data {
    /// Equatorial radius (km); the +X and +Z semi-axes.
    equatorial_radius_km: f64,
    /// Polar radius (km); the +Y (rotation-pole) semi-axis.
    polar_radius_km: f64,
    /// `OUT_DIR` file name of the equirectangular albedo map (downloaded
    /// verbatim by build.rs). The renderer `include_bytes!`-es these in `ALL`
    /// order; kept here as the single source of the mapping.
    texture_file: &'static str,
    rotation: Rotation,
}

impl CelestialBody {
    /// The planet's constants. `const fn` so the table is evaluated at compile
    /// time and the accessors below stay zero-cost. Panics on a non-planet body
    /// (the Terra/Luna have their own geometry modules); the accessors are only
    /// ever reached for planet variants.
    const fn planet_data(self) -> Data {
        match self {
            // Mercury: a near-perfect sphere; prograde, very slow spin.
            CelestialBody::Mercury => Data {
                equatorial_radius_km: 2439.7,
                polar_radius_km: 2439.7,
                texture_file: "8k_mercury.jpg",
                rotation: Rotation {
                    ra0_deg: 281.0103,
                    ra0_rate_per_century: -0.0328,
                    dec0_deg: 61.4155,
                    dec0_rate_per_century: -0.0049,
                    w0_deg: 329.5988,
                    w_rate_per_day: 6.1385108,
                },
            },
            // Venus: sphere; retrograde spin (negative w_rate), nearly upright.
            CelestialBody::Venus => Data {
                equatorial_radius_km: 6051.8,
                polar_radius_km: 6051.8,
                texture_file: "8k_venus_surface.jpg",
                rotation: Rotation {
                    ra0_deg: 272.76,
                    ra0_rate_per_century: 0.0,
                    dec0_deg: 67.16,
                    dec0_rate_per_century: 0.0,
                    w0_deg: 160.20,
                    w_rate_per_day: -1.4813688,
                },
            },
            // Mars: slightly oblate; ~24.6 h prograde day.
            CelestialBody::Mars => Data {
                equatorial_radius_km: 3396.19,
                polar_radius_km: 3376.20,
                texture_file: "8k_mars.jpg",
                rotation: Rotation {
                    ra0_deg: 317.269,
                    ra0_rate_per_century: -0.10,
                    dec0_deg: 54.432,
                    dec0_rate_per_century: -0.06,
                    w0_deg: 176.049863,
                    w_rate_per_day: 350.891982443297,
                },
            },
            // Jupiter: strongly oblate; ~9.9 h System III rotation.
            CelestialBody::Jupiter => Data {
                equatorial_radius_km: 71492.0,
                polar_radius_km: 66854.0,
                texture_file: "8k_jupiter.jpg",
                rotation: Rotation {
                    ra0_deg: 268.056595,
                    ra0_rate_per_century: -0.006499,
                    dec0_deg: 64.495303,
                    dec0_rate_per_century: 0.002413,
                    w0_deg: 284.95,
                    w_rate_per_day: 870.5360000,
                },
            },
            // Saturn: most oblate of all; ~10.7 h rotation.
            CelestialBody::Saturn => Data {
                equatorial_radius_km: 60268.0,
                polar_radius_km: 54364.0,
                texture_file: "8k_saturn.jpg",
                rotation: Rotation {
                    ra0_deg: 40.589,
                    ra0_rate_per_century: -0.036,
                    dec0_deg: 83.537,
                    dec0_rate_per_century: -0.004,
                    w0_deg: 38.90,
                    w_rate_per_day: 810.7939024,
                },
            },
            // Uranus: oblate; retrograde spin about a nearly equator-on pole.
            CelestialBody::Uranus => Data {
                equatorial_radius_km: 25559.0,
                polar_radius_km: 24973.0,
                texture_file: "2k_uranus.jpg",
                rotation: Rotation {
                    ra0_deg: 257.311,
                    ra0_rate_per_century: 0.0,
                    dec0_deg: -15.175,
                    dec0_rate_per_century: 0.0,
                    w0_deg: 203.81,
                    w_rate_per_day: -501.1600928,
                },
            },
            // Neptune: oblate; prograde (the N-libration trig terms dropped).
            CelestialBody::Neptune => Data {
                equatorial_radius_km: 24764.0,
                polar_radius_km: 24341.0,
                texture_file: "2k_neptune.jpg",
                rotation: Rotation {
                    ra0_deg: 299.36,
                    ra0_rate_per_century: 0.0,
                    dec0_deg: 43.46,
                    dec0_rate_per_century: 0.0,
                    w0_deg: 249.978,
                    w_rate_per_day: 541.1397757,
                },
            },
            // The Terra/Luna are not planets - they have their own geometry
            // modules and never reach the planet accessors.
            CelestialBody::TerraSystem(_) => {
                panic!("planet data requested for a non-planet body")
            }
        }
    }

    /// Equatorial semi-axis (+X/+Z) in km. The renderer uses it for the
    /// apparent-size test and to size + trace the billboard impostor ellipsoid.
    /// Planet variants only.
    pub fn equatorial_radius_km(self) -> f32 {
        self.planet_data().equatorial_radius_km as f32
    }

    /// Polar semi-axis (+Y, the rotation pole) in km; the impostor's
    /// oblateness. Planet variants only.
    pub fn polar_radius_km(self) -> f32 {
        self.planet_data().polar_radius_km as f32
    }

    /// The IAU rotational elements (evaluated against time in
    /// `celestial_sphere`). Planet variants only.
    pub fn rotation(self) -> Rotation {
        self.planet_data().rotation
    }

    /// `OUT_DIR` file name of the planet's albedo texture. Planet variants
    /// only.
    pub fn texture_file(self) -> &'static str {
        self.planet_data().texture_file
    }
}

/// Point on the oblate planet ellipsoid at the given planetographic
/// latitude/longitude (radians), in the body frame (km). Parametric form (the
/// sphere direction with each axis scaled by its semi-axis), matching the
/// equirectangular texture exactly as `terra`/`luna` do. `body` must be a
/// planet variant.
pub fn surface_position(body: CelestialBody, latitude: f32, longitude: f32) -> Vec3 {
    let d = body.planet_data();
    let (req, rpol) = (d.equatorial_radius_km, d.polar_radius_km);
    let (sin_lat, cos_lat) = (latitude as f64).sin_cos();
    let (sin_lon, cos_lon) = (longitude as f64).sin_cos();

    Vec3::new(
        (req * cos_lat * sin_lon) as f32,
        (rpol * sin_lat) as f32,
        (req * cos_lat * cos_lon) as f32,
    )
}

/// Outward unit normal of the oblate ellipsoid at the given latitude/longitude
/// (radians), in the body frame. The ellipsoid gradient `(x/rx^2, y/ry^2,
/// z/rz^2)` (with `rx = rz = req`, `ry = rpol`), not the radial direction - so
/// the gas giants' visible flattening lights correctly at the poles. `body`
/// must be a planet variant.
pub fn geodetic_normal(body: CelestialBody, latitude: f32, longitude: f32) -> Vec3 {
    let d = body.planet_data();
    let (req, rpol) = (d.equatorial_radius_km, d.polar_radius_km);
    let (sin_lat, cos_lat) = (latitude as f64).sin_cos();
    let (sin_lon, cos_lon) = (longitude as f64).sin_cos();

    Vec3::new(
        (cos_lat * sin_lon / (req * req)) as f32,
        (sin_lat / (rpol * rpol)) as f32,
        (cos_lat * cos_lon / (req * req)) as f32,
    )
    .normalize()
}
