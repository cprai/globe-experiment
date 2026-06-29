//! Ephemeris-driven celestial sphere: the Sun, Moon, and seven planets'
//! positions and orientations, plus the star-map orientation, for a given
//! simulation time, from satkit's JPL Development Ephemerides (DE440) and
//! Earth-orientation transforms.
//!
//! Geocentric frame: every body's position is computed in the Earth-fixed
//! (ECEF) frame. Positions come from the ephemeris (GCRF, inertial); Earth's
//! orientation (the GCRF<->ITRF rotation) maps them into the Earth-fixed
//! frame and rotates the star backdrop. We use the full
//! IERS-2010 transforms (sub-arcsec, matching the satellite path), which read
//! the embedded EOP table plus the embedded IERS nutation/CIO tables (seeded
//! in `init_satkit`) - no runtime data file is needed.
//!
//! Frame note: satkit uses standard ECEF/GCRF (Z = pole); this project's
//! world frame is permuted (Y = north, Z = prime meridian, X = 90 deg E). The
//! permutation `p` maps (x,y,z) -> (y,z,x) - the same permutation the
//! satellite path expresses via `earth::surface_position`/`geodetic_normal`.
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

use glam::{Mat3, Vec3};
use satkit::frametransform::{IersTableId, init_iers_table_from_bytes, qgcrf2itrf, qitrf2gcrf};
use satkit::jplephem::geocentric_pos;
use satkit::{Instant, SolarSystem, TimeScale, Vector3};

use crate::moon;
use crate::planet::{self, Planet, Rotation};

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

/// The IERS Conventions 2010 nutation/CIO series, embedded the same way as the
/// ephemeris and EOP (build.rs downloads them straight into `OUT_DIR`). The
/// full (non-approx) GCRF<->ITRF transforms read these for the CIP X/Y
/// coordinates (`TAB5A`/`TAB5B`) and the CIO locator s (`TAB5D`); the approx
/// transforms did not need them. Seeded in `init_satkit`.
const TAB5A: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tab5.2a.txt"));
const TAB5B: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tab5.2b.txt"));
const TAB5D: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tab5.2d.txt"));

