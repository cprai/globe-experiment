//! Two-line element sets: parsing plus the element fields SGP4 consumers
//! need, over the `sgp4` crate. The propagation constants are built at
//! parse time, so element errors surface at load - strictly earlier than
//! the satkit era's propagate-time failures.

use chrono::{Datelike, Timelike};
use hifitime::Epoch;

/// A parsed element set with its SGP4 propagation constants. Immutable:
/// the satkit-era `&mut`-for-propagator-caching quirk is gone (note for
/// the eventual engine migration).
pub struct Tle {
    pub(crate) constants: sgp4::Constants,
    epoch: Epoch,
    name: Option<String>,
    mean_motion_rev_day: f64,
}

impl Tle {
    /// Parses a 3-line set (name line + two element lines).
    pub fn load_3line(line0: &str, line1: &str, line2: &str) -> Result<Self> {
        Self::build(
            sgp4::Elements::from_tle(
                Some(line0.trim().to_string()),
                line1.as_bytes(),
                line2.as_bytes(),
            )
            .map_err(err)?,
        )
    }

    /// Parses a 2-line set (no name line).
    pub fn load_2line(line1: &str, line2: &str) -> Result<Self> {
        Self::build(
            sgp4::Elements::from_tle(None, line1.as_bytes(), line2.as_bytes()).map_err(err)?,
        )
    }

    fn build(elements: sgp4::Elements) -> Result<Self> {
        let constants = sgp4::Constants::from_elements(&elements).map_err(err)?;
        // chrono exits the crate here: the TLE epoch (UTC by convention)
        // becomes a hifitime Epoch once, at parse.
        let datetime = elements.datetime;
        let epoch = Epoch::from_gregorian_utc(
            datetime.year(),
            datetime.month() as u8,
            datetime.day() as u8,
            datetime.hour() as u8,
            datetime.minute() as u8,
            datetime.second() as u8,
            datetime.nanosecond(),
        );
        Ok(Self {
            constants,
            epoch,
            name: elements.object_name,
            mean_motion_rev_day: elements.mean_motion,
        })
    }

    /// Object name from the name line; `None` for a 2-line set.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Element-set epoch.
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Mean motion, revolutions per day.
    pub fn mean_motion_rev_day(&self) -> f64 {
        self.mean_motion_rev_day
    }
}

/// TLE parse failure (malformed lines, checksum, or degenerate elements).
#[derive(Debug)]
pub struct TleError(String);

impl std::fmt::Display for TleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TLE parse failed: {}", self.0)
    }
}

impl std::error::Error for TleError {}

pub type Result<T> = core::result::Result<T, TleError>;

fn err<E: std::fmt::Display>(error: E) -> TleError {
    TleError(error.to_string())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The ISS element set, epoch 2024-001.5 - shared with the sgp4 tests.
    /// Checksum digits are real (the `sgp4` crate validates them; satkit
    /// never did, so the satkit-era fixture carried stale ones).
    pub(crate) const ISS_TLE: [&str; 3] = [
        "ISS (ZARYA)",
        "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9009",
        "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299350",
    ];

    #[test]
    fn parses_3line_elements() {
        let tle = Tle::load_3line(ISS_TLE[0], ISS_TLE[1], ISS_TLE[2]).expect("valid TLE");
        assert_eq!(tle.name(), Some("ISS (ZARYA)"));
        assert!(
            (tle.mean_motion_rev_day() - 15.489_153).abs() < 1e-5,
            "mean motion {}",
            tle.mean_motion_rev_day()
        );
        let (year, month, day, hour, ..) = tle.epoch().to_gregorian_utc();
        assert_eq!(
            (year, month, day, hour),
            (2024, 1, 1, 12),
            "epoch 2024-001.5"
        );
    }

    #[test]
    fn two_line_set_has_no_name() {
        let tle = Tle::load_2line(ISS_TLE[1], ISS_TLE[2]).expect("valid TLE");
        assert_eq!(tle.name(), None);
    }

    #[test]
    fn rejects_malformed_lines() {
        assert!(Tle::load_2line("garbage", "lines").is_err());
    }
}
