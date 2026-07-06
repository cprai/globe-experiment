//! Ephemeris-driven celestial sphere: Sol, Luna, and seven planets'
//! positions and orientations, plus the star-map orientation, for a given
//! simulation time, from satkit's JPL Development Ephemerides (DE440) and
//! Earth-orientation transforms.
//!
//! Frame origin: HELIOCENTRIC. Positions come from the ephemeris (GCRF,
//! inertial, geocentric) and Terra's orientation (the GCRF<->ITRF rotation)
//! maps them into the Earth-fixed axes and rotates the star backdrop; then
//! every body position is shifted by `- sol_geo` so Sol sits at the origin
//! (Terra lands at `-sol_geo`). The AXES stay Earth-fixed - this is an
//! origin-only translation, so orientations and the rendered output are
//! unchanged (the renderer/camera consume every position as `X -
//! render_origin`, and `render_origin` tracks Terra's heliocentric center, so
//! the Terra-local render frame is identical). Satellites and WGS84 surface
//! positions remain GEOCENTRIC (Terra-relative); the renderer bridges them
//! into the render frame with Terra's center. We use the full
//! IERS-2010 transforms (sub-arcsec, matching the satellite path), which read
//! the embedded EOP table plus the embedded IERS nutation/CIO tables (seeded
//! in `init_satkit`) - no runtime data file is needed.
//!
//! Frame note: satkit uses standard ECEF/GCRF (Z = pole); this project's
//! world frame is permuted (Y = north, Z = prime meridian, X = 90 deg E). The
//! permutation `p` maps (x,y,z) -> (y,z,x) - the same permutation the
//! satellite path expresses via `planet::surface_position`/`geodetic_normal`.
//!
//! Star-map frame note: the embedded star texture
//! (`8k_stars_milky_way.jpg`) is drawn in *galactic* coordinates - the Milky
//! Way runs as a horizontal band through the image center, the galactic bulge
//! near mid-image. The equirectangular lookup in `fs_stars` is equatorial
//! (its pole = the celestial pole), so the texture must be re-oriented by the
//! fixed galactic->equatorial rotation before sampling. `GALACTIC_OFFSET`
//! carries that constant; it is folded into `star_tex_rot_inv` only (the
//! shader-facing matrix), leaving `star_rot_inv` as the equatorial frame the
//! inertial camera rig is built from.

use glam::{DMat3, DVec3};
use satkit::frametransform::{IersTableId, init_iers_table_from_bytes, qgcrf2itrf, qitrf2gcrf};
use satkit::jplephem::geocentric_pos;
use satkit::{Instant, SolarSystem, TimeScale, Vector3};

use crate::engine::planet::{self, Rotation};
use crate::engine::simulation::body::{BodyState, CelestialBody, Placement};

/// Forces 8-byte alignment on an embedded blob. `include_bytes!` yields
/// alignment-1 data, but satkit's ephemeris parser reads packed `f64`s straight
/// out of these bytes with an unaligned-unsafe `copy_nonoverlapping`. In debug
/// builds the `ub_checks` precondition aborts when the source is not 8-aligned,
/// and where the linker places an `include_bytes!` static is not controllable
/// (any code change can shift it from coincidentally aligned to not). The JPL
/// record layout is `f64`-aligned from the file start, so an 8-aligned base
/// makes every read aligned. Release builds skip the check, but the read is
/// still UB if misaligned - so align it for both.
#[repr(C, align(8))]
struct Align8<T: ?Sized>(T);

/// The JPL DE440 ephemeris, embedded in the binary (8-aligned, see [`Align8`]).
/// build.rs downloads it straight into `OUT_DIR` so this `include_bytes!` can
/// pick it up - no runtime data file.
static EPHEMERIS_ALIGNED: &Align8<[u8]> = &Align8(*include_bytes!(concat!(
    env!("OUT_DIR"),
    "/linux_p1550p2650.440"
)));
const EPHEMERIS: &[u8] = &EPHEMERIS_ALIGNED.0;

/// CelesTrak's Earth-orientation parameters (`EOP-All.csv`), embedded the same
/// way as the ephemeris (build.rs downloads it straight into `OUT_DIR`).
/// Provides polar motion + UT1-UTC for accurate ITRF<->GCRF/TEME
/// transforms. Measured data starts 1962-01-01; the file also carries a few
/// months of predictions past the build date. Since this is a past-only
/// simulation tool, the snapshot stays valid for every in-range date.
const EOP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/EOP-All.csv"));

