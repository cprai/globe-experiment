//! Scenes: each entry point pins the simulation to a specific **past** event
//! and runs the app for it. Most pin a satellite/TLE set plus the time window
//! its clock starts in; the eclipse scenes are **empty** (no tracked
//! objects) and just wind the celestial sphere to the event, framing it on
//! launch. `main` does nothing but parse the CLI and dispatch to one of these;
//! all the per-scene assembly (which tracked objects, in which order, so the
//! clock starts at the right epoch) lives here, not in `main`.
//!
//! Add a scene by adding a module here (with its own `clap::Args` struct -
//! each scene subcommand declares exactly its own arguments) and wiring a
//! `SceneCommand` variant to its `run` in `main.rs`. Keep each scene's time
//! window inside the bundled EOP range
//! (1962-01-01 .. build date) so the astronomical-accuracy goal holds - see the
//! "Scenes & valid time range" rules in `CLAUDE.md`.
//!
//! Every scene talks to its clock through the `SceneClock` trait
//! (`engine::scene::clock`, next to `Clock` itself - whose fields are
//! private, so the compiler enforces this): implement `clock_mut` and the
//! trait's default methods are the whole clock API. See the trait doc for
//! the Time panel's request-flag idiom
//! (`request_toggle_run`/`request_multiplier` folded in `advance()` via
//! `SceneClock::apply_clock_requests`).

pub mod iss;
pub mod iss_and_hubble;
pub mod lunar_eclipse;
pub mod manual_control;
// The `*_py` scenes are clones of their Rust siblings whose UI panels are
// produced by a Python script under the repo-root `scenes/` directory (via
// the embedded interpreter, `engine::py`) - kept side by side so the Rust
// and Python panel APIs can be compared.
pub mod manual_control_py;
pub mod solar_eclipse;
pub mod solar_system;
pub mod solar_system_py;
