//! Embedded astronomical data (DE440 SPKs, the 1962-2125 Earth PCK,
//! planetary constants, packed EGM2008, space weather) and [`AstroData`],
//! the eagerly-loaded bundle over it. No process-global state and no lazy
//! parsing: [`AstroData::load`] parses everything up front, and every
//! data-dependent API function takes `&AstroData` as its first argument.

use anise::almanac::Almanac;
use anise::prelude::{BPC, SPK};
use anise::structure::PlanetaryDataSet;

use crate::propagation::forces::atmosphere::SpaceWeatherTable;
use crate::propagation::forces::harmonics::EgmTable;

/// Forces 8-byte alignment on an embedded blob: anise's zero-copy DAF
/// parser casts the base to `[f64]` and errors on a misaligned start,
/// while `include_bytes!` yields alignment-1 data.
#[repr(C, align(8))]
struct Align8<T: ?Sized>(T);

/// JPL DE440 excerpt (1849-2150) and full span (1550-2650), anise `.bsp`
/// format. The two are byte-identical where they overlap;
/// [`AstroData::load`] loads full-then-excerpt so the excerpt serves the
/// overlap and the full file everything outside it.
static DE440S_ALIGNED: &Align8<[u8]> =
    &Align8(*include_bytes!(concat!(env!("OUT_DIR"), "/de440s.bsp")));
static DE440S: &[u8] = &DE440S_ALIGNED.0;
static DE440_ALIGNED: &Align8<[u8]> =
    &Align8(*include_bytes!(concat!(env!("OUT_DIR"), "/de440.bsp")));
static DE440: &[u8] = &DE440_ALIGNED.0;

/// High-precision binary Earth PCK (ITRF93), 1962-2125 - the GCRF<->ITRF
/// rotation source (nutation, polar motion, UT1 baked in by NAIF).
static EARTH_BPC_ALIGNED: &Align8<[u8]> = &Align8(*include_bytes!(concat!(
    env!("OUT_DIR"),
    "/earth_1962_250826_2125_combined.bpc"
)));
static EARTH_BPC: &[u8] = &EARTH_BPC_ALIGNED.0;

/// anise planetary-constants kernel (per-body mu/radii/flattening).
/// DER-encoded - no alignment requirement.
static PCK11: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pck11.pca"));

/// EGM2008 to degree/order 360, packed by `build.rs` into the crate-owned
/// little-endian format (40-byte header carrying the model's own GM and
/// reference radius, then the C-bar/S-bar triangular arrays). Parsed into
/// [`AstroData::egm2008`] by [`AstroData::load`].
pub(crate) static EGM2008_PACKED: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/egm2008_n360.le64"));

/// CelesTrak `SW-All.csv` space weather (F10.7, Ap, observed/predicted
/// flags), parsed into [`AstroData::space_weather`] by [`AstroData::load`]
/// - only drag reads it.
pub(crate) static SPACE_WEATHER_CSV: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/SW-All.csv"));

/// The crate's astronomical data, parsed from the embedded kernels and
/// tables in one eager [`load`](Self::load). Every API function that
/// touches data takes `&AstroData`; the caller owns exactly one and
/// decides when the (one-time, ~100 ms) parse cost is paid.
pub struct AstroData {
    pub(crate) almanac: Almanac,
    pub(crate) egm2008: EgmTable,
    pub(crate) space_weather: SpaceWeatherTable,
}

impl AstroData {
    /// Parses ALL embedded data up front: the DE440 SPKs, the Earth PCK,
    /// the planetary-constants kernel, the packed EGM2008 table, and the
    /// space-weather snapshot. Nothing is deferred - queries over the
    /// returned value never parse. Panics on parse failure (a broken
    /// build).
    pub fn load() -> Self {
        let de440 = SPK::from_static(&DE440).expect("parse embedded de440.bsp");
        let de440s = SPK::from_static(&DE440S).expect("parse embedded de440s.bsp");
        let bpc = BPC::from_static(&EARTH_BPC).expect("parse embedded 1962-2125 Earth BPC");
        // The pca travels over plain HTTP (see build.rs); pin the published
        // raw-byte crc32 of the snapshot instead of trusting the transport.
        // (`DataSet::crc32` hashes only the inner payload - not comparable.)
        assert_eq!(
            crc32fast::hash(PCK11),
            0x1edb_3eac,
            "pck11.pca drifted from the pinned snapshot"
        );
        let pca = PlanetaryDataSet::try_from_bytes(PCK11).expect("parse embedded pck11.pca");

        // Load order is the ephemeris-selection rule: anise searches SPKs
        // newest-loaded-first and falls through per file when the epoch is
        // outside its coverage, so de440s (loaded last) serves 1849-2150 and
        // the full de440 serves the 1550-2650 remainder.
        let almanac = Almanac::default()
            .with_spk(de440)
            .with_spk(de440s)
            .with_bpc(bpc)
            .with_planetary_data(pca);
        Self {
            almanac,
            egm2008: EgmTable::parse(EGM2008_PACKED),
            space_weather: SpaceWeatherTable::parse(SPACE_WEATHER_CSV),
        }
    }
}

