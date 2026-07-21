//! Third-body point-mass gravity (spec §4.2), in Battin's
//! difference-of-cubes form: when the spacecraft is much closer to the
//! central body than to the perturber, the direct and indirect terms
//! nearly cancel and the naive evaluation loses most of its digits; the
//! F(q) formulation is exact and cancellation-free.

use glam::DVec3;

use super::{EvalContext, ForceModel};
use crate::ephemeris::Body;

/// A perturbing body about the current (Earth) central body. Positions
/// come from the crate's own ephemeris per evaluation; mu from the
/// planetary-constants kernel at config-build time. (Generalizing the
/// observer beyond Earth arrives with the central-body switch, P7.)
pub(crate) struct ThirdBodyGravity {
    pub body: Body,
    pub mu_m3_s2: f64,
}

impl ForceModel for ThirdBodyGravity {
    fn acceleration_can(
        &self,
        ctx: &EvalContext,
        r_can: DVec3,
        _v_can: DVec3,
    ) -> Result<DVec3, String> {
        let r3_m = ctx
            .geocentric_pos_m(self.body)
            .map_err(|error| format!("third body {:?}: {error}", self.body))?;
        let r3_can = ctx.units.length_to_can(r3_m);
        let mu3_can = self.mu_m3_s2 / ctx.units.mu_m3_s2;
        Ok(battin_acceleration(mu3_can, r_can, r3_can))
    }
}

/// `a = -mu3 / |r - r3|^3 * (r + F(q) r3)` with
/// `q = r . (r - 2 r3) / |r3|^2` and
/// `F(q) = q (3 + 3q + q^2) / (1 + (1+q)^{3/2})` - algebraically identical
/// to the direct-minus-indirect form (`F(q) = (1+q)^{3/2} - 1` exactly),
/// but with the cancellation eliminated (Battin; spec Appendix A).
pub(crate) fn battin_acceleration(mu3: f64, r: DVec3, r3: DVec3) -> DVec3 {
    let q = r.dot(r - 2.0 * r3) / r3.length_squared();
    let f = q * (3.0 + (3.0 + q) * q) / (1.0 + (1.0 + q).powf(1.5));
    let d = r - r3;
    -mu3 / d.length().powi(3) * (r + f * r3)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The naive direct-minus-indirect evaluation, for comparison only.
    fn naive(mu3: f64, r: DVec3, r3: DVec3) -> DVec3 {
        let d = r3 - r;
        mu3 * (d / d.length().powi(3) - r3 / r3.length().powi(3))
    }

    /// At a moderate separation ratio there is no cancellation - both
    /// forms are healthy and must agree to near machine precision.
    #[test]
    fn matches_naive_at_moderate_ratio() {
        let r = DVec3::new(0.07, 0.05, -0.03);
        let r3 = DVec3::new(0.8, -0.4, 0.2);
        let battin = battin_acceleration(1e-2, r, r3);
        let naive = naive(1e-2, r, r3);
        assert!(
            (battin - naive).length() < 1e-12 * naive.length(),
            "battin {battin:?} vs naive {naive:?}"
        );
    }

    /// Deep in the cancellation regime (LEO vs the Sun: ratio ~ 4e-5) the
    /// perturbation must match the analytic tidal limit
    /// `a ~ mu3/r3^3 (3 (r . u) u - r)` to first order.
    #[test]
    fn matches_tidal_limit_at_small_ratio() {
        let mu3 = 333_000.0; // Sun in Earth-mu units
        let r3 = DVec3::new(23_000.0, 4_000.0, -1_500.0); // ~1 AU in DU
        let r = DVec3::new(0.71, -0.42, 0.31); // ~LEO in DU
        let battin = battin_acceleration(mu3, r, r3);
        let u = r3.normalize();
        let tidal = mu3 / r3.length().powi(3) * (3.0 * r.dot(u) * u - r);
        assert!(
            (battin - tidal).length() < 1e-3 * tidal.length(),
            "battin {battin:?} vs tidal {tidal:?}"
        );
    }
}
