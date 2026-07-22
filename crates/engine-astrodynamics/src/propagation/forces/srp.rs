//! Direct solar radiation pressure (spec §4.4):
//! `a = nu * P_sun * (AU/d)^2 * C_r * (A/m) * r_hat_sun`, with `r_hat_sun`
//! pointing FROM the Sun TO the spacecraft (SRP pushes away from the Sun -
//! the sign convention of spec §4.3, stated once and enforced here), and
//! `nu` the conical-shadow illumination fraction of §4.5.

use glam::DVec3;

use super::shadow::illumination;
use super::{EvalContext, ForceModel};
use crate::ephemeris::Body;
use crate::propagation::spacecraft::SpacecraftModel;

/// Solar radiation pressure at 1 AU: 1361 W/m^2 total solar irradiance
/// over c. (The legacy 4.56e-6 pairs with the older 1367 W/m^2 constant -
/// never mix the pairs; spec §4.4.)
pub(crate) const SOLAR_PRESSURE_N_M2: f64 = 4.5398e-6;
/// The reference distance `AU_ref` for [`SOLAR_PRESSURE_N_M2`] - unrelated
/// to the canonical acceleration unit.
pub(crate) const AU_M: f64 = 1.495_978_707e11;
/// IAU nominal solar radius, for the apparent solar disk.
pub(crate) const SUN_RADIUS_M: f64 = 6.957e8;

/// One body that can block the Sun (spec §4.5: per-segment configuration;
/// the central body by default, plus Luna for Earth orbiters).
#[derive(Clone)]
pub(crate) enum Occulter {
    /// The segment's central body (position = minus the spacecraft vector).
    Central { radius_m: f64 },
    /// Luna, positioned via the ephemeris per evaluation.
    Luna { radius_m: f64 },
}

#[derive(Clone)]
pub(crate) struct SolarRadiationPressure {
    pub spacecraft: SpacecraftModel,
    pub occulters: Vec<Occulter>,
}

impl SolarRadiationPressure {
    fn to_occulter_m(
        &self,
        occulter: &Occulter,
        ctx: &EvalContext,
        r_m: DVec3,
    ) -> Result<(DVec3, f64), String> {
        Ok(match occulter {
            Occulter::Central { radius_m } => (-r_m, *radius_m),
            Occulter::Luna { radius_m } => (ctx.geocentric_pos_m(Body::Luna)? - r_m, *radius_m),
        })
    }

    /// The illumination fraction `nu = prod nu_i` at the given state.
    pub(crate) fn illumination_at(&self, ctx: &EvalContext, r_can: DVec3) -> Result<f64, String> {
        let r_m = ctx.units.length_to_m(r_can);
        let to_sun = ctx.geocentric_pos_m(Body::Sol)? - r_m;
        let mut nu = 1.0;
        for occulter in &self.occulters {
            let (to_occ, radius) = self.to_occulter_m(occulter, ctx, r_m)?;
            nu *= illumination(to_sun, to_occ, SUN_RADIUS_M, radius);
            if nu == 0.0 {
                break;
            }
        }
        Ok(nu)
    }

    /// Signed shadow-boundary functions for event detection (spec §5),
    /// returned as `(outer, inner)`: per occulter,
    /// `g1 = separation - (a_sun + a_occ)` crosses zero at the penumbra's
    /// OUTER edge and `g2 = separation - |a_occ - a_sun|` at its INNER
    /// edge (umbra or annular onset); each family combines across
    /// occulters with `min`, so the sign flips whenever ANY occulter's
    /// edge is crossed. Two separate functions are load-bearing: a single
    /// combined form (e.g. the product) has equal signs in full sun and
    /// umbra, and an integrator step that swallows the whole ~10 s LEO
    /// penumbra transit would see no sign change at all.
    pub(crate) fn boundary_functions(
        &self,
        ctx: &EvalContext,
        r_can: DVec3,
    ) -> Result<(f64, f64), String> {
        let r_m = ctx.units.length_to_m(r_can);
        let to_sun = ctx.geocentric_pos_m(Body::Sol)? - r_m;
        let d_sun = to_sun.length();
        let sun_apparent = (SUN_RADIUS_M / d_sun).min(1.0).asin();
        let (mut outer, mut inner) = (f64::INFINITY, f64::INFINITY);
        for occulter in &self.occulters {
            let (to_occ, radius) = self.to_occulter_m(occulter, ctx, r_m)?;
            let d_occ = to_occ.length();
            if d_occ >= d_sun {
                continue; // cannot occult; contributes no boundary
            }
            let occ_apparent = (radius / d_occ.max(radius)).min(1.0).asin();
            let separation = to_sun.angle_between(to_occ);
            outer = outer.min(separation - (sun_apparent + occ_apparent));
            inner = inner.min(separation - (occ_apparent - sun_apparent).abs());
        }
        if outer == f64::INFINITY {
            return Ok((1.0, 1.0)); // nothing can occult here
        }
        Ok((outer, inner))
    }
}