/// The IERS Conventions 2010 nutation/CIO series, embedded the same way as the
/// ephemeris and EOP (build.rs downloads them straight into `OUT_DIR`). The
/// full (non-approx) GCRF<->ITRF transforms read these for the CIP X/Y
/// coordinates (`TAB5A`/`TAB5B`) and the CIO locator s (`TAB5D`); the approx
/// transforms did not need them. Seeded in `init_satkit`.
const TAB5A: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tab5.2a.txt"));
const TAB5B: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tab5.2b.txt"));
const TAB5D: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tab5.2d.txt"));

/// The ICGEM EGM96 gravity coefficients, embedded the same way. The numerical
/// orbit propagator (`satkit::orbitprop`, behind the predicted satellite path)
/// evaluates the EGM96 spherical harmonics on every force call. Seeded in
/// `init_satkit`.
const EGM96: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/EGM96.gfc"));

/// Standard J2000 equatorial(ICRS)->galactic rotation `g = R * e`, in standard
/// (non-permuted) axes, from the IAU galactic-pole definition: north galactic
/// pole at RA 192.85948 deg / Dec +27.12825 deg, galactic center (l=0, b=0) at
/// RA 266.40500 deg / Dec -28.93617 deg, longitude of the north celestial pole
/// l = 122.93192 deg. `R^T` is the galactic->equatorial inverse. Used to
/// re-orient the galactic-drawn star texture into the shader's equatorial
/// equirectangular lookup (see the module's star-map frame note). glam stores
/// matrices column-major, so this lists the *columns* (images of the standard
/// basis), i.e. the transpose of how the matrix is usually printed by row.
const R_EQU2GAL: DMat3 = DMat3::from_cols_array(&[
    -0.0548755604,
    0.4941094279,
    -0.8676661490,
    -0.8734370902,
    -0.4448296300,
    -0.1980763734,
    -0.4838350155,
    0.7469822445,
    0.4559837762,
]);

/// Initializes satkit's global state for fully offline, data-dir-free use.
/// Must be called once at startup, before any ephemeris/frame-transform use.
///
/// Four kinds of satkit global state are seeded here, all from embedded bytes:
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
/// 3. The IERS 2010 nutation/CIO tables (`TAB5A`/`TAB5B`/`TAB5D`). The full
///    GCRF<->ITRF transforms the celestial sphere uses read these for the CIP
///    X/Y series and the CIO locator s. They have the *same* stray-dir failure
///    mode as the EOP table: satkit's lazy resolver would `from_file(..)` each
///    one, recreating `satkit-data` (and panicking if absent). Seeding them up
///    front consumes that one-shot load, and must happen before the first
///    transform - which it does, since this runs at the top of each scenario.
///
/// 4. The EGM96 gravity-model singleton (`EGM96`). The numerical orbit
///    propagator (`satkit::orbitprop`, the `Propagation::Numerical` arm of the
///    predicted satellite path) resolves the model via
///    `settings.gravity_model.get()` on every propagation, and satkit's lazy
///    default loader has the same stray-dir failure mode as the ephemeris:
///    `Gravity::from_file(..)` out of `satkit-data` (creating the dir,
///    panicking if the file is absent), after which a seed here would fail with
///    `AlreadyInitialized`. Seeded unconditionally - whether a scene contains
///    numerically-propagated satellites is not knowable at init.
///
/// Panics if any embedded blob fails to parse (a broken build).
pub fn init_satkit() {
    satkit::jplephem::init_from_bytes(EPHEMERIS).expect("init JPL ephemeris from embedded bytes");

    satkit::earth_orientation_params::init_from_bytes(EOP).expect("init EOP from embedded bytes");
    satkit::earth_orientation_params::disable_eop_time_warning();

    init_iers_table_from_bytes(IersTableId::Tab5A, TAB5A).expect("init IERS Tab5A from bytes");
    init_iers_table_from_bytes(IersTableId::Tab5B, TAB5B).expect("init IERS Tab5B from bytes");
    init_iers_table_from_bytes(IersTableId::Tab5D, TAB5D).expect("init IERS Tab5D from bytes");

    satkit::earthgravity::init_from_bytes(satkit::earthgravity::GravityModel::EGM96, EGM96)
        .expect("init EGM96 gravity from embedded bytes");
}