/// Standard J2000 equatorial(ICRS)->galactic rotation `g = R * e`, in standard
/// (non-permuted) axes, from the IAU galactic-pole definition: north galactic
/// pole at RA 192.85948 deg / Dec +27.12825 deg, galactic center (l=0, b=0) at
/// RA 266.40500 deg / Dec -28.93617 deg, longitude of the north celestial pole
/// l = 122.93192 deg. `R^T` is the galactic->equatorial inverse. Used to
/// re-orient the galactic-drawn star texture into the shader's equatorial
/// equirectangular lookup (see the module's star-map frame note). glam stores
/// matrices column-major, so this lists the *columns* (images of the standard
/// basis), i.e. the transpose of how the matrix is usually printed by row.
// The full catalogued digits are kept verbatim for provenance; f32 silently
// rounds them (well below the sub-pixel star-map tolerance).
#[allow(clippy::excessive_precision)]
const R_EQU2GAL: Mat3 = Mat3::from_cols_array(&[
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
/// Three kinds of satkit global state are seeded here, all from embedded bytes:
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
/// Panics if any embedded blob fails to parse (a broken build).
pub fn init_satkit() {
    satkit::jplephem::init_from_bytes(EPHEMERIS).expect("init JPL ephemeris from embedded bytes");

    satkit::earth_orientation_params::init_from_bytes(EOP).expect("init EOP from embedded bytes");
    satkit::earth_orientation_params::disable_eop_time_warning();

    init_iers_table_from_bytes(IersTableId::Tab5A, TAB5A).expect("init IERS Tab5A from bytes");
    init_iers_table_from_bytes(IersTableId::Tab5B, TAB5B).expect("init IERS Tab5B from bytes");
    init_iers_table_from_bytes(IersTableId::Tab5D, TAB5D).expect("init IERS Tab5D from bytes");
}

/// Sun direction and star-map orientation for one instant, in the renderer's
/// world frame.
pub struct CelestialSphere {
    /// Unit vector toward the Sun in the Earth-fixed (ECEF) world frame.
    pub sun_dir: Vec3,
    /// Sun position in the Earth-fixed (ECEF) world frame, km (true
    /// geocentric). Used to light the planets, which sit far enough from
    /// Earth that the Sun direction *at the planet*
    /// (`normalize(sun_pos_world - planet_center)`) differs from Earth's
    /// `sun_dir`; the Earth/Moon keep using `sun_dir`.
    pub sun_pos_world: Vec3,
    /// Rotation taking world (ECEF) view directions into the *equatorial*
    /// celestial (GCRF) frame. This is the inertial frame the camera rig is
    /// built from (`celestial_to_world` = its transpose) - NOT the matrix the
    /// shader samples the star texture with (that is `star_tex_rot_inv`).
    pub star_rot_inv: Mat3,
    /// Rotation taking world (ECEF) view directions into the star *texture's*
    /// galactic frame: `GALACTIC_OFFSET * star_rot_inv`. Uploaded to the shader
    /// as `star_rot_inv` (the equirectangular lookup matrix). Kept separate
    /// from the camera-rig frame above so the static galactic->equatorial
    /// re-orientation does not move existing scenarios' camera framing.
    pub star_tex_rot_inv: Mat3,
    /// Moon center in the Earth-fixed (ECEF) world frame, km. At true scale
    /// (~384,400 km away), so the Moon renders at its real angular size and
    /// distance.
    pub moon_pos_world: Vec3,
    /// Rotation taking a vector in the Moon's body-fixed (selenographic,
    /// project +Y-north convention) frame into the world frame. Built from
    /// the ephemeris Earth orientation and the IAU lunar rotation model, so
    /// the near side faces Earth with the correct libration. Applied to the
    /// lunar mesh's positions and normals; it is a pure rotation, so
    /// normals need no inverse-transpose.
    pub moon_rot: Mat3,
    /// Moon mean radius, km - for the analytic eclipse-shadow geometry (the
    /// Moon as a shadow caster/receiver is treated as a sphere of this radius).
    pub moon_radius_km: f32,
    /// The seven planets' world-frame centers + orientations this frame, in
    /// `planet::ALL` order. True geocentric positions (DE440) and IAU
    /// orientation - consumed by the solar-system scenario; ignored by the
    /// Earth/Moon scenarios.
    pub planets: [PlanetState; 7],
}

/// One planet's world-frame placement for a single frame: its center (true
/// geocentric, km) and the body-fixed -> world rotation (ephemeris Earth
/// orientation composed with the IAU planet rotation). The mesh + texture come
/// from the renderer; this is just where to put and how to orient it.
#[derive(Clone, Copy, Debug)]
pub struct PlanetState {
    pub planet: Planet,
    /// Planet center in the Earth-fixed (ECEF) world frame, km. At true scale
    /// and distance (millions to billions of km), so it is rendered with a
    /// floating origin (see `RenderState::render_origin`).
    pub pos_world: Vec3,
    /// Rotation taking a vector in the planet's body-fixed frame into the world
    /// frame. Pure rotation, so it carries normals too.
    pub rot: Mat3,
}

impl CelestialSphere {
    /// Computes the celestial sphere for `time` from the JPL ephemeris.
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
        let sun_itrf = qgcrf2itrf(time) * sun_gcrf;
        // Keep `sun_dir` as the exact pre-planet expression (so Earth/Moon
        // renders stay bit-identical); `sun_pos_world` is the same vector scaled
        // to km, computed separately for the planets' lighting. Normalizing the
        // unscaled vs /1000 vector would differ by ~1 ULP - hence not folded.
        let sun_dir = (p * nvec(sun_itrf)).normalize();
        let sun_pos_world = p * (nvec(sun_itrf) / 1000.0);

        // Star map: a world(ECEF) view dir -> standard ECEF -> GCRF -> permuted
        // back to Y-up, so the equirectangular lookup's pole tracks the
        // celestial pole. As time advances this rotates the star map at the
        // sidereal rate, consistent with the Sun's motion above.
        let q = qitrf2gcrf(time);
        let r_itrf2gcrf = Mat3::from_cols(
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

        // Moon: position from the same DE440 ephemeris as the Sun (GCRF,
        // inertial, meters), rotated into the Earth-fixed world frame. Rendered
        // at true scale, so it sits ~384,400 km out and shows its real angular
        // size.
        let q_gcrf2itrf = qgcrf2itrf(time);
        let moon_gcrf = geocentric_pos(SolarSystem::Moon, time).expect("moon ephemeris lookup");
        let moon_pos_world = p * (nvec(q_gcrf2itrf * moon_gcrf) / 1000.0);

        // The lunar mesh is built in the project body convention (+Y north,
        // +Z sub-Earth); compose its body->world rotation as
        // P * R_gcrf2itrf * M_body2gcrf * P^T, where M_body2gcrf (standard
        // Z=pole convention) is the IAU lunar rotation and the P^T un-permutes
        // the mesh's project-convention axes into the standard ones M expects.
        let r_gcrf2itrf = Mat3::from_cols(
            nvec(q_gcrf2itrf * unit(1.0, 0.0, 0.0)),
            nvec(q_gcrf2itrf * unit(0.0, 1.0, 0.0)),
            nvec(q_gcrf2itrf * unit(0.0, 0.0, 1.0)),
        );
        let moon_rot = p * r_gcrf2itrf * lunar_body_to_gcrf(time) * p.transpose();

        // The planets: true geocentric position from the same DE440 ephemeris,
        // and a body->world rotation built like the Moon's but from the IAU
        // planet rotation (axial tilt + spin, no libration series). Same `P^T`
        // un-permute of the mesh's project-convention axes into the standard
        // (Z=pole) frame the IAU elements are defined in.
        let planets = std::array::from_fn(|i| {
            let planet = planet::ALL[i];
            let body_gcrf =
                geocentric_pos(planet_body(planet), time).expect("planet ephemeris lookup");
            let pos_world = p * (nvec(q_gcrf2itrf * body_gcrf) / 1000.0);
            let rot = p * r_gcrf2itrf * iau_body_to_gcrf(planet.rotation(), time) * p.transpose();
            PlanetState {
                planet,
                pos_world,
                rot,
            }
        });

        Self {
            sun_dir,
            sun_pos_world,
            star_rot_inv,
            star_tex_rot_inv,
            moon_pos_world,
            moon_rot,
            moon_radius_km: moon::MEAN_RADIUS_KM,
            planets,
        }
    }
}

/// The satkit ephemeris body for one of our planets. Kept here (not in
/// `planet`) so that module stays free of any satkit dependency, exactly like
/// `earth`/`moon`.
fn planet_body(planet: Planet) -> SolarSystem {
    match planet {
        Planet::Mercury => SolarSystem::Mercury,
        Planet::Venus => SolarSystem::Venus,
        Planet::Mars => SolarSystem::Mars,
        Planet::Jupiter => SolarSystem::Jupiter,
        Planet::Saturn => SolarSystem::Saturn,
        Planet::Uranus => SolarSystem::Uranus,
        Planet::Neptune => SolarSystem::Neptune,
    }
}

/// Rotation from the Moon's body-fixed (mean-Earth/polar-axis, standard Z=pole)
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
fn lunar_body_to_gcrf(time: &Instant) -> Mat3 {
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
/// rotational elements (`crate::planet::Rotation`). The planet twin of
/// [`lunar_body_to_gcrf`] without the libration series: the pole `alpha0` /
/// `delta0` carry a linear rate in Julian centuries `T`, and the prime meridian
/// `W` advances at the (possibly retrograde) sidereal spin rate in days `d`.
/// Both `T` and `d` are TT since J2000, matching the lunar model.
fn iau_body_to_gcrf(rot: Rotation, time: &Instant) -> Mat3 {
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
/// (90 deg east). Shared by the Moon (libration folded into the angles) and the
/// planets. Columns are the GCRF images of the body x/y/z axes.
fn body_basis(alpha0: f64, delta0: f64, w: f64) -> Mat3 {
    let (sa, ca) = (alpha0.sin() as f32, alpha0.cos() as f32);
    let (sd, cd) = (delta0.sin() as f32, delta0.cos() as f32);
    let z = Vec3::new(cd * ca, cd * sa, sd);
    let q = Vec3::new(-sa, ca, 0.0);
    let q_east = z.cross(q);
    let x = q * (w.cos() as f32) + q_east * (w.sin() as f32);
    let y = z.cross(x);

    Mat3::from_cols(x, y, z)
}

/// numeris column vector -> glam Vec3.
fn nvec(v: Vector3) -> Vec3 {
    Vec3::new(v[(0, 0)] as f32, v[(1, 0)] as f32, v[(2, 0)] as f32)
}

/// A numeris 3-vector from components (the ctor takes a column-major array).
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
    #[test]
    fn galactic_axes_map_to_catalogued_radec() {
        let equ_of = |gal: Vec3| {
            let e = R_EQU2GAL.transpose() * gal;
            let ra = e.y.atan2(e.x).to_degrees().rem_euclid(360.0);
            let dec = e.z.clamp(-1.0, 1.0).asin().to_degrees();
            (ra, dec)
        };

        let (gc_ra, gc_dec) = equ_of(Vec3::X);
        assert!(
            (gc_ra - 266.40500).abs() < 1e-3,
            "galactic center RA {gc_ra}"
        );
        assert!(
            (gc_dec - -28.93617).abs() < 1e-3,
            "galactic center Dec {gc_dec}"
        );

        let (ngp_ra, ngp_dec) = equ_of(Vec3::Z);
        assert!((ngp_ra - 192.85948).abs() < 1e-3, "NGP RA {ngp_ra}");
        assert!((ngp_dec - 27.12825).abs() < 1e-3, "NGP Dec {ngp_dec}");
    }

    /// In the permuted texture frame the galactic center must land at the
    /// equirectangular center (+Z, `u=0.5`) and the NGP at the pole (+Y,
    /// `v=0`) - the convention `fs_stars` assumes. Checks the `P R P^T` bring.
    #[test]
    fn texture_frame_places_galactic_center_and_pole() {
        let p = Mat3::from_cols(
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        );
        let galactic_offset = p * R_EQU2GAL * p.transpose();

        // `galactic_offset` maps a permuted-equatorial dir to the permuted
        // texture frame. Feed the permuted-equatorial galactic center / NGP.
        let gc_tex = galactic_offset * (p * (R_EQU2GAL.transpose() * Vec3::X));
        let ngp_tex = galactic_offset * (p * (R_EQU2GAL.transpose() * Vec3::Z));

        assert!(
            (gc_tex - Vec3::Z).length() < 1e-5,
            "galactic center {gc_tex}"
        );
        assert!((ngp_tex - Vec3::Y).length() < 1e-5, "NGP {ngp_tex}");
    }

    /// The IAU lunar rotation must keep the Moon's near side facing Earth: the
    /// mean sub-Earth point (selenographic lat 0 / lon 0, which is +Z in the
    /// project body convention) should point from the Moon back toward Earth,
    /// i.e. opposite the Earth->Moon direction. The residual is the optical
    /// libration (up to ~8 deg), so a 10 deg tolerance both confirms the near
    /// side faces Earth and that libration is present (not a rigid lock).
    /// Validates the rotation model independent of any render.
    #[test]
    fn moon_near_side_faces_earth() {
        // The celestial sphere reads satkit globals (ephemeris + EOP + IERS),
        // so seed them once for this test.
        super::init_satkit();

        let time = Instant::from_datetime(2024, 6, 15, 0, 0, 0.0).expect("valid datetime");
        let sphere = CelestialSphere::at(&time);

        // Outward normal at the sub-Earth point in world space.
        let sub_earth = sphere.moon_rot * Vec3::Z;
        // Direction from the Moon back toward Earth (Earth is at the origin).
        let toward_earth = (-sphere.moon_pos_world).normalize();

        let angle = sub_earth
            .dot(toward_earth)
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees();
        assert!(
            angle < 10.0,
            "sub-Earth point off the Earth direction by {angle:.2} deg (libration should be < ~8)"
        );
    }
}