impl ForceModel for SolarRadiationPressure {
    fn acceleration_can(
        &self,
        ctx: &EvalContext,
        r_can: DVec3,
        _v_can: DVec3,
    ) -> Result<DVec3, String> {
        let nu = self.illumination_at(ctx, r_can)?;
        if nu == 0.0 {
            return Ok(DVec3::ZERO); // umbra: skip, don't multiply by zero
        }
        let r_m = ctx.units.length_to_m(r_can);
        let from_sun = r_m - ctx.geocentric_pos_m(Body::Sol)?;
        let distance = from_sun.length();
        let direction = from_sun / distance;
        let pressure = SOLAR_PRESSURE_N_M2 * (AU_M / distance).powi(2);
        let a_m_s2 = nu * pressure * self.spacecraft.c_r * self.spacecraft.area_m2(direction)
            / self.spacecraft.mass_kg
            * direction;
        Ok(ctx.units.accel_to_can(a_m_s2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::propagation::units::CanonicalUnits;
    use hifitime::Epoch;

    fn context() -> EvalContext<'static> {
        EvalContext::new(
            crate::data::test_data(),
            CanonicalUnits::new(3.986_004_418e14, 6_378_137.0),
            Epoch::from_gregorian_utc(2024, 1, 15, 12, 0, 0, 0),
        )
    }

    fn spacecraft() -> SpacecraftModel {
        // A/m = pi r^2 / m = 0.02 m^2/kg (spec §4.4's typical value).
        SpacecraftModel {
            mass_kg: 157.0,
            radius_m: 1.0,
            c_r: 1.3,
            c_d: 2.2,
        }
    }

    /// Magnitude ~ 1.2e-7 m/s^2 at 1 AU for C_r A/m ~ 0.026 (spec §4.4),
    /// and the push points away from the Sun.
    #[test]
    fn magnitude_and_direction_at_one_au() {
        let ctx = context();
        let force = SolarRadiationPressure {
            spacecraft: spacecraft(),
            occulters: Vec::new(),
        };
        // A sunlit spacecraft position: displaced toward the Sun.
        let sun_m = ctx.geocentric_pos_m(crate::ephemeris::Body::Sol).unwrap();
        let r_m = sun_m.normalize() * 7.0e6;
        let r_can = ctx.units.length_to_can(r_m);
        let a_can = force
            .acceleration_can(&ctx, r_can, glam::DVec3::ZERO)
            .unwrap();
        let a_m_s2 = a_can * ctx.units.acu_m_s2();
        // Scale the 1-AU reference by the ACTUAL Sun distance (mid-January
        // sits near perihelion, ~0.984 AU -> +3.3%).
        let distance = (r_m - sun_m).length();
        let expected =
            SOLAR_PRESSURE_N_M2 * (AU_M / distance).powi(2) * 1.3 * std::f64::consts::PI / 157.0;
        assert!(
            (a_m_s2.length() - expected).abs() < 1e-3 * expected,
            "|a| = {:.3e} vs {expected:.3e}",
            a_m_s2.length()
        );
        assert!(
            a_m_s2.normalize().dot((r_m - sun_m).normalize()) > 0.999,
            "SRP must push away from the Sun"
        );
    }

    /// With the central body as occulter, the anti-sunward LEO point is in
    /// umbra: nu = 0 and the force vanishes; the sunward point is clear.
    #[test]
    fn central_occulter_shadows_the_antisolar_point() {
        let ctx = context();
        let force = SolarRadiationPressure {
            spacecraft: spacecraft(),
            occulters: vec![Occulter::Central { radius_m: 6.378e6 }],
        };
        let sun_dir = ctx
            .geocentric_pos_m(crate::ephemeris::Body::Sol)
            .unwrap()
            .normalize();
        let shadowed = ctx.units.length_to_can(-sun_dir * 7.0e6);
        assert_eq!(force.illumination_at(&ctx, shadowed).unwrap(), 0.0);
        assert_eq!(
            force
                .acceleration_can(&ctx, shadowed, glam::DVec3::ZERO)
                .unwrap(),
            glam::DVec3::ZERO
        );
        let sunlit = ctx.units.length_to_can(sun_dir * 7.0e6);
        assert_eq!(force.illumination_at(&ctx, sunlit).unwrap(), 1.0);
        // Boundary-function signs: both + in full sun, both - in umbra.
        let (outer, inner) = force.boundary_functions(&ctx, sunlit).unwrap();
        assert!(outer > 0.0 && inner > 0.0, "sunlit: ({outer}, {inner})");
        let (outer, inner) = force.boundary_functions(&ctx, shadowed).unwrap();
        assert!(outer < 0.0 && inner < 0.0, "umbra: ({outer}, {inner})");
    }
}
