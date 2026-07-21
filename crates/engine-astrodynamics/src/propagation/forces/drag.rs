//! Atmospheric drag (spec §4.7):
//! `a = -1/2 rho |v_rel| v_rel C_d (A/m)`, with the atmosphere-relative
//! velocity accounting for co-rotation - `v_rel = v - omega x r`
//! (`omega R` is ~490 m/s at LEO radius; omitting it is a classic large
//! error). `omega` comes from the same rotation-matrix derivative the
//! frame chain uses, never a hand-rolled rate constant (plan §5).
//!
//! Drag is non-conservative and dissipative: energy-conservation checks
//! must exclude arcs where it is active (spec §4.7).

use glam::DVec3;

use super::{EvalContext, ForceModel};
use crate::propagation::bodies::AtmosphereModel;
use crate::propagation::spacecraft::SpacecraftModel;

pub(crate) struct AtmosphericDrag {
    pub spacecraft: SpacecraftModel,
    pub atmosphere: Box<dyn AtmosphereModel>,
}

impl ForceModel for AtmosphericDrag {
    fn acceleration_can(
        &self,
        ctx: &EvalContext,
        r_can: DVec3,
        v_can: DVec3,
    ) -> Result<DVec3, String> {
        let r_m = ctx.units.length_to_m(r_can);
        let (q_gcrf_to_body, omega_gcrf) = ctx.earth_rotation()?;
        let density = match self
            .atmosphere
            .density_kg_m3(q_gcrf_to_body * r_m, ctx.epoch)?
        {
            Some(density) => density,
            None => return Ok(DVec3::ZERO), // vacuum body or above the ceiling: skip
        };

        let v_m_s = ctx.units.velocity_to_m_s(v_can);
        let v_rel = v_m_s - omega_gcrf.cross(r_m);
        let speed = v_rel.length();
        if speed == 0.0 {
            return Ok(DVec3::ZERO);
        }
        let direction = v_rel / speed;
        let a_m_s2 =
            -0.5 * density * speed * self.spacecraft.c_d * self.spacecraft.area_m2(direction)
                / self.spacecraft.mass_kg
                * v_rel;
        Ok(ctx.units.accel_to_can(a_m_s2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::propagation::forces::atmosphere::EarthAtmosphere;
    use crate::propagation::units::CanonicalUnits;
    use hifitime::Epoch;

    fn context() -> EvalContext {
        crate::init();
        EvalContext::new(
            CanonicalUnits::new(3.986_004_418e14, 6_378_137.0),
            Epoch::from_gregorian_utc(2024, 1, 15, 12, 0, 0, 0),
        )
    }

    fn drag() -> AtmosphericDrag {
        AtmosphericDrag {
            spacecraft: SpacecraftModel {
                mass_kg: 157.0,
                radius_m: 1.0,
                c_r: 1.3,
                c_d: 2.2,
            },
            atmosphere: Box::new(EarthAtmosphere::new()),
        }
    }

    /// The shared rotation derivative yields Terra's sidereal rate about
    /// the (near-)polar axis - the co-rotation term's foundation.
    #[test]
    fn earth_angular_velocity_from_the_rotation_derivative() {
        let ctx = context();
        let (_, omega) = ctx.earth_rotation().unwrap();
        assert!(
            (omega.length() - 7.292_115e-5).abs() < 1e-8,
            "|omega| = {:.6e} rad/s",
            omega.length()
        );
        assert!(
            omega.normalize().z > 0.999,
            "omega must point near +Z (GCRF): {omega:?}"
        );
    }

    /// Spec §7.15's co-rotation A/B: at the same LEO point, a retrograde
    /// orbit faces a larger atmosphere-relative speed than a prograde one
    /// (the atmosphere moves eastward), so its drag is markedly stronger -
    /// the asymmetry only exists if the omega term is real.
    #[test]
    fn co_rotation_asymmetry() {
        let ctx = context();
        let force = drag();
        let radius = 6_378_137.0 + 300e3;
        let r_can = ctx.units.length_to_can(DVec3::new(radius, 0.0, 0.0));
        let speed = (3.986_004_418e14_f64 / radius).sqrt();
        let prograde = ctx.units.velocity_to_can(DVec3::new(0.0, speed, 0.0));
        let retrograde = ctx.units.velocity_to_can(DVec3::new(0.0, -speed, 0.0));

        let a_pro = force.acceleration_can(&ctx, r_can, prograde).unwrap();
        let a_retro = force.acceleration_can(&ctx, r_can, retrograde).unwrap();
        let ratio = a_retro.length() / a_pro.length();
        // (v + omega R)^2 / (v - omega R)^2 with omega R ~ 490 m/s of
        // ~7730 m/s: ~1.29.
        assert!(
            (1.15..1.45).contains(&ratio),
            "retro/pro drag ratio = {ratio:.3}"
        );
        // And drag opposes the relative wind.
        let v_m = ctx.units.velocity_to_m_s(prograde);
        let v_rel = v_m
            - ctx
                .earth_rotation()
                .unwrap()
                .1
                .cross(DVec3::new(radius, 0.0, 0.0));
        assert!(
            (a_pro.normalize()).dot(v_rel.normalize()) < -0.999,
            "drag must oppose v_rel"
        );
    }

    /// Above the model ceiling the force skips entirely.
    #[test]
    fn skips_above_the_ceiling() {
        let ctx = context();
        let force = drag();
        let r_can = ctx
            .units
            .length_to_can(DVec3::new(6_378_137.0 + 1_500e3, 0.0, 0.0));
        let v_can = ctx.units.velocity_to_can(DVec3::new(0.0, 7_000.0, 0.0));
        assert_eq!(
            force.acceleration_can(&ctx, r_can, v_can).unwrap(),
            DVec3::ZERO
        );
    }
}
