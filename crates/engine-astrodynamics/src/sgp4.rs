//! SGP4 propagation of element sets, delegating to satkit. Output is the
//! TEME frame (SGP4's native quasi-inertial frame), meters and m/s.

use glam::DVec3;
use satkit::Instant;
use satkit::sgp4::SGP4Error;

use crate::tle::Tle;

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

/// Propagates `tle` to each instant, one TEME state per input time (`&mut`:
/// the initialized propagator is cached in the TLE). Stricter than satkit:
/// a per-sample SGP4 error code becomes an `Err`, never a silently garbage
/// state.
pub fn sgp4(tle: &mut Tle, times: &[Instant]) -> Result<Vec<TemeState>> {
    let state =
        satkit::sgp4::sgp4(&mut tle.inner, times).map_err(|error| Sgp4Error(error.to_string()))?;

    if let Some(code) = state
        .errcode
        .iter()
        .find(|code| !matches!(code, SGP4Error::SGP4Success))
    {
        return Err(Sgp4Error(code.to_string()));
    }

    Ok((0..times.len())
        .map(|i| TemeState {
            pos_teme_m: DVec3::new(state.pos[(0, i)], state.pos[(1, i)], state.pos[(2, i)]),
            vel_teme_m_s: DVec3::new(state.vel[(0, i)], state.vel[(1, i)], state.vel[(2, i)]),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tle::tests::ISS_TLE;
    use satkit::Duration;

    fn iss() -> Tle {
        Tle::load_3line(ISS_TLE[0], ISS_TLE[1], ISS_TLE[2]).expect("valid TLE")
    }

    /// One sample at the element-set epoch must land in the ISS's LEO
    /// envelope (radius and inertial speed).
    #[test]
    fn epoch_sample_is_leo() {
        let mut tle = iss();
        let epoch = tle.epoch();
        let states = sgp4(&mut tle, &[epoch]).expect("sgp4 at epoch");
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
        let mut tle = iss();
        let t0 = tle.epoch();
        let times: Vec<Instant> = (0..10)
            .map(|i| t0 + Duration::from_seconds(60.0 * f64::from(i)))
            .collect();
        let states = sgp4(&mut tle, &times).expect("batch sgp4");
        assert_eq!(states.len(), times.len());
        for state in &states {
            assert!((6.6e6..7.0e6).contains(&state.pos_teme_m.length()));
        }
        assert!(
            (states[9].pos_teme_m - states[0].pos_teme_m).length() > 1_000_000.0,
            "nine minutes must move well along track"
        );
    }
}