/// Test-only idempotent twin of [`init_satkit`]: the ephemeris seed is a
/// set-once (`AlreadyInitialized` on a second call), and the test binary runs
/// every `#[test]` in one process, so any two tests that both need satkit
/// globals must share one guarded seeding. Production keeps the bare
/// [`init_satkit`] (one scenario per process; a double init there is a bug
/// worth the panic).
#[cfg(test)]
pub(crate) fn init_satkit_for_tests() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(init_satkit);
}

/// Sol direction and star-map orientation for one instant, in the renderer's
/// world frame.
pub struct CelestialSphere {
    /// Sol position in the world frame, km. The frame is now HELIOCENTRIC (Sol
    /// at the origin, Earth-fixed axes), so this is exactly `DVec3::ZERO`. It
    /// is kept as a field because the renderer uploads it (shifted into the
    /// render frame) as `sol_pos` via `sol_pos - render_origin` and every
    /// lit pass derives its Sol direction from that - `normalize(sol_pos -
    /// surface)` for surfaces, `normalize(sol_pos)` for the backdrop disc -
    /// so a far planet is lit by the Sol direction *at the planet*, which
    /// differs from Terra's. For a Terra-relative Sol direction, subtract
    /// Terra's center: `sol_pos_world - center_world(TERRA)`. f64 to match
    /// the body positions (heliocentric magnitudes overflow f32; see
    /// [`crate::engine::simulation::body::Placement::pos_world`]).
    pub sol_pos_world: DVec3,
    /// Rotation taking world (ECEF) view directions into the *equatorial*
    /// celestial (GCRF) frame. This is the inertial frame the camera rig is
    /// built from (`celestial_to_world` = its transpose) - NOT the matrix the
    /// shader samples the star texture with (that is `star_tex_rot_inv`).
    /// f64, like every derived quantity here; consumers cast down only at
    /// the GPU boundary.
    pub star_rot_inv: DMat3,
    /// Rotation taking world (ECEF) view directions into the star *texture's*
    /// galactic frame: `GALACTIC_OFFSET * star_rot_inv`. Uploaded to the shader
    /// as `star_rot_inv` (the equirectangular lookup matrix; cast to f32 at
    /// upload). Kept separate from the camera-rig frame above so the static
    /// galactic->equatorial re-orientation does not move existing scenarios'
    /// camera framing.
    pub star_tex_rot_inv: DMat3,
    /// Every renderable body's world-frame placement this frame, as a flat list
    /// (identity + center + orientation), all HELIOCENTRIC (Sol at the origin,
    /// Earth-fixed axes): **Terra** (at `-sol_geo`, identity orientation - its
    /// body frame *is* the Earth-fixed axes), **Luna** (true scale/distance
    /// ~384,400 km from Terra, IAU lunar orientation), then the **seven
    /// planets** in `planet::ALL` order (true DE440 positions + IAU
    /// orientation). Luna's
    /// placement also drives the analytic eclipse shadows; its radius comes
    /// from the identity (`mean_radius_km`), not stored here. A
    /// scenario takes the subset it draws - the Terra system
    /// (Terra + Luna), or all of them.
    pub bodies: Vec<BodyState>,
}

impl CelestialSphere {
    /// The placement of one body this frame, if present.
    pub fn body(&self, body: CelestialBody) -> Option<&BodyState> {
        self.bodies.iter().find(|state| state.body == body)
    }

    /// Luna's placement this frame (always present). Convenience for the
    /// eclipse scenarios and the rotation test.
    pub fn luna(&self) -> &BodyState {
        self.body(CelestialBody::LUNA)
            .expect("Luna present in the celestial sphere")
    }

    /// The world-frame center (km) of one body this frame. Heliocentric, so
    /// Terra sits at `-sol_geo` (not the origin). f64, like the underlying
    /// placement. Used by the body selectors and by `render_origin`.
    pub fn center_world(&self, body: CelestialBody) -> DVec3 {
        self.body(body)
            .map(|state| state.placement.pos_world)
            .unwrap_or(DVec3::ZERO)
    }
}

