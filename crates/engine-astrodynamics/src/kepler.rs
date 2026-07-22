//! Osculating Keplerian elements from an inertial state vector - a
//! single-pass closed form over Terra's GM, which [`AstroData`] resolves
//! once at load from the embedded planetary-constants kernel (the same mu
//! anise's `Orbit` accessors would fetch per call). The formulas mirror
//! anise's `evec`/`energy_km2_s2`/`sma_km`/`period` chain exactly, minus
//! its re-derivation of shared intermediates.
//!
//! [`AstroData`]: crate::data::AstroData

use glam::DVec3;

use crate::data::AstroData;

/// Osculating elements of an elliptic orbit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Kepler {
    pub semi_major_axis_m: f64,
    pub eccentricity: f64,
    pub period_s: f64,
}

impl Kepler {
    /// Elements from an inertial (GCRF) position (m) + velocity (m/s).
    /// Errs for a non-elliptic (e >= 1, escape) state, which has no
    /// period, and for a degenerate (zero-radius) state.
    pub fn from_pv(data: &AstroData, pos_m: DVec3, vel_m_s: DVec3) -> Result<Self> {
        let mu = data.earth_mu_km3_s2;
        // km like the kernel's mu; the boundary conversion mirrors the
        // anise-backed implementation this replaced.
        let r = pos_m / 1e3;
        let v = vel_m_s / 1e3;
        let rmag_km = r.length();
        if rmag_km <= f64::EPSILON {
            return Err(KeplerError("zero-radius state".to_string()));
        }

        // Eccentricity vector and vis-viva energy, anise's expressions
        // verbatim (v.length().powi(2), not length_squared, so the values
        // match the old accessor chain bit for bit).
        let vmag2 = v.length().powi(2);
        let evec = ((vmag2 - mu / rmag_km) * r - r.dot(v) * v) / mu;
        let eccentricity = evec.length();
        let energy = vmag2 / 2.0 - mu / rmag_km;
        let sma_km = -mu / (2.0 * energy);

        // The e >= 1 -> Err contract: hyperbolic/parabolic states have no
        // period, and the engine's orbit-shape readout relies on the Err
        // fallback (anise's period() would silently return zero here).
        if eccentricity >= 1.0 || sma_km <= 0.0 {
            return Err(KeplerError(format!(
                "non-elliptic state: e = {eccentricity:.6}, a = {sma_km:.1} km"
            )));
        }
        Ok(Self {
            semi_major_axis_m: sma_km * 1e3,
            eccentricity,
            period_s: std::f64::consts::TAU * (sma_km.powi(3) / mu).sqrt(),
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
        let kepler = Kepler::from_pv(crate::data::test_data(), pos, vel).expect("elliptic state");
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

    /// The closed form must reproduce the anise `Orbit` accessor chain it
    /// replaced: same mu (resolved once at load), same expressions. The
    /// period tolerance covers anise's hifitime `Duration` round-trip
    /// (ns quantization the closed form no longer performs).
    #[test]
    fn matches_anise_orbit_accessors() {
        use anise::astro::orbit::Orbit;
        use anise::constants::frames::EARTH_J2000;
        let data = crate::data::test_data();
        let frame = data.almanac.frame_info(EARTH_J2000).expect("Terra frame");
        let epoch = crate::Epoch::from_gregorian_utc(2000, 1, 1, 12, 0, 0, 0);
        for e_target in [0.05_f64, 0.4, 0.7] {
            let radius_m = 6_778_000.0_f64;
            let speed = (MU_M3_S2 * (1.0 + e_target) / radius_m).sqrt();
            let (pos, vel) = (DVec3::new(radius_m, 0.0, 0.0), DVec3::new(0.0, speed, 0.0));
            let kepler = Kepler::from_pv(data, pos, vel).expect("elliptic state");
            let orbit = Orbit::new(
                pos.x / 1e3,
                pos.y / 1e3,
                pos.z / 1e3,
                vel.x / 1e3,
                vel.y / 1e3,
                vel.z / 1e3,
                epoch,
                frame,
            );
            let ecc = orbit.ecc().expect("anise ecc");
            let sma_m = orbit.sma_km().expect("anise sma") * 1e3;
            let period_s = orbit.period().expect("anise period").to_seconds();
            assert!((kepler.eccentricity - ecc).abs() < 1e-12, "e vs anise");
            assert!(
                (kepler.semi_major_axis_m - sma_m).abs() / sma_m < 1e-12,
                "a vs anise"
            );
            assert!(
                (kepler.period_s - period_s).abs() / period_s < 1e-9,
                "T vs anise"
            );
        }
    }

    /// An escape state (e >= 1) has no elements - the readout/trail `None`
    /// fallback the engine relies on. anise alone would NOT err here (its
    /// `period()` returns zero for hyperbolic states); the crate's own gate
    /// must.
    #[test]
    fn escape_state_errs() {
        let (pos, mut vel) = circular_pv();
        vel *= 2.0; // well past escape velocity
        assert!(Kepler::from_pv(crate::data::test_data(), pos, vel).is_err());
    }
}
