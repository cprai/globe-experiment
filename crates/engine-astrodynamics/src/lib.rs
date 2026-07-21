//! Orbit math for the solar-system renderer: ephemeris lookups over
//! embedded anise/JPL DE440 kernels ([`ephemeris`]), inertial/Earth-fixed
//! frame rotations ([`frames`]), osculating elements ([`kepler`]), WGS84
//! geodetic conversion ([`geodetic`]), SGP4 over element sets ([`tle`] +
//! [`sgp4`]), and the crate's own deep-space numerical propagator
//! ([`propagation`]). Standalone - not yet consumed by `engine`, which
//! still runs on satkit until its migration.
//!
//! No process-global state: queries parse the embedded kernels lazily on
//! first touch, and [`init`] merely front-loads that (keeping the cost out
//! of a first frame). Call it or don't - repeat calls are free.

mod data;
pub mod ephemeris;
pub mod frames;
pub mod geodetic;
pub mod kepler;
pub mod propagation;
pub mod sgp4;
pub mod tle;

pub use data::init;
/// The crate's time vocabulary, re-exported from hifitime: integer-backed
/// (centuries + nanoseconds), exact arithmetic, TAI/TT/TDB/UTC built in.
pub use hifitime::{Duration, Epoch, TimeScale};