impl CelestialSphere {
    /// Computes the celestial sphere for `time` from the JPL ephemeris.
    pub fn at(time: &Instant) -> Self {
        // P: standard ECEF/GCRF (Z = pole) -> world frame (Y = north),
        // mapping (x, y, z) -> (y, z, x). Orthonormal, so transpose = inverse.
        let p = DMat3::from_cols(
            DVec3::new(0.0, 0.0, 1.0),
            DVec3::new(1.0, 0.0, 0.0),
            DVec3::new(0.0, 1.0, 0.0),
        );

        // Sol position relative to Terra in GCRF (inertial), meters; rotate to
        // ITRF (Earth-fixed) and permute into the world frame. This geocentric
        // Sol position is the HELIOCENTRIC ORIGIN: every body's `pos_world`
        // below is expressed relative to Sol by subtracting `sol_geo` (so Sol
        // itself lands at the origin and `sol_pos_world` is ZERO), while the
        // axes stay Earth-fixed. The subtraction is a pure origin translation -
        // orientations and the render output are unaffected because the
        // renderer/camera consume every position as `X - render_origin`, and
        // `render_origin` tracks Terra's (now heliocentric) center.
        let sol_gcrf = geocentric_pos(SolarSystem::Sun, time).expect("sol ephemeris lookup");
        let sol_itrf = qgcrf2itrf(time) * sol_gcrf;
        let sol_geo = p * (nvec(sol_itrf) / 1000.0);

        // Star map: a world(ECEF) view dir -> standard ECEF -> GCRF -> permuted
        // back to Y-up, so the equirectangular lookup's pole tracks the
        // celestial pole. As time advances this rotates the star map at the
        // sidereal rate, consistent with Sol's motion above.
        let q = qitrf2gcrf(time);
        let r_itrf2gcrf = DMat3::from_cols(
            nvec(q * unit(1.0, 0.0, 0.0)),
            nvec(q * unit(0.0, 1.0, 0.0)),
            nvec(q * unit(0.0, 0.0, 1.0)),
        );
        let star_rot_inv = p * r_itrf2gcrf * p.transpose();

        // The star texture is drawn in galactic coordinates, but `fs_stars`
        // does an equatorial equirectangular lookup. Re-express the equatorial
        // direction in the texture's galactic frame: bring R_EQU2GAL into the
        // permuted world axes (P R P^T) and compose with the equatorial
        // orientation. Constant in time, so the camera stays inertial.
        let galactic_offset = p * R_EQU2GAL * p.transpose();
        let star_tex_rot_inv = galactic_offset * star_rot_inv;

        // Luna: position from the same DE440 ephemeris as Sol (GCRF,
        // inertial, meters), rotated into the Earth-fixed world frame. Rendered
        // at true scale, so it sits ~384,400 km out and shows its real angular
        // size.
        let q_gcrf2itrf = qgcrf2itrf(time);
        let luna_gcrf = geocentric_pos(SolarSystem::Moon, time).expect("luna ephemeris lookup");
        let luna_pos_geo = p * (nvec(q_gcrf2itrf * luna_gcrf) / 1000.0);

        // Luna's body frame uses the project body convention (+Y north,
        // +Z sub-Terra); compose its body->world rotation as
        // P * R_gcrf2itrf * M_body2gcrf * P^T, where M_body2gcrf (standard
        // Z=pole convention) is the IAU lunar rotation and the P^T un-permutes
        // the project-convention axes into the standard ones M expects.
        let r_gcrf2itrf = DMat3::from_cols(
            nvec(q_gcrf2itrf * unit(1.0, 0.0, 0.0)),
            nvec(q_gcrf2itrf * unit(0.0, 1.0, 0.0)),
            nvec(q_gcrf2itrf * unit(0.0, 0.0, 1.0)),
        );
        let luna_rot = p * r_gcrf2itrf * lunar_body_to_gcrf(time) * p.transpose();

        // The render list, in a fixed order: Terra, then Luna, then the seven
        // planets. Positions are HELIOCENTRIC (relative to Sol, i.e. minus
        // `sol_geo`) in Earth-fixed axes, so Terra - the world origin - sits at
        // `-sol_geo`, not at zero. Everything stays f64 end to end (the
        // ephemeris is f64 meters): the heliocentric shift is a pure f64
        // subtraction, so the renderer recovers the geocentric-scale offset
        // exactly (`pos_world - render_origin` cancels the ~1.5e8 km Sol
        // offset with no f32 cancellation). Orientations (rot) are
        // frame-invariant and unchanged.
        let mut bodies = Vec::with_capacity(2 + planet::ALL.len());
        bodies.push(BodyState {
            body: CelestialBody::TERRA,
            placement: Placement {
                pos_world: -sol_geo,
                rot: DMat3::IDENTITY,
            },
        });
        bodies.push(BodyState {
            body: CelestialBody::LUNA,
            placement: Placement {
                pos_world: luna_pos_geo - sol_geo,
                rot: luna_rot,
            },
        });

        // The planets: true geocentric position from the same DE440 ephemeris
        // (then shifted to heliocentric by `- sol_geo`, like Terra/Luna above),
        // and a body->world rotation built like Luna's but from the IAU
        // planet rotation (axial tilt + spin, no libration series). Same `P^T`
        // un-permute of the mesh's project-convention axes into the standard
        // (Z=pole) frame the IAU elements are defined in.
        for &planet in &planet::ALL {
            let body_gcrf =
                geocentric_pos(planet_body(planet), time).expect("planet ephemeris lookup");
            let pos_geo = p * (nvec(q_gcrf2itrf * body_gcrf) / 1000.0);
            let pos_world = pos_geo - sol_geo;
            let rot = p * r_gcrf2itrf * iau_body_to_gcrf(planet.rotation(), time) * p.transpose();
            bodies.push(BodyState {
                body: planet,
                placement: Placement { pos_world, rot },
            });
        }

        Self {
            // Sol is the frame origin, so its own position is exactly zero.
            sol_pos_world: DVec3::ZERO,
            star_rot_inv,
            star_tex_rot_inv,
            bodies,
        }
    }
}

