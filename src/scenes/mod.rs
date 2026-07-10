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
//! Every scene talks to its clock through [`SceneClock`], never through the
//! `Clock` internals (whose fields are private, so the compiler enforces
//! this). The Time panel's Run-toggle/speed-slider callbacks cannot call the
//! `&mut self` API themselves - a method call would borrow the whole scene
//! and collide with the other panel closures' disjoint field captures - so
//! they set the scene's `request_toggle_run`/`request_multiplier` fields
//! (the selector/burn request-flag idiom) and `advance()` folds them into
//! the clock via [`SceneClock::apply_clock_requests`].

use crate::engine::scene::Clock;

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

/// The public clock API every scene goes through. A scene implements only
/// `clock_mut` (where the clock lives depends on the struct - a plain field
/// for the Rust scenes, a field of the pyclass Inner for the `*_py` scenes);
/// everything else is a default method calling down through it, so the API
/// surface lives in one place. The `*_py` scenes re-expose these to their
/// scripts as scene-pyclass properties.
pub trait SceneClock {
    /// The one per-scene hook: where the clock lives in the struct.
    fn clock_mut(&mut self) -> &mut Clock;

    /// Advance simulation time by the scaled wall-clock delta; returns
    /// whether the clock is running (= keep requesting frames).
    fn tick_clock(&mut self) -> bool {
        self.clock_mut().tick()
    }

    /// The current simulation time.
    fn clock_now(&mut self) -> satkit::Instant {
        self.clock_mut().now()
    }

    /// The current simulation datetime formatted for display (UTC).
    fn clock_datetime_label(&mut self) -> String {
        self.clock_mut().datetime_label()
    }

    fn clock_paused(&mut self) -> bool {
        self.clock_mut().paused()
    }

    fn set_clock_paused(&mut self, paused: bool) {
        self.clock_mut().set_paused(paused);
    }

    fn clock_multiplier(&mut self) -> f32 {
        self.clock_mut().multiplier()
    }

    fn set_clock_multiplier(&mut self, multiplier: f32) {
        self.clock_mut().set_multiplier(multiplier);
    }

    /// Fold the Time panel's clock requests (the request-flag pattern; see
    /// the module doc): toggle Run, then apply a requested speed. Called at
    /// the top of each scene's `advance()`, before the tick.
    fn apply_clock_requests(&mut self, toggle_run: bool, multiplier: Option<f32>) {
        let clock = self.clock_mut();
        if toggle_run {
            let paused = clock.paused();
            clock.set_paused(!paused);
        }
        if let Some(multiplier) = multiplier {
            clock.set_multiplier(multiplier);
        }
    }
}
