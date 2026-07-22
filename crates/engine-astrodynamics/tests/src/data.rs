//! Embedded satkit reference data and the harness-owned seeding of
//! satkit's global stores - moved here from the parent crate when it
//! dropped satkit (refactor P4).
//!
//! satkit's data stores are PROCESS-WIDE SET-ONCE: seed them exclusively
//! through the `Once`-guarded [`seed_satkit`], never via satkit's own
//! `init_*` functions, and never combine this harness in one process with
//! any other satkit seeder (e.g. the engine's `init_satkit` - nothing
//! links both today).

use std::sync::{Once, OnceLock};

use engine_astrodynamics::AstroData;
use satkit::frametransform::{IersTableId, init_iers_table_from_bytes};

/// The crate side's eagerly-loaded data, shared across the harness so the
/// kernel parse happens once per test process (the crate API itself has no
/// global - this `OnceLock` is harness convenience only).
pub fn astro() -> &'static AstroData {
    static DATA: OnceLock<AstroData> = OnceLock::new();
    DATA.get_or_init(AstroData::load)
}

/// The astrodyn reference side's ephemeris, the same DE440 excerpt the
/// crate embeds but in SPICE `.bsp` layout. astrodyn delegates to anise
/// like the crate does, so this comparison side cross-checks body mapping,
/// units, and frame wiring rather than the Chebyshev evaluation itself.
/// Per-process `OnceLock` for the same reason as [`astro`].
pub fn astrodyn_eph() -> &'static astrodyn_ephemeris::Ephemeris {
    static EPH: OnceLock<astrodyn_ephemeris::Ephemeris> = OnceLock::new();
    EPH.get_or_init(|| {
        astrodyn_ephemeris::Ephemeris::from_bsp_bytes(DE440S)
            .expect("parse embedded de440s.bsp for the astrodyn reference")
    })
}

/// Forces 8-byte alignment on an embedded blob. `include_bytes!` yields
/// alignment-1 data, but satkit's ephemeris parser reads packed `f64`s with
/// an unaligned-unsafe `copy_nonoverlapping`; the JPL record layout is
/// f64-aligned from file start, so an 8-aligned base makes every read
/// aligned.
#[repr(C, align(8))]
struct Align8<T: ?Sized>(T);

/// The JPL DE440 ephemeris in satkit's native layout (8-aligned).
static EPHEMERIS_ALIGNED: &Align8<[u8]> = &Align8(*include_bytes!(concat!(
    env!("OUT_DIR"),
    "/linux_p1550p2650.440"
)));
const EPHEMERIS: &[u8] = &EPHEMERIS_ALIGNED.0;

/// CelesTrak `EOP-All.csv` (polar motion + UT1-UTC), 1962 to ~build date.
const EOP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/EOP-All.csv"));

/// IERS Conventions 2010 series: CIP X/Y (`TAB5A`/`TAB5B`) and the CIO
/// locator s (`TAB5D`), read by satkit's full GCRF<->ITRF transforms.
const TAB5A: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tab5.2a.txt"));
const TAB5B: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tab5.2b.txt"));
const TAB5D: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tab5.2d.txt"));

/// ICGEM EGM96 gravity coefficients, for satkit's orbit propagator.
const EGM96: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/EGM96.gfc"));

/// The DE440 excerpt in SPICE `.bsp` layout for the astrodyn reference.
/// 8-aligned like the satkit ephemeris above: anise's DAF views read
/// packed `f64`s relative to the byte base.
static DE440S_ALIGNED: &Align8<[u8]> =
    &Align8(*include_bytes!(concat!(env!("OUT_DIR"), "/de440s.bsp")));
const DE440S: &[u8] = &DE440S_ALIGNED.0;

/// Seeds ALL FOUR satkit global stores from embedded bytes: DE440
/// ephemeris, EOP table (+ out-of-range warning disabled; such lookups
/// fall back to zeros), the three IERS-2010 tables, and the EGM96 gravity
/// model. Every seed is load-bearing: each consumes one of satkit's
/// one-shot lazy loaders, which would otherwise create a stray
/// `satkit-data` dir (after which a seed fails `AlreadyInitialized`).
/// Once-guarded, so repeat calls are no-ops. Panics on parse failure.
pub fn seed_satkit() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        satkit::jplephem::init_from_bytes(EPHEMERIS)
            .expect("init JPL ephemeris from embedded bytes");

        satkit::earth_orientation_params::init_from_bytes(EOP)
            .expect("init EOP from embedded bytes");
        satkit::earth_orientation_params::disable_eop_time_warning();

        init_iers_table_from_bytes(IersTableId::Tab5A, TAB5A).expect("init IERS Tab5A from bytes");
        init_iers_table_from_bytes(IersTableId::Tab5B, TAB5B).expect("init IERS Tab5B from bytes");
        init_iers_table_from_bytes(IersTableId::Tab5D, TAB5D).expect("init IERS Tab5D from bytes");

        satkit::earthgravity::init_from_bytes(satkit::earthgravity::GravityModel::EGM96, EGM96)
            .expect("init EGM96 gravity from embedded bytes");
    });
}
