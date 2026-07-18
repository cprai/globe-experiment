//! Embedded satkit data (DE440 ephemeris, EOP, IERS tables, EGM96) and the
//! seeding of satkit's global stores.

use std::sync::Once;

use satkit::frametransform::{IersTableId, init_iers_table_from_bytes};

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

/// CelesTrak `EOP-All.csv` (polar motion + UT1-UTC), 1962 to ~build date.
const EOP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/EOP-All.csv"));

/// IERS Conventions 2010 series: CIP X/Y (`TAB5A`/`TAB5B`) and the CIO
/// locator s (`TAB5D`), read by the full GCRF<->ITRF transforms.
const TAB5A: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tab5.2a.txt"));
const TAB5B: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tab5.2b.txt"));
const TAB5D: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tab5.2d.txt"));

/// ICGEM EGM96 gravity coefficients, for the numerical orbit propagator.
const EGM96: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/EGM96.gfc"));

/// Seeds ALL FOUR satkit global stores from embedded bytes: DE440 ephemeris,
/// EOP table (+ out-of-range warning disabled; such lookups fall back to
/// zeros), the three IERS-2010 tables, and the EGM96 gravity model. Every
/// seed is load-bearing even though only the ephemeris is queried today:
/// each consumes one of satkit's one-shot lazy loaders, which would
/// otherwise create a stray `satkit-data` dir (after which a seed fails
/// `AlreadyInitialized`). Once-guarded, so repeat calls are no-ops - but the
/// stores are process-wide set-once, so this must never share a process with
/// the engine's `init_satkit` (nothing links both today). Panics on parse
/// failure (a broken build).
pub fn init() {
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
