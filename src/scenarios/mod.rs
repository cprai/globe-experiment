//! Scenarios: each entry point pins the simulation to a specific **past** event
//! and runs the app for it. Most pin a satellite/TLE set plus the time window
//! its clock starts in; the eclipse scenarios are **empty** (no tracked
//! objects) and just wind the celestial sphere to the event, framing it on
//! launch. `main` does nothing but parse the CLI and dispatch to one of these;
//! all the per-scenario assembly (which tracked objects, in which order, so the
//! clock starts at the right epoch) lives here, not in `main`.
//!
//! Add a scenario by adding a module here and wiring a CLI value to its `run`
//! in `main.rs`. Keep each scenario's time window inside the bundled EOP range
//! (1962-01-01 .. build date) so the astronomical-accuracy goal holds - see the
//! "Scenarios & valid time range" rules in `CLAUDE.md`.

pub mod iss;
pub mod iss_and_hubble;
pub mod lunar_eclipse;
pub mod solar_eclipse;
pub mod solar_system;