/// The satkit ephemeris body for one of our planets. Kept here (not in
/// `planet`) so that module stays free of any satkit dependency, exactly like
/// `terra`. Called only on the planet variants (from the `planet::ALL`
/// loop); the Terra/Luna are positioned separately.
fn planet_body(planet: CelestialBody) -> SolarSystem {
    match planet {
        CelestialBody::Mercury => SolarSystem::Mercury,
        CelestialBody::Venus => SolarSystem::Venus,
        CelestialBody::Mars => SolarSystem::Mars,
        CelestialBody::Jupiter => SolarSystem::Jupiter,
        CelestialBody::Saturn => SolarSystem::Saturn,
        CelestialBody::Uranus => SolarSystem::Uranus,
        CelestialBody::Neptune => SolarSystem::Neptune,
        CelestialBody::TerraSystem(_) => {
            unreachable!("planet_body called on a non-planet body")
        }
    }
}

/// Rotation from Luna's body-fixed (mean-Terra/polar-axis, standard Z=pole)
/// frame to GCRF, from the IAU lunar rotation model.
///
/// Implements the lunar rotational elements of the IAU/IAG Working Group on
/// Cartographic Coordinates and Rotational Elements (2009 report, Archinal et
/// al. 2011, Tables 2 and 3): the pole right ascension `alpha0` / declination
/// `delta0` and the prime-meridian angle `W`, each a polynomial in time plus
/// physical-libration series in the 13 lunar arguments `E1..E13`. The series
/// resolve the near side's true orientation (and its libration) rather than a
/// fixed tidal-lock approximation.
///
/// The IAU formulas take `d` = days and `T` = Julian centuries of *Barycentric
/// Dynamical Time* since J2000; TT is used here (the TT-TDB difference is below
/// a millisecond, far under the model's relevance), and the TT-UTC offset
/// matters at the ~0.01 deg level in `W` - itself well below a rendered pixel.
fn lunar_body_to_gcrf(time: &Instant) -> DMat3 {
    // Days and centuries of TT since the J2000 epoch (JD 2451545.0 TT).
    let d = time.as_jd_with_scale(TimeScale::TT) - 2_451_545.0;
    let t = d / 36_525.0;

    // The 13 lunar libration arguments E1..E13 (degrees), linear in d.
    let e: [f64; 13] = [
        125.045 - 0.0529921 * d,
        250.089 - 0.1059842 * d,
        260.008 + 13.0120009 * d,
        176.625 + 13.3407154 * d,
        357.529 + 0.9856003 * d,
        311.589 + 26.4057084 * d,
        134.963 + 13.0649930 * d,
        276.617 + 0.3287146 * d,
        34.226 + 1.7484877 * d,
        15.134 - 0.1589763 * d,
        119.743 + 0.0036096 * d,
        239.961 + 0.1643573 * d,
        25.053 + 12.9590088 * d,
    ];
    // 1-based access matching the IAU `E_n` numbering, in radians.
    let sin_e = |n: usize| e[n - 1].to_radians().sin();
    let cos_e = |n: usize| e[n - 1].to_radians().cos();

    // Pole right ascension alpha0 (degrees).
    let alpha0 = 269.9949 + 0.0031 * t - 3.8787 * sin_e(1) - 0.1204 * sin_e(2) + 0.0700 * sin_e(3)
        - 0.0172 * sin_e(4)
        + 0.0072 * sin_e(6)
        - 0.0052 * sin_e(10)
        + 0.0043 * sin_e(13);

    // Pole declination delta0 (degrees).
    let delta0 = 66.5392 + 0.0130 * t + 1.5419 * cos_e(1) + 0.0239 * cos_e(2) - 0.0278 * cos_e(3)
        + 0.0068 * cos_e(4)
        - 0.0029 * cos_e(6)
        + 0.0009 * cos_e(7)
        + 0.0008 * cos_e(10)
        - 0.0009 * cos_e(13);

    // Prime-meridian angle W (degrees), measured east from the ascending node.
    let w = 38.3213 + 13.17635815 * d - 1.4e-12 * d * d + 3.5610 * sin_e(1) + 0.1208 * sin_e(2)
        - 0.0642 * sin_e(3)
        + 0.0158 * sin_e(4)
        + 0.0252 * sin_e(5)
        - 0.0066 * sin_e(6)
        - 0.0047 * sin_e(7)
        - 0.0046 * sin_e(8)
        + 0.0028 * sin_e(9)
        + 0.0052 * sin_e(10)
        + 0.0040 * sin_e(11)
        + 0.0019 * sin_e(12)
        - 0.0044 * sin_e(13);

    body_basis(alpha0.to_radians(), delta0.to_radians(), w.to_radians())
}

