//! Orbit math for the solar-system renderer: ephemeris lookups over
//! embedded anise/JPL DE440 kernels ([`ephemeris`]), inertial/Earth-fixed
//! frame rotations ([`frames`]), osculating elements ([`kepler`]), WGS84
//! geodetic conversion ([`geodetic`]), SGP4 over element sets ([`tle`] +
//! [`sgp4`]), and the crate's own deep-space numerical propagator
//! ([`propagation`]). Standalone - not yet consumed by `engine`, which
//! still runs on satkit until its migration.
//!
//! No process-global state and no lazy loading: [`AstroData::load`] parses
//! every embedded kernel and table up front, and each data-dependent API
//! function takes `&AstroData` as its first argument - the caller owns the
//! data and decides when the one-time parse cost is paid.

mod data;
pub mod ephemeris;
pub mod frames;
pub mod geodetic;
pub mod kepler;
pub mod propagation;
mod segments;
pub mod sgp4;
pub mod tle;

pub use data::AstroData;
/// The crate's time vocabulary, re-exported from hifitime: integer-backed
/// (centuries + nanoseconds), exact arithmetic, TAI/TT/TDB/UTC built in.
pub use hifitime::{Duration, Epoch, TimeScale};
