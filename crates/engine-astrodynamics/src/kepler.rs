//! Osculating Keplerian elements from an inertial state vector, delegating
//! to satkit.

use glam::DVec3;
use satkit::Vector3;

/// Osculating elements of an elliptic orbit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Kepler {
    pub semi_major_axis_m: f64,
    pub eccentricity: f64,
    pub period_s: f64,
}

impl Kepler {
    /// Elements from an inertial (GCRF) position (m) + velocity (m/s).
    /// Errs for a non-elliptic (e >= 1, escape) state, which has no period.
    pub fn from_pv(pos_m: DVec3, vel_m_s: DVec3) -> Result<Self> {
        let kepler = satkit::Kepler::from_pv(vec3(pos_m), vec3(vel_m_s))
            .map_err(|error| KeplerError(error.to_string()))?;
        Ok(Self {
            semi_major_axis_m: kepler.a,
            eccentricity: kepler.eccen,
            period_s: kepler.period(),
        })
    }
}

/// Element extraction failure (a non-elliptic or degenerate state).
#[derive(Debug)]
pub struct KeplerError(String);

impl std::fmt::Display for KeplerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Keplerian elements failed: {}", self.0)
    }
}

impl std::error::Error for KeplerError {}

pub type Result<T> = core::result::Result<T, KeplerError>;

fn vec3(v: DVec3) -> Vector3 {
    Vector3::new([[v.x], [v.y], [v.z]])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Terra's GM (m^3/s^2) - only to construct the test states.
    const MU_M3_S2: f64 = 3.986004418e14;
    const RADIUS_M: f64 = 6_778_000.0;

    fn circular_pv() -> (DVec3, DVec3) {
        let speed = (MU_M3_S2 / RADIUS_M).sqrt();
        (DVec3::new(RADIUS_M, 0.0, 0.0), DVec3::new(0.0, speed, 0.0))
    }

    /// A circular state's elements: `a` = the radius, near-zero
    /// eccentricity, and the two-body period.
    #[test]
    fn circular_orbit_elements() {
        let (pos, vel) = circular_pv();
        let kepler = Kepler::from_pv(pos, vel).expect("elliptic state");
        assert!(
            (kepler.semi_major_axis_m - RADIUS_M).abs() < 10_000.0,
            "a = {:.1} km",
            kepler.semi_major_axis_m / 1000.0
        );
        assert!(kepler.eccentricity < 0.01, "e = {}", kepler.eccentricity);
        let two_body_period = 2.0 * std::f64::consts::PI * (RADIUS_M.powi(3) / MU_M3_S2).sqrt();
        assert!(
            (kepler.period_s - two_body_period).abs() / two_body_period < 0.01,
            "period {:.1} s vs {two_body_period:.1} s",
            kepler.period_s
        );
    }

    /// An escape state (e >= 1) has no elements - the readout/trail `None`
    /// fallback the engine relies on.
    #[test]
    fn escape_state_errs() {
        let (pos, mut vel) = circular_pv();
        vel *= 2.0; // well past escape velocity
        assert!(Kepler::from_pv(pos, vel).is_err());
    }
}
