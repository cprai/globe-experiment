//! Orbit math for the solar-system renderer: the long-term home of all
//! astrodynamics and the eventual satkit replacement. Today, with satkit as
//! a hidden backend: ephemeris lookups over an embedded JPL DE440
//! ([`ephemeris`]), numerical orbit propagation ([`propagation`]), SGP4
//! over element sets ([`tle`] + [`sgp4`]), inertial/Earth-fixed frame
//! rotations ([`frametransform`]), osculating elements ([`kepler`]), and
//! geodetic conversion ([`itrfcoord`]). Standalone - not yet consumed by
//! `engine`.
//!
//! Call [`init`] before any query that reads the embedded data (ephemeris,
//! propagation, frame transforms). satkit's data stores are process-wide
//! set-once: this `init` must never share a process with the engine's
//! `init_satkit`.

mod data;
pub mod ephemeris;
pub mod frametransform;
pub mod itrfcoord;
pub mod kepler;
pub mod propagation;
pub mod sgp4;
pub mod tle;

pub use data::init;
/// Temporary time types, re-exported from satkit until crate-owned ones land.
pub use satkit::{Duration, Instant, TimeScale};
