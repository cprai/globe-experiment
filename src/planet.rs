//! Physical constants and geometry for the seven classical planets, in
//! real-world units - the multi-body sibling of [`crate::earth`] and
//! [`crate::moon`].
//!
//! Like those modules this is **pure body geometry**: it gives each planet's
//! body-fixed surface points and outward normals in the same parameterization
//! convention (**+Y is north** / the rotation pole, prime meridian -> **+Z**,
//! +X is 90 deg east), plus the data the rest of the program needs to place,
//! orient, and texture the body. The orientation of each body frame in the
//! world is supplied separately by the ephemeris-driven IAU rotation (see
//! `celestial_sphere`); here we only define the geometry + the IAU constants it
//! consumes. It deliberately depends on neither satkit nor wgpu, exactly like
//! `earth`/`moon`.
//!
//! Each planet is modeled as an **oblate ellipsoid of revolution** (equatorial
//! radius along +X/+Z, polar radius along +Y). For the gas giants the
//! flattening is large and visible (Saturn ~10%); for the rocky planets it is
//! tiny but modeled for uniformity.

use glam::Vec3;

/// The seven classical planets, in increasing distance from the Sun. Earth and
/// the Moon are handled by their own modules (`earth`/`moon`); this enum is the
/// set of bodies that share the generic textured/sun-lit planet rendering path.
/// The array order is also the order the renderer loads the planet textures and
/// builds the per-planet meshes/bind groups, so the two must not drift.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Planet {
    Mercury,
    Venus,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
}

/// Every planet, in load/render order. Indexed by the renderer's per-planet GPU
/// resource arrays and by the scenario's body selector.
pub const ALL: [Planet; 7] = [
    Planet::Mercury,
    Planet::Venus,
    Planet::Mars,
    Planet::Jupiter,
    Planet::Saturn,
    Planet::Uranus,
    Planet::Neptune,
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
    name: &'static str,
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

impl Planet {
    /// The planet's constants. `const fn` so the table is evaluated at compile
    /// time and the accessors below stay zero-cost.
    const fn data(self) -> Data {
        match self {
            // Mercury: a near-perfect sphere; prograde, very slow spin.
            Planet::Mercury => Data {
                name: "Mercury",
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
            Planet::Venus => Data {
                name: "Venus",
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
            Planet::Mars => Data {
                name: "Mars",
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
            Planet::Jupiter => Data {
                name: "Jupiter",
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
            Planet::Saturn => Data {
                name: "Saturn",
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
            Planet::Uranus => Data {
                name: "Uranus",
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
            Planet::Neptune => Data {
                name: "Neptune",
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
        }
    }

    /// Display name (e.g. "Jupiter"), for the body-selector readout.
    pub fn name(self) -> &'static str {
        self.data().name
    }

    /// IAU volumetric-ish mean radius (km), `(2*req + rpol)/3`. Used to scale
    /// the camera's distance/zoom/near limits so the interaction feel is the
    /// same fraction of the body whichever planet is orbited.
    pub fn mean_radius_km(self) -> f32 {
        let d = self.data();
        ((2.0 * d.equatorial_radius_km + d.polar_radius_km) / 3.0) as f32
    }

    /// Equatorial semi-axis (+X/+Z) in km. The renderer uses it for the
    /// apparent-size test and to size + trace the billboard impostor ellipsoid.
    pub fn equatorial_radius_km(self) -> f32 {
        self.data().equatorial_radius_km as f32
    }

    /// Polar semi-axis (+Y, the rotation pole) in km; the impostor's
    /// oblateness.
    pub fn polar_radius_km(self) -> f32 {
        self.data().polar_radius_km as f32
    }

    /// The IAU rotational elements (evaluated against time in
    /// `celestial_sphere`).
    pub fn rotation(self) -> Rotation {
        self.data().rotation
    }

    /// `OUT_DIR` file name of the planet's albedo texture.
    pub fn texture_file(self) -> &'static str {
        self.data().texture_file
    }

    /// Point on the oblate planet ellipsoid at the given planetographic
    /// latitude/longitude (radians), in the body frame (km). Parametric form
    /// (the sphere direction with each axis scaled by its semi-axis), matching
    /// the equirectangular texture exactly as `earth`/`moon` do.
    pub fn surface_position(self, latitude: f32, longitude: f32) -> Vec3 {
        let d = self.data();
        let (req, rpol) = (d.equatorial_radius_km, d.polar_radius_km);
        let (sin_lat, cos_lat) = (latitude as f64).sin_cos();
        let (sin_lon, cos_lon) = (longitude as f64).sin_cos();

        Vec3::new(
            (req * cos_lat * sin_lon) as f32,
            (rpol * sin_lat) as f32,
            (req * cos_lat * cos_lon) as f32,
        )
    }

    /// Outward unit normal of the oblate ellipsoid at the given latitude/
    /// longitude (radians), in the body frame. The ellipsoid gradient
    /// `(x/rx^2, y/ry^2, z/rz^2)` (with `rx = rz = req`, `ry = rpol`), not the
    /// radial direction - so the gas giants' visible flattening lights
    /// correctly at the poles.
    pub fn geodetic_normal(self, latitude: f32, longitude: f32) -> Vec3 {
        let d = self.data();
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
}
