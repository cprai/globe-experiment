//! Ephemeris-driven sky: the Sun's direction and the star-map orientation for
//! a given simulation time, from satkit's JPL Development Ephemerides (DE440)
//! and Earth-orientation transforms.
//!
//! Geocentric model: the Earth stays the rendered globe at the origin in its
//! Earth-fixed (ECEF) frame. The Sun's position comes from the ephemeris
//! (GCRF, inertial); Earth's orientation (the GCRF<->ITRF rotation) maps it
//! into the Earth-fixed frame and rotates the star backdrop. We use the
//! IAU-76/FK5 "approx" transforms (~1 arcsec, well sub-pixel here), so no
//! Earth-orientation (EOP) data file is needed - only the ephemeris.
//!
//! Frame note: satkit uses standard ECEF/GCRF (Z = pole); this project's
//! world frame is permuted (Y = north, Z = prime meridian, X = 90 deg E). The
//! permutation `p` maps (x,y,z) -> (y,z,x) - the same permutation the
//! satellite path expresses via `earth::surface_position`/`geodetic_normal`.

use glam::{Mat3, Vec3};
use satkit::frametransform::{qgcrf2itrf_approx, qitrf2gcrf_approx};
use satkit::jplephem::geocentric_pos;
use satkit::{Instant, SolarSystem, Vector3};

/// The JPL DE440 ephemeris, embedded in the binary. build.rs downloads it
/// straight into `OUT_DIR` so this `include_bytes!` can pick it up - no runtime
/// data file.
const EPHEMERIS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/linux_p1550p2650.440"));

/// CelesTrak's Earth-orientation parameters (`EOP-All.csv`), embedded the same
/// way as the ephemeris (build.rs downloads it straight into `OUT_DIR`).
/// Provides polar motion + UT1-UTC for accurate ITRF<->GCRF/TEME
/// transforms. Measured data starts 1962-01-01; the file also carries a few
/// months of predictions past the build date. Since this is a past-only
/// simulation tool, the snapshot stays valid for every in-range date.
const EOP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/EOP-All.csv"));

/// Initializes satkit's global state for fully offline, data-dir-free use.
/// Must be called once at startup, before any ephemeris/frame-transform use.
///
/// Two pieces of satkit global state are seeded here, both from embedded bytes:
///
/// 1. The JPL ephemeris singleton (`EPHEMERIS`). satkit lazily loads it from
///    disk on the first position query otherwise, after which this would fail
///    with `AlreadyInitialized`.
///
/// 2. The Earth-orientation-parameter (EOP) table (`EOP`). Seeding it here is
///    needed for two reasons. (a) *Accuracy*: real EOP (polar motion, UT1-UTC)
///    is what lets the ITRF<->GCRF/TEME transforms reach sub-arcsec - the
///    satellite path (`qteme2itrf`) consumes polar motion + UT1-UTC directly,
///    and `gmst`'s UT1 conversion picks up UT1-UTC everywhere. (b) *No stray
///    dir*: every frame transform reads satkit's global EOP table on first use,
///    and satkit's default loader *resolves a data directory and creates an
///    empty `satkit-data` dir next to the binary* as a side effect. Seeding the
///    table up front consumes satkit's one-shot lazy load, so that dir is never
///    created. `disable_eop_time_warning()` silences the once-per-run stderr
///    warning satkit prints for lookups outside the table's date range (e.g. a
///    pre-1962 time), which then fall back to zeros.
///
/// Panics if either embedded blob fails to parse (a broken build).
pub fn init_satkit() {
    satkit::jplephem::init_from_bytes(EPHEMERIS).expect("init JPL ephemeris from embedded bytes");

    satkit::earth_orientation_params::init_from_bytes(EOP).expect("init EOP from embedded bytes");
    satkit::earth_orientation_params::disable_eop_time_warning();
}

/// Sun direction and star-map orientation for one instant, in the renderer's
/// world frame.
pub struct Sky {
    /// Unit vector toward the Sun in the Earth-fixed (ECEF) world frame.
    pub sun_dir: Vec3,
    /// Rotation taking world (ECEF) view directions into the star map's
    /// celestial (GCRF) frame; uploaded to the shader as `star_rot_inv`.
    pub star_rot_inv: Mat3,
    /// Subsolar geodetic latitude/longitude (degrees), for display.
    pub subsolar_lat_deg: f32,
    pub subsolar_lon_deg: f32,
}

impl Sky {
    /// Computes the sky for `time` from the JPL ephemeris.
    pub fn at(time: &Instant) -> Self {
        // P: standard ECEF/GCRF (Z = pole) -> world frame (Y = north),
        // mapping (x, y, z) -> (y, z, x). Orthonormal, so transpose = inverse.
        let p = Mat3::from_cols(
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        );

        // Sun position relative to Earth in GCRF (inertial), meters; rotate to
        // ITRF (Earth-fixed) and permute into the world frame.
        let sun_gcrf = geocentric_pos(SolarSystem::Sun, time).expect("sun ephemeris lookup");
        let sun_itrf = qgcrf2itrf_approx(time) * sun_gcrf;
        let sun_dir = (p * nvec(sun_itrf)).normalize();

        // Star map: a world(ECEF) view dir -> standard ECEF -> GCRF -> permuted
        // back to Y-up, so the equirectangular lookup's pole tracks the
        // celestial pole. As time advances this rotates the sky at the
        // sidereal rate, consistent with the Sun's motion above.
        let q = qitrf2gcrf_approx(time);
        let r_itrf2gcrf = Mat3::from_cols(
            nvec(q * unit(1.0, 0.0, 0.0)),
            nvec(q * unit(0.0, 1.0, 0.0)),
            nvec(q * unit(0.0, 0.0, 1.0)),
        );
        let star_rot_inv = p * r_itrf2gcrf * p.transpose();

        // Subsolar point (inverse of earth::geodetic_normal) for display.
        let subsolar_lat_deg = sun_dir.y.asin().to_degrees();
        let subsolar_lon_deg = sun_dir.x.atan2(sun_dir.z).to_degrees();

        Self {
            sun_dir,
            star_rot_inv,
            subsolar_lat_deg,
            subsolar_lon_deg,
        }
    }
}

/// numeris column vector -> glam Vec3.
fn nvec(v: Vector3) -> Vec3 {
    Vec3::new(v[(0, 0)] as f32, v[(1, 0)] as f32, v[(2, 0)] as f32)
}

/// A numeris 3-vector from components (the ctor takes a column-major array).
fn unit(x: f64, y: f64, z: f64) -> Vector3 {
    Vector3::new([[x], [y], [z]])
}
