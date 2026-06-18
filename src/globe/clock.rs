//! Simulation clock driving the satellite's motion.
//!
//! Time starts at the TLE epoch and advances by the wall-clock delta between
//! redraws, scaled by an adjustable multiplier (1x real time .. 100x, set on
//! an exponential base-e slider in the UI). It can
//! be paused. While running it acts like the camera's inertia/zoom - an
//! "animating" source that keeps requesting frames; while paused it advances
//! nothing, so the app returns to the idle = zero-GPU state.

use std::time::Instant as WallClock;

use satkit::{Duration, Instant};

pub struct Clock {
    /// Simulation time zero - the TLE's epoch.
    epoch: Instant,
    /// Simulation seconds advanced past the epoch.
    elapsed_seconds: f64,
    /// Time scale: 1.0 = real time, up to 100.0 = 100x real time. The UI
    /// drives this on an exponential (base e) slider, but it is stored as the
    /// plain linear factor.
    pub multiplier: f32,
    /// When true, time is frozen.
    pub paused: bool,
    /// Wall-clock instant of the previous advance; `None` whenever the clock
    /// is not running, so resuming doesn't jump by the paused interval.
    last: Option<WallClock>,
}

impl Clock {
    /// Real time to 100x real time.
    pub const MIN_MULTIPLIER: f32 = 1.0;
    pub const MAX_MULTIPLIER: f32 = 100.0;

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

    /// The current simulation time.
    pub fn now(&self) -> Instant {
        self.epoch + Duration::from_seconds(self.elapsed_seconds)
    }

    /// Advances simulation time by the wall-clock delta since the previous
    /// call, scaled by the multiplier. Returns whether the clock is running
    /// (so the caller keeps requesting frames and refreshes the satellite).
    pub fn tick(&mut self) -> bool {
        let now = WallClock::now();
        if self.paused {
            // Forget the reference point so the next resume starts fresh.
            self.last = None;
            return false;
        }

        let dt = self
            .last
            .map_or(0.0, |last| now.duration_since(last).as_secs_f64());
        self.last = Some(now);
        self.elapsed_seconds += dt * self.multiplier as f64;
        true
    }

    /// The current simulation datetime formatted for display (UTC).
    pub fn datetime_label(&self) -> String {
        let (year, month, day, hour, minute, second) = self.now().as_datetime();
        format!(
            "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{:02} UTC",
            second as i32
        )
    }
}
