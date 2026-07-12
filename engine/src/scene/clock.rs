//! Simulation clock: wall-clock dt x multiplier, play/pause. Paused frames
//! keep rendering but depict a frozen instant (a paused tick yields dt = 0).
//!
//! [`Clock`] is plain data; all logic lives in [`SceneClock`]'s default
//! methods, which (same module) are the only code that can reach the private
//! fields - nothing can mutate the clock behind the API.

use std::time::Instant as WallClock;

use satkit::{Duration, Instant};

/// All fields private; every consumer goes through [`SceneClock`]. The
/// clock imposes no multiplier range - each scene owns its own min/max
/// constants for its speed slider.
pub struct Clock {
    /// Simulation time zero - the TLE's epoch.
    epoch: Instant,
    /// Simulation seconds advanced past the epoch.
    elapsed_seconds: f64,
    /// Linear time-scale factor (the UI slider is exponential base e, but
    /// the stored value is plain linear).
    multiplier: f32,
    /// When true, time is frozen.
    paused: bool,
    /// Wall-clock instant of the previous advance; `None` whenever the clock
    /// is not running, so resuming doesn't jump by the paused interval.
    last: Option<WallClock>,
}

impl Clock {
    pub fn new(epoch: Instant) -> Self {
        Self {
            epoch,
            elapsed_seconds: 0.0,
            // Real time; a scene wanting a different start speed sets it
            // through `SceneClock`.
            multiplier: 1.0,
            // Per the owner's choice, the clock runs from launch.
            paused: false,
            last: None,
        }
    }
}

/// The clock API every scene goes through. `#[derive(SceneClock)]` (the
/// `macros` crate, re-exported next to this trait) supplies the only
/// required method, `clock_mut`, from a field named `clock` (a
/// plain field; the `*_py` scenes keep it on the wrapper, outside the
/// pyclass, precisely so this hook can hand out the `&mut Clock` a pyclass
/// cell's borrow guard could not); the default methods hold all the logic
/// and are the only code that can touch [`Clock`]'s fields. Implementing
/// this also grants the `Scene` trait's provided `tick_scene`.
///
/// The Time panel's callbacks call the setters directly with build-time
/// snapshots (`move |scene| scene.set_clock_paused(running)`), never
/// read-modify-write: egui's discard pass can fire a callback twice per
/// frame, so callbacks must stay idempotent.
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
