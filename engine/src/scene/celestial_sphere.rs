//! Ephemeris-driven celestial sphere: Sol, Luna, and the seven planets'
//! placements plus the star-map orientation for one instant, from satkit's
//! JPL DE440 ephemeris and the full IERS-2010 Earth-orientation transforms
//! (embedded EOP + IERS tables, seeded in `init_satkit` - no runtime data
//! file).
//!
//! Origin: HELIOCENTRIC. Ephemeris positions (GCRF, geocentric) are rotated
//! into Earth-fixed axes, then every body is shifted by `-sol_geo` so Sol
//! sits at the origin (Terra at `-sol_geo`). Origin-only translation: the
//! axes stay Earth-fixed, so orientations and the Terra-local render frame
//! are unchanged. Tracked bodies and WGS84 surface positions stay
//! geocentric.
//!
//! Frame note: satkit uses standard ECEF/GCRF (Z = pole); the world frame is
//! permuted (Y = north, Z = prime meridian, X = 90 deg E) via P: (x,y,z) ->
//! (y,z,x).
//!
//! Star-map note: the embedded star texture is drawn in *galactic*
//! coordinates, but `fs_stars` does an equatorial equirectangular lookup, so
//! the fixed galactic->equatorial rotation is folded into the shader-facing
//! `star_tex_rot_inv` only, leaving `star_rot_inv` as the equatorial frame
//! the inertial camera rig is built from.

use glam::{DMat3, DVec3};
use satkit::frametransform::{IersTableId, init_iers_table_from_bytes, qgcrf2itrf, qitrf2gcrf};
use satkit::jplephem::geocentric_pos;
use satkit::{Instant, SolarSystem, TimeScale, Vector3};

use crate::planet::{self, Rotation};
use crate::scene::celestial_body::{BodyState, CelestialBody, Placement};

/// Forces 8-byte alignment on an embedded blob. `include_bytes!` yields
/// alignment-1 data, but satkit's ephemeris parser reads packed `f64`s with
/// an unaligned-unsafe `copy_nonoverlapping` (debug `ub_checks` abort; still
/// UB in release, and linker placement is not controllable). The JPL record
/// layout is f64-aligned from file start, so an 8-aligned base makes every
/// read aligned.
#[repr(C, align(8))]
struct Align8<T: ?Sized>(T);

/// The JPL DE440 ephemeris, embedded (8-aligned, see [`Align8`]).
static EPHEMERIS_ALIGNED: &Align8<[u8]> = &Align8(*include_bytes!(concat!(
    env!("OUT_DIR"),
    "/linux_p1550p2650.440"
)));
const EPHEMERIS: &[u8] = &EPHEMERIS_ALIGNED.0;

/// CelesTrak `EOP-All.csv` (polar motion + UT1-UTC). Measured data starts
/// 1962-01-01 plus a few months of predictions past the build date - valid
/// for every in-range date of this past-only tool.
const EOP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/EOP-All.csv"));

/// IERS Conventions 2010 series: CIP X/Y (`TAB5A`/`TAB5B`) and the CIO
/// locator s (`TAB5D`), read by the full GCRF<->ITRF transforms.
const TAB5A: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tab5.2a.txt"));
const TAB5B: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tab5.2b.txt"));
const TAB5D: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tab5.2d.txt"));

/// ICGEM EGM96 gravity coefficients, for the numerical orbit propagator.
const EGM96: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/EGM96.gfc"));

/// Standard J2000 equatorial(ICRS)->galactic rotation `g = R * e`, standard
/// axes, from the IAU galactic-pole definition: NGP at RA 192.85948 / Dec
/// +27.12825 deg, galactic center at RA 266.40500 / Dec -28.93617 deg, node
/// longitude l = 122.93192 deg. `R^T` is the inverse. glam is column-major,
/// so this lists the *columns* (transpose of the usual row-printed form).
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

/// Seeds ALL FOUR satkit global stores from embedded bytes: DE440 ephemeris,
/// EOP table (+ out-of-range warning disabled; such lookups fall back to
/// zeros), the three IERS-2010 tables, and the EGM96 gravity model. Every
/// seed is load-bearing: it feeds the full transforms real data AND consumes
/// satkit's one-shot lazy loaders, which would otherwise create a stray
/// `satkit-data` dir (after which a seed fails `AlreadyInitialized`). EGM96
/// is seeded unconditionally - whether a scene numerically propagates is
/// unknowable at init. Do not drop any seed. Panics on parse failure (a
/// broken build).
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

