//! Scenarios: each entry point pins the simulation to a specific **past** event
//! (a satellite/TLE set + the time window its clock starts in) and runs the app
//! for it. `main` does nothing but parse the CLI and dispatch to one of these;
//! all the per-scenario assembly (which tracked objects, in which order, so the
//! clock starts at the right epoch) lives here, not in `main`.
//!
//! Add a scenario by adding a module here and wiring a CLI value to its `run`
//! in `main.rs`. Keep each scenario's time window inside the bundled EOP range
//! (1962-01-01 .. build date) so the astronomical-accuracy goal holds - see the
//! "Scenarios & valid time range" rules in `CLAUDE.md`.

pub mod iss_and_hubble;