/// Test-only shared instance so the unit-test suite parses the kernels
/// once per process instead of once per test. Production code paths never
/// touch this - the public API has no global.
#[cfg(test)]
pub(crate) fn test_data() -> &'static AstroData {
    use std::sync::OnceLock;
    static DATA: OnceLock<AstroData> = OnceLock::new();
    DATA.get_or_init(AstroData::load)
}

#[cfg(test)]
mod tests {
    use anise::constants::frames::{EARTH_ITRF93, EARTH_J2000, MOON_J2000};
    use glam::DVec3;
    use hifitime::Epoch;

    use super::*;

    /// Luna's geocentric distance through the loaded data lands in the true
    /// perigee..apogee envelope - the kernels resolve and the frames are
    /// sane. (The tight cross-implementation comparison lives in the
    /// harness, against reference satkit.)
    #[test]
    fn loaded_luna_distance_plausible() {
        let state = test_data()
            .almanac
            .translate(
                MOON_J2000,
                EARTH_J2000,
                Epoch::from_gregorian_utc(2020, 1, 1, 0, 0, 0, 0),
                None,
            )
            .expect("luna geocentric via the almanac");
        let distance_m =
            DVec3::new(state.radius_km.x, state.radius_km.y, state.radius_km.z).length() * 1e3;
        assert!(
            (356_500e3..406_700e3).contains(&distance_m),
            "luna distance {distance_m} m"
        );
    }

    /// Proves the build.rs stream-truncate-repack: header layout, the
    /// model's defining constants, and the first packed coefficient pair.
    #[test]
    fn egm2008_pack_header_and_c20() {
        let read_u32 =
            |at: usize| u32::from_le_bytes(EGM2008_PACKED[at..at + 4].try_into().unwrap());
        let read_f64 =
            |at: usize| f64::from_le_bytes(EGM2008_PACKED[at..at + 8].try_into().unwrap());

        assert_eq!(&EGM2008_PACKED[..8], b"EGM2008\0");
        assert_eq!(read_u32(8), 1, "format version");
        let n_max = read_u32(12) as usize;
        assert_eq!(n_max, 360);
        assert!((read_f64(16) - 3.986_004_415e14).abs() < 1.0, "defining GM");
        assert!((read_f64(24) - 6_378_136.3).abs() < 1e-3, "defining radius");

        let pairs = (n_max + 1) * (n_max + 2) / 2 - 3;
        assert_eq!(EGM2008_PACKED.len(), 40 + 2 * 8 * pairs, "payload size");
        // First entries of each triangular array are (n = 2, m = 0).
        let c20 = read_f64(40);
        let s20 = read_f64(40 + 8 * pairs);
        assert!(
            (c20 - -4.841_651_437_908e-4).abs() < 1e-12,
            "fully-normalized tide-free C(2,0): {c20}"
        );
        assert_eq!(s20, 0.0, "S(2,0) is identically zero");
    }

    /// Pins the BPC-coverage assumption the frame module's infallible
    /// surface relies on: rotations resolve across 1962-2125 and error
    /// outside (anise never extrapolates a binary PCK).
    #[test]
    fn loaded_bpc_covers_1962_2125_only() {
        let rotate = |epoch: Epoch| test_data().almanac.rotate(EARTH_J2000, EARTH_ITRF93, epoch);
        assert!(rotate(Epoch::from_gregorian_utc(1962, 6, 1, 0, 0, 0, 0)).is_ok());
        assert!(rotate(Epoch::from_gregorian_utc(2100, 1, 1, 0, 0, 0, 0)).is_ok());
        assert!(rotate(Epoch::from_gregorian_utc(1950, 1, 1, 0, 0, 0, 0)).is_err());
    }
}