/// Test-only idempotent twin of [`init_satkit`]: the seeds are set-once per
/// process and the test binary runs every `#[test]` in one process, so all
/// satkit-needing tests must share this guard. Production keeps the bare
/// panic-on-double init.
#[cfg(test)]
pub(crate) fn init_satkit_for_tests() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(init_satkit);
}

/// Per-instant body placements, Sol position, and star-map orientation, in
/// the renderer's world frame.
pub struct CelestialSphere {
    /// Sol position in the world frame, km - exactly `DVec3::ZERO` in the
    /// heliocentric frame. Kept as a field because the renderer uploads
    /// `sol_pos - render_origin` and lights every pass from Sol *position*
    /// (a far planet's Sol direction differs from Terra's). For a
    /// Terra-relative Sol direction, subtract `center_world(TERRA)`. f64 to
    /// match the body positions.
    pub sol_pos_world: DVec3,
    /// World (ECEF) view dirs -> the *equatorial* celestial (GCRF) frame:
    /// the inertial frame the camera rig is built from - NOT the shader's
    /// star-texture matrix (that is `star_tex_rot_inv`). f64; cast down only
    /// at the GPU boundary.
    pub star_rot_inv: DMat3,
    /// World view dirs -> the star *texture's* galactic frame
    /// (`GALACTIC_OFFSET * star_rot_inv`), the shader's equirectangular
    /// lookup matrix. Separate from the camera-rig frame so the static
    /// galactic re-orientation does not move existing camera framing.
    pub star_tex_rot_inv: DMat3,
    /// Every renderable body's placement this frame, all HELIOCENTRIC in
    /// Earth-fixed axes: Terra (at `-sol_geo`, identity orientation - its
    /// body frame IS the Earth-fixed axes), Luna (true scale/distance, IAU
    /// lunar orientation), then the seven planets in `planet::ALL` order.
    pub bodies: Vec<BodyState>,
}

impl CelestialSphere {
    /// The placement of one body this frame, if present.
    pub fn body(&self, body: CelestialBody) -> Option<&BodyState> {
        self.bodies.iter().find(|state| state.body == body)
    }

    /// Luna's placement this frame (always present).
    pub fn luna(&self) -> &BodyState {
        self.body(CelestialBody::LUNA)
            .expect("Luna present in the celestial sphere")
    }

    /// One body's world-frame center (km), heliocentric (Terra at
    /// `-sol_geo`, not the origin). f64, like the placement.
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

        // Geocentric Sol: GCRF (m) -> ITRF -> world km. `sol_geo` is the
        // heliocentric origin: every body's `pos_world` below subtracts it -
        // a pure origin translation (axes stay Earth-fixed), so orientations
        // and the render output are unaffected.
        let sol_gcrf = geocentric_pos(SolarSystem::Sun, time).expect("sol ephemeris lookup");
        let sol_itrf = qgcrf2itrf(time) * sol_gcrf;
        let sol_geo = p * (nvec(sol_itrf) / 1000.0);

        // Star map: world(ECEF) view dir -> standard ECEF -> GCRF ->
        // permuted back to Y-up, so the equirectangular pole tracks the
        // celestial pole; rotates at the sidereal rate, consistent with Sol.
        let q = qitrf2gcrf(time);
        let r_itrf2gcrf = DMat3::from_cols(
            nvec(q * unit(1.0, 0.0, 0.0)),
            nvec(q * unit(0.0, 1.0, 0.0)),
            nvec(q * unit(0.0, 0.0, 1.0)),
        );
        let star_rot_inv = p * r_itrf2gcrf * p.transpose();

        // Bring R_EQU2GAL into the permuted world axes (P R P^T) and compose
        // with the equatorial orientation. Constant in time, so the camera
        // stays inertial.
        let galactic_offset = p * R_EQU2GAL * p.transpose();
        let star_tex_rot_inv = galactic_offset * star_rot_inv;

        // Luna: DE440 GCRF position (m) rotated into the Earth-fixed world
        // frame; rendered at true scale/distance (~384,400 km).
        let q_gcrf2itrf = qgcrf2itrf(time);
        let luna_gcrf = geocentric_pos(SolarSystem::Moon, time).expect("luna ephemeris lookup");
        let luna_pos_geo = p * (nvec(q_gcrf2itrf * luna_gcrf) / 1000.0);