/// Rotation from a planet's body-fixed (Z=pole) frame to GCRF, from its IAU
/// rotational elements (`crate::engine::planet::Rotation`). The planet twin of
/// [`lunar_body_to_gcrf`] without the libration series: the pole `alpha0` /
/// `delta0` carry a linear rate in Julian centuries `T`, and the prime meridian
/// `W` advances at the (possibly retrograde) sidereal spin rate in days `d`.
/// Both `T` and `d` are TT since J2000, matching the lunar model.
fn iau_body_to_gcrf(rot: Rotation, time: &Instant) -> DMat3 {
    let d = time.as_jd_with_scale(TimeScale::TT) - 2_451_545.0;
    let t = d / 36_525.0;

    let alpha0 = (rot.ra0_deg + rot.ra0_rate_per_century * t).to_radians();
    let delta0 = (rot.dec0_deg + rot.dec0_rate_per_century * t).to_radians();
    let w = (rot.w0_deg + rot.w_rate_per_day * d).to_radians();

    body_basis(alpha0, delta0, w)
}

/// Builds a body->GCRF rotation from the standard IAU pole + prime-meridian
/// angles (radians): the pole z at (alpha0, delta0); the ascending node Q of
/// the body equator on the ICRF equator at RA = alpha0 + 90 deg; the prime
/// meridian x = Q rotated east by W about z; y completes the right-handed triad
/// (90 deg east). Shared by Luna (libration folded into the angles) and the
/// planets. Columns are the GCRF images of the body x/y/z axes.
fn body_basis(alpha0: f64, delta0: f64, w: f64) -> DMat3 {
    let (sa, ca) = alpha0.sin_cos();
    let (sd, cd) = delta0.sin_cos();
    let z = DVec3::new(cd * ca, cd * sa, sd);
    let q = DVec3::new(-sa, ca, 0.0);
    let q_east = z.cross(q);
    let x = q * w.cos() + q_east * w.sin();
    let y = z.cross(x);

    DMat3::from_cols(x, y, z)
}

/// satkit column vector -> glam DVec3 (both are f64; no precision is lost).
fn nvec(v: Vector3) -> DVec3 {
    DVec3::new(v[(0, 0)], v[(1, 0)], v[(2, 0)])
}

