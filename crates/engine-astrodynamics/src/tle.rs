//! Two-line element sets: parsing plus the element fields SGP4 consumers
//! need, delegating to satkit's `tle`.

use satkit::Instant;
use satkit::tle::TLE;

/// A parsed element set. Taken `&mut` by [`crate::sgp4::sgp4`] because the
/// initialized propagator is cached inside it between calls.
pub struct Tle {
    pub(crate) inner: TLE,
}

impl Tle {
    /// Parses a 3-line set (name line + two element lines).
    pub fn load_3line(line0: &str, line1: &str, line2: &str) -> Result<Self> {
        TLE::load_3line(line0, line1, line2)
            .map(|inner| Self { inner })
            .map_err(|error| TleError(error.to_string()))
    }

    /// Parses a 2-line set (no name line).
    pub fn load_2line(line1: &str, line2: &str) -> Result<Self> {
        TLE::load_2line(line1, line2)
            .map(|inner| Self { inner })
            .map_err(|error| TleError(error.to_string()))
    }

    /// Object name from the name line (e.g. "ISS (ZARYA)").
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// Element-set epoch.
    pub fn epoch(&self) -> Instant {
        self.inner.epoch
    }

    /// Mean motion, revolutions per day.
    pub fn mean_motion_rev_day(&self) -> f64 {
        self.inner.mean_motion
    }
}

/// TLE parse failure (malformed lines or checksum).
#[derive(Debug)]
pub struct TleError(String);

impl std::fmt::Display for TleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TLE parse failed: {}", self.0)
    }
}

impl std::error::Error for TleError {}

pub type Result<T> = core::result::Result<T, TleError>;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The ISS element set, epoch 2024-001.5 - shared with the sgp4 tests.
    pub(crate) const ISS_TLE: [&str; 3] = [
        "ISS (ZARYA)",
        "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9003",
        "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299357",
    ];

    #[test]
    fn parses_3line_elements() {
        let tle = Tle::load_3line(ISS_TLE[0], ISS_TLE[1], ISS_TLE[2]).expect("valid TLE");
        assert_eq!(tle.name(), "ISS (ZARYA)");
        assert!(
            (tle.mean_motion_rev_day() - 15.489_153).abs() < 1e-5,
            "mean motion {}",
            tle.mean_motion_rev_day()
        );
        let (year, month, day, hour, _, _) = tle.epoch().as_datetime();
        assert_eq!(
            (year, month, day, hour),
            (2024, 1, 1, 12),
            "epoch 2024-001.5"
        );
    }

    #[test]
    fn rejects_malformed_lines() {
        assert!(Tle::load_2line("garbage", "lines").is_err());
    }
}
