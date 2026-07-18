//! Orbit math for the solar-system renderer: the long-term home of all
//! astrodynamics and the eventual satkit replacement. Today it provides
//! ephemeris lookups over an embedded JPL DE440, with satkit as a hidden
//! backend. Standalone - not yet consumed by `engine`.
//!
//! Call [`init`] before any query. satkit's data stores are process-wide
//! set-once: this `init` must never share a process with the engine's
//! `init_satkit`.

mod data;
mod ephemeris;

pub use data::init;
pub use ephemeris::{
    Body, EphemerisError, Result, barycentric_pos, barycentric_state, geocentric_pos,
    geocentric_state,
};
/// Temporary time types, re-exported from satkit until crate-owned ones land.
pub use satkit::{Duration, Instant};
