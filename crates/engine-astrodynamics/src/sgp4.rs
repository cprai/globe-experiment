//! SGP4 propagation of element sets over the `sgp4` crate. Output is the
//! TEME frame (SGP4's native quasi-inertial frame), meters and m/s.

use glam::DVec3;
use hifitime::{Epoch, Unit};

use crate::tle::Tle;

/// SGP4's own decay floor: the WGS72 equatorial radius. The reference
/// implementation flags samples below one Earth radius; the `sgp4` crate
/// returns them without complaint, so the check is reinstated here - this
/// module's contract is "never a silently garbage state".
const EARTH_RADIUS_M: f64 = 6_378_135.0;

/// A TEME state vector from SGP4: position meters, velocity m/s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TemeState {
    pub pos_teme_m: DVec3,
    pub vel_teme_m_s: DVec3,
}

/// SGP4 failure (degenerate elements, or a decayed/escaped sample).
#[derive(Debug)]
pub struct Sgp4Error(String);

impl std::fmt::Display for Sgp4Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sgp4 propagation failed: {}", self.0)
    }
}

impl std::error::Error for Sgp4Error {}

pub type Result<T> = core::result::Result<T, Sgp4Error>;

/// Propagates `tle` to each epoch, one TEME state per input time. Stricter
/// than the backing crate: a model error or a sub-surface (decayed) sample
/// becomes an `Err`, never a silently garbage state.
pub fn sgp4(tle: &Tle, epochs: &[Epoch]) -> Result<Vec<TemeState>> {
    epochs
        .iter()
        .map(|&epoch| {
            let minutes = (epoch - tle.epoch()).to_unit(Unit::Minute);
            let prediction = tle
                .constants
                .propagate(sgp4::MinutesSinceEpoch(minutes))
                .map_err(|error| Sgp4Error(error.to_string()))?;
            let state = TemeState {
                pos_teme_m: DVec3::from_array(prediction.position) * 1e3,
                vel_teme_m_s: DVec3::from_array(prediction.velocity) * 1e3,
            };
            if state.pos_teme_m.length() < EARTH_RADIUS_M {
                return Err(Sgp4Error(format!(
                    "decayed: radius {:.1} km is below one Earth radius",
                    state.pos_teme_m.length() / 1e3
                )));
            }
            Ok(state)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Duration;
    use crate::tle::tests::ISS_TLE;

    fn iss() -> Tle {
        Tle::load_3line(ISS_TLE[0], ISS_TLE[1], ISS_TLE[2]).expect("valid TLE")
    }

    /// One sample at the element-set epoch must land in the ISS's LEO
    /// envelope (radius and inertial speed).
    #[test]
    fn epoch_sample_is_leo() {
        let tle = iss();
        let epoch = tle.epoch();
        let states = sgp4(&tle, &[epoch]).expect("sgp4 at epoch");
        assert_eq!(states.len(), 1);
        let radius = states[0].pos_teme_m.length();
        let speed = states[0].vel_teme_m_s.length();
        assert!(
            (6.6e6..7.0e6).contains(&radius),
            "ISS radius {:.1} km",
            radius / 1000.0
        );
        assert!(
            (7_500.0..7_800.0).contains(&speed),
            "ISS speed {speed:.1} m/s"
        );
    }

    /// Batch propagation: one state per input instant, all on the orbit,
    /// moving along track between samples.
    #[test]
    fn batch_matches_input_times() {
        let tle = iss();
        let t0 = tle.epoch();
        let times: Vec<Epoch> = (0..10)
            .map(|i| t0 + Duration::from_seconds(60.0 * f64::from(i)))
            .collect();
        let states = sgp4(&tle, &times).expect("batch sgp4");
        assert_eq!(states.len(), times.len());
        for state in &states {
            assert!((6.6e6..7.0e6).contains(&state.pos_teme_m.length()));
        }
        assert!(
            (states[9].pos_teme_m - states[0].pos_teme_m).length() > 1_000_000.0,
            "nine minutes must move well along track"
        );
    }

    /// Appends the TLE checksum digit (digit sum, '-' counts 1, mod 10).
    fn checksummed(line: &str) -> String {
        let sum: u32 = line
            .chars()
            .map(|c| match c {
                '0'..='9' => c as u32 - '0' as u32,
                '-' => 1,
                _ => 0,
            })
            .sum();
        format!("{line}{}", sum % 10)
    }

    /// A sub-surface sample must be an `Err`, not a garbage state: same
    /// ISS elements reshaped to e = 0.05 at 16.9 rev/day, whose perigee
    /// (~6100 km radius) sits below one Earth radius from the start.
    #[test]
    fn sub_surface_sample_errs_as_decayed() {
        let line2 =
            checksummed("2 25544  51.6432 351.4697 0500000 130.5364 329.6482 16.9000000029935");
        let tle = Tle::load_2line(ISS_TLE[1], &line2).expect("valid decayed-orbit TLE");
        let error = sgp4(&tle, &[tle.epoch()]).expect_err("sub-surface sample");
        assert!(
            error.to_string().contains("decayed"),
            "unexpected error: {error}"
        );
    }
}
