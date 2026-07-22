//! The cross-implementation comparison tests: one module file per
//! domain (the same split as the bench targets in `src/benches/`), and
//! within each file one nested module per reference implementation
//! (`satkit`, `astrodyn` — more comparison crates land as siblings).

mod ephemeris;
mod frames;
mod geodetic;
mod kepler;
mod propagation;
mod sgp4;