        // Luna's body frame uses the project convention (+Y north, +Z
        // sub-Terra): body->world = P * R_gcrf2itrf * M_body2gcrf * P^T,
        // where M (standard Z=pole) is the IAU lunar rotation and P^T
        // un-permutes the project-convention axes into the standard frame M
        // expects.
        let r_gcrf2itrf = DMat3::from_cols(
            nvec(q_gcrf2itrf * unit(1.0, 0.0, 0.0)),
            nvec(q_gcrf2itrf * unit(0.0, 1.0, 0.0)),
            nvec(q_gcrf2itrf * unit(0.0, 0.0, 1.0)),
        );
        let luna_rot = p * r_gcrf2itrf * lunar_body_to_gcrf(time) * p.transpose();

        // Render list, fixed order: Terra, Luna, then the seven planets.
        // Positions heliocentric (- sol_geo) in Earth-fixed axes, f64 end to
        // end: the shift is a pure f64 subtraction, so downstream
        // `pos_world - render_origin` cancels the ~1.5e8 km Sol offset with
        // no f32 cancellation.
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

        // Planets: DE440 position (shifted heliocentric like Terra/Luna) +
        // body->world rotation built like Luna's but from the IAU planet
        // elements (no libration series); same P^T un-permute.
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

/// The satkit ephemeris body for a planet variant. Lives here (not in
/// `planet`) so that module stays satkit-free.
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

/// Luna body-fixed (mean-Terra/polar-axis, standard Z=pole) -> GCRF, from
/// the IAU/IAG WG lunar rotation model (2009 report, Archinal et al. 2011,
/// Tables 2 and 3): pole `alpha0`/`delta0` and prime-meridian angle `W`,
/// each a polynomial in time plus physical-libration series in the 13 lunar
/// arguments `E1..E13` - the true near-side orientation with libration, not
/// a fixed tidal-lock approximation.
///
/// The IAU formulas take `d` days / `T` centuries of TDB since J2000; TT is
/// used here (TT-TDB < 1 ms, far under the model's relevance; the TT-UTC
/// offset matters ~0.01 deg in `W` - well below a rendered pixel).
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

/// Planet body-fixed (Z=pole) -> GCRF from its IAU rotational elements
/// (`planet::Rotation`): the lunar model minus the libration series - linear
/// pole rates in centuries `T`, prime meridian at the (possibly retrograde)
/// sidereal spin rate in days `d`. Both TT since J2000, matching the lunar
/// model.
fn iau_body_to_gcrf(rot: Rotation, time: &Instant) -> DMat3 {
    let d = time.as_jd_with_scale(TimeScale::TT) - 2_451_545.0;
    let t = d / 36_525.0;

    let alpha0 = (rot.ra0_deg + rot.ra0_rate_per_century * t).to_radians();
    let delta0 = (rot.dec0_deg + rot.dec0_rate_per_century * t).to_radians();
    let w = (rot.w0_deg + rot.w_rate_per_day * d).to_radians();

    body_basis(alpha0, delta0, w)
}

/// Body->GCRF rotation from the standard IAU angles (radians): pole z at
/// (alpha0, delta0); ascending node Q of the body equator on the ICRF
/// equator at RA = alpha0 + 90 deg; prime meridian x = Q rotated east by W
/// about z; y completes the right-handed triad. Columns are the GCRF images
/// of the body axes.
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

/// satkit column vector -> glam DVec3 (both f64; no precision lost).
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
    /// inverse maps the galactic center (1,0,0) and the NGP (0,0,1) back to
    /// their catalogued equatorial RA/Dec, independent of any render or time.
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
    /// `v=0`) - the convention `fs_stars` assumes. Checks the `P R P^T`.
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
    /// mean sub-Terra point (+Z in the project body convention) points back
    /// toward Terra. The residual is the optical libration (up to ~8 deg),
    /// so a 10 deg tolerance confirms both the near side and that libration
    /// is present (not a rigid lock).
    #[test]
    fn luna_near_side_faces_terra() {
        // Satkit globals are read here; seed once via the shared test guard
        // (the tracked-body tests seed the same process-wide globals).
        super::init_satkit_for_tests();

        let time = Instant::from_datetime(2024, 6, 15, 0, 0, 0.0).expect("valid datetime");
        let sphere = CelestialSphere::at(&time);

        // Outward normal at the sub-Terra point in world space.
        let luna = sphere.luna().placement;
        let sub_terra = luna.rot * DVec3::Z;
        // Direction from Luna back toward Terra. Heliocentric frame, so
        // Terra is at `center_world(TERRA)`, not zero.
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
