//! Canonical (non-dimensional) units for the propagator (spec §1): a
//! distance unit DU and the central body's mu define TU = sqrt(DU^3 / mu),
//! putting positions, velocities, and accelerations near order 1 and making
//! the central point-mass term exactly -r/|r|^3.
//!
//! Unit discipline is by naming convention (the spec's sanctioned
//! alternative to newtypes): every canonical quantity carries a `_can`
//! suffix, every SI quantity a `_m` / `_m_s` / `_m_s2` / `_s` suffix.
//! Derive the scale factors from the SAME mu the dynamics use - a mismatch
//! silently breaks the mu = 1 assumption inside the integrator.

use glam::DVec3;

#[derive(Clone, Copy, Debug)]
pub(crate) struct CanonicalUnits {
    pub du_m: f64,
    pub tu_s: f64,
    pub mu_m3_s2: f64,
}

impl CanonicalUnits {
    pub(crate) fn new(mu_m3_s2: f64, du_m: f64) -> Self {
        Self {
            du_m,
            tu_s: (du_m.powi(3) / mu_m3_s2).sqrt(),
            mu_m3_s2,
        }
    }

    /// The canonical velocity unit, m/s.
    pub(crate) fn vu_m_s(&self) -> f64 {
        self.du_m / self.tu_s
    }

    /// The canonical acceleration unit, m/s^2.
    pub(crate) fn acu_m_s2(&self) -> f64 {
        self.du_m / (self.tu_s * self.tu_s)
    }

    pub(crate) fn length_to_can(&self, meters: DVec3) -> DVec3 {
        meters / self.du_m
    }

    pub(crate) fn length_to_m(&self, can: DVec3) -> DVec3 {
        can * self.du_m
    }

    pub(crate) fn velocity_to_can(&self, m_s: DVec3) -> DVec3 {
        m_s / self.vu_m_s()
    }

    pub(crate) fn velocity_to_m_s(&self, can: DVec3) -> DVec3 {
        can * self.vu_m_s()
    }

    pub(crate) fn accel_to_can(&self, m_s2: DVec3) -> DVec3 {
        m_s2 / self.acu_m_s2()
    }

    pub(crate) fn time_to_can(&self, seconds: f64) -> f64 {
        seconds / self.tu_s
    }

    pub(crate) fn time_to_s(&self, can: f64) -> f64 {
        can * self.tu_s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec §1's illustrative sets, re-derived from their own constants:
    /// heliocentric TU ~ 58.1324 days, geocentric TU ~ 806.81 s.
    #[test]
    fn spec_reference_scales() {
        let heliocentric = CanonicalUnits::new(1.327_124_400_18e20, 1.495_978_707e11);
        assert!(
            (heliocentric.tu_s / 86_400.0 - 58.1324).abs() < 5e-4,
            "heliocentric TU = {} days",
            heliocentric.tu_s / 86_400.0
        );
        let geocentric = CanonicalUnits::new(3.986_004_418e14, 6_378_137.0);
        assert!(
            (geocentric.tu_s - 806.81).abs() < 0.01,
            "geocentric TU = {} s",
            geocentric.tu_s
        );
        assert!((geocentric.vu_m_s() - 7_905.4).abs() < 0.1);
    }

    /// Spec §7.12: SI -> canonical -> SI round trips for every quantity.
    #[test]
    fn round_trips() {
        let units = CanonicalUnits::new(3.986_004_418e14, 6_378_137.0);
        let v = DVec3::new(7.3e6, -1.2e5, 4.4e6);
        assert!((units.length_to_m(units.length_to_can(v)) - v).length() < 1e-6 * v.length());
        let vel = DVec3::new(-7.1e3, 2.0e2, 3.3e3);
        assert!(
            (units.velocity_to_m_s(units.velocity_to_can(vel)) - vel).length()
                < 1e-12 * vel.length()
        );
        let acc = DVec3::new(9.1, -0.3, 0.02);
        let acc_rt = units.accel_to_can(acc) * units.acu_m_s2();
        assert!((acc_rt - acc).length() < 1e-12 * acc.length());
        let t = 86_400.0 * 3.7;
        assert!((units.time_to_s(units.time_to_can(t)) - t).abs() < 1e-9 * t);
    }

    /// A circular orbit at 1 DU has speed 1 and period 2 pi by construction.
    #[test]
    fn circular_orbit_at_one_du_is_unit_speed() {
        let units = CanonicalUnits::new(3.986_004_418e14, 6_378_137.0);
        let circular_speed_m_s = (units.mu_m3_s2 / units.du_m).sqrt();
        let speed_can = units
            .velocity_to_can(DVec3::new(0.0, circular_speed_m_s, 0.0))
            .length();
        assert!((speed_can - 1.0).abs() < 1e-12, "speed {speed_can}");
    }
}
