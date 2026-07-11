//! Simulation clock driving the satellite's motion.
//!
//! Time starts at the TLE epoch and advances by the wall-clock delta between
//! redraws, scaled by an adjustable multiplier (1x real time .. 100x, set on
//! an exponential base-e slider in the UI). It can be paused: frames keep
//! rendering (the app renders unconditionally), but they depict a frozen
//! instant - a paused tick advances nothing and yields dt = 0.
//!
//! [`Clock`] itself is plain data - a constructor and private fields, no
//! behavior. All the clock logic lives in [`SceneClock`]'s default methods,
//! which (being in this module) are the only code that can reach the fields:
//! a scene implements just `clock_mut` and gets the whole API, and nothing
//! outside this module can mutate the clock behind it.

use std::time::Instant as WallClock;

use pyo3::prelude::*;
use satkit::{Duration, Instant};

/// `pyclass` only for the `MIN_MULTIPLIER`/`MAX_MULTIPLIER` classattrs (a
/// script reads them for its speed-slider range): no `Clock` instance
/// crosses into Python - a `*_py` scene exposes the clock through its own
/// scene pyclass properties, a snapshot/request mirror of the wrapper-owned
/// clock behind the [`SceneClock`] trait API.
/// All fields are private: every consumer (Rust scene or script) goes
/// through that API, so no field can be mutated behind the clock's back.
#[pyclass(module = "globe")]
pub struct Clock {
    /// Simulation time zero - the TLE's epoch.
    epoch: Instant,
    /// Simulation seconds advanced past the epoch.
    elapsed_seconds: f64,
    /// Time scale: 1.0 = real time, up to 100.0 = 100x real time. The UI
    /// drives this on an exponential (base e) slider, but it is stored as the
    /// plain linear factor.
    multiplier: f32,
    /// When true, time is frozen.
    paused: bool,
    /// Wall-clock instant of the previous advance; `None` whenever the clock
    /// is not running, so resuming doesn't jump by the paused interval.
    last: Option<WallClock>,
}

#[pymethods]
impl Clock {
    /// Real time to 100x real time. `classattr` so a script can read
    /// `Clock.MIN_MULTIPLIER` for its speed-slider range; unchanged as Rust
    /// associated consts.
    #[classattr]
    pub const MIN_MULTIPLIER: f32 = 1.0;
    #[classattr]
    pub const MAX_MULTIPLIER: f32 = 100.0;
}

impl Clock {
    pub fn new(epoch: Instant) -> Self {
        Self {
            epoch,
            elapsed_seconds: 0.0,
            multiplier: Self::MIN_MULTIPLIER,
            // Per the owner's choice, the clock runs from launch.
            paused: false,
            last: None,
        }
    }
}

/// The public clock API every scene goes through. A scene implements only
/// `clock_mut` (a plain `clock` field in every scene struct - the `*_py`
/// scenes keep it on their wrapper, outside the pyclass, precisely so this
/// hook can hand out the `&mut Clock` a pyclass cell's borrow guard could
/// not); everything else is a default method working directly on the
/// [`Clock`] fields (private, so only this module's defaults can - the API
/// surface AND its logic live in one place). Implementing this is also what
/// grants a scene the `Scene` trait's provided `tick_scene`. The `*_py`
/// scenes mirror the API to their scripts as scene-pyclass snapshot/request
/// properties.
///
/// The Time panel's Run-toggle/speed-slider callbacks call the setters
/// directly: a panel callback receives the live scene as its `&mut S`
/// argument at fire time (see `ui::UIDrawablePanel` - it captures nothing,
/// so every panel callback coexists). The one rule: a callback must stay
/// idempotent under egui's discard-pass double fire, so the Run toggle
/// writes a paused value snapshotted at panel-build time
/// (`move |scene| scene.set_clock_paused(running)`) rather than flipping
/// live state it re-reads.
pub trait SceneClock {
    /// The one per-scene hook: where the clock lives in the struct.
    fn clock_mut(&mut self) -> &mut Clock;

    /// Advance simulation time by the wall-clock delta since the previous
    /// call, scaled by the multiplier. Paused, it advances nothing.
    fn tick_clock(&mut self) {
        let clock = self.clock_mut();
        let now = WallClock::now();
        if clock.paused {
            // Forget the reference point so the next resume starts fresh.
            clock.last = None;
            return;
        }

        let dt = clock
            .last
            .map_or(0.0, |last| now.duration_since(last).as_secs_f64());
        clock.last = Some(now);
        clock.elapsed_seconds += dt * clock.multiplier as f64;
    }

    /// The current simulation time.
    fn clock_now(&mut self) -> Instant {
        let clock = self.clock_mut();
        clock.epoch + Duration::from_seconds(clock.elapsed_seconds)
    }

    /// The current simulation datetime formatted for display (UTC).
    fn clock_datetime_label(&mut self) -> String {
        let (year, month, day, hour, minute, second) = self.clock_now().as_datetime();
        format!(
            "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{:02} UTC",
            second as i32
        )
    }

    fn clock_paused(&mut self) -> bool {
        self.clock_mut().paused
    }

    fn set_clock_paused(&mut self, paused: bool) {
        self.clock_mut().paused = paused;
    }

    fn clock_multiplier(&mut self) -> f32 {
        self.clock_mut().multiplier
    }

    fn set_clock_multiplier(&mut self, multiplier: f32) {
        self.clock_mut().multiplier = multiplier;
    }
}