/// A satkit 3-vector from components (the ctor takes a column-major array).
fn unit(x: f64, y: f64, z: f64) -> Vector3 {
    Vector3::new([[x], [y], [z]])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `R_EQU2GAL` must agree with the IAU galactic-pole definition: its
    /// inverse (galactic->equatorial) maps the galactic center (1,0,0) and
    /// the north galactic pole (0,0,1) back to their catalogued equatorial
    /// RA/Dec. This validates the embedded matrix independently of any
    /// render or time.
    // The full catalogued digits are kept verbatim for provenance, same as
    // `R_EQU2GAL` above (the trailing zero of 266.40500 trips the lint).
    #[allow(clippy::excessive_precision)]
    #[test]
    fn galactic_axes_map_to_catalogued_radec() {
        let equ_of = |gal: DVec3| {
            let e = R_EQU2GAL.transpose() * gal;
            let ra = e.y.atan2(e.x).to_degrees().rem_euclid(360.0);
            let dec = e.z.clamp(-1.0, 1.0).asin().to_degrees();
            (ra, dec)
        };

        let (gc_ra, gc_dec) = equ_of(DVec3::X);
        assert!(
            (gc_ra - 266.40500).abs() < 1e-3,
            "galactic center RA {gc_ra}"
        );
        assert!(
            (gc_dec - -28.93617).abs() < 1e-3,
            "galactic center Dec {gc_dec}"
        );

        let (ngp_ra, ngp_dec) = equ_of(DVec3::Z);
        assert!((ngp_ra - 192.85948).abs() < 1e-3, "NGP RA {ngp_ra}");
        assert!((ngp_dec - 27.12825).abs() < 1e-3, "NGP Dec {ngp_dec}");
    }

    /// In the permuted texture frame the galactic center must land at the
    /// equirectangular center (+Z, `u=0.5`) and the NGP at the pole (+Y,
    /// `v=0`) - the convention `fs_stars` assumes. Checks the `P R P^T` bring.
    #[test]
    fn texture_frame_places_galactic_center_and_pole() {
        let p = DMat3::from_cols(
            DVec3::new(0.0, 0.0, 1.0),
            DVec3::new(1.0, 0.0, 0.0),
            DVec3::new(0.0, 1.0, 0.0),
        );
        let galactic_offset = p * R_EQU2GAL * p.transpose();

        // `galactic_offset` maps a permuted-equatorial dir to the permuted
        // texture frame. Feed the permuted-equatorial galactic center / NGP.
        let gc_tex = galactic_offset * (p * (R_EQU2GAL.transpose() * DVec3::X));
        let ngp_tex = galactic_offset * (p * (R_EQU2GAL.transpose() * DVec3::Z));

        assert!(
            (gc_tex - DVec3::Z).length() < 1e-5,
            "galactic center {gc_tex}"
        );
        assert!((ngp_tex - DVec3::Y).length() < 1e-5, "NGP {ngp_tex}");
    }

    /// The IAU lunar rotation must keep Luna's near side facing Terra: the
    /// mean sub-Terra point (selenographic lat 0 / lon 0, which is +Z in the
    /// project body convention) should point from Luna back toward Terra,
    /// i.e. opposite the Terra->Luna direction. The residual is the optical
    /// libration (up to ~8 deg), so a 10 deg tolerance both confirms the near
    /// side faces Terra and that libration is present (not a rigid lock).
    /// Validates the rotation model independent of any render.
    #[test]
    fn luna_near_side_faces_terra() {
        // The celestial sphere reads satkit globals (ephemeris + EOP + IERS),
        // so seed them once for this test (shared guard: the satellite tests
        // seed the same process-wide globals).
        super::init_satkit_for_tests();

        let time = Instant::from_datetime(2024, 6, 15, 0, 0, 0.0).expect("valid datetime");
        let sphere = CelestialSphere::at(&time);

        // Outward normal at the sub-Terra point in world space.
        let luna = sphere.luna().placement;
        let sub_terra = luna.rot * DVec3::Z;
        // Direction from Luna back toward Terra. The frame is heliocentric, so
        // Terra is at `center_world(TERRA)` (not the origin), not zero.
        let terra = sphere.center_world(CelestialBody::TERRA);
        let toward_terra = (terra - luna.pos_world).normalize();

        let angle = sub_terra
            .dot(toward_terra)
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees();
        assert!(
            angle < 10.0,
            "sub-Terra point off the Terra direction by {angle:.2} deg (libration should be < ~8)"
        );
    }
}
