//! Planetary albedo (reflected sunlight) and thermal IR (the body's own
//! emission), spec §4.6: both act RADIALLY OUTWARD from the central body,
//! unlike direct SRP which acts anti-sunward. The minimum viable model -
//! a single-element disk with view factor - is implemented; the coarse
//! ring discretization is the recorded upgrade path if the accuracy budget
//! ever demands it.
//!
//! The Bond albedo table lives here: anise's planetary-constants kernel
//! carries no albedo anywhere (verified), so spec §4.0's "albedo from
//! ANISE" is unsatisfiable as written - this in-crate table is the
//! sanctioned exception (plan §5).

use glam::DVec3;

use super::srp::AU_M;
use super::{EvalContext, ForceModel, SPEED_OF_LIGHT_M_S};
use crate::ephemeris::Body;
use crate::propagation::spacecraft::SpacecraftModel;

/// Total solar irradiance at 1 AU (the pair-mate of `SOLAR_PRESSURE_N_M2`).
const SOLAR_CONSTANT_W_M2: f64 = 1361.0;

/// Beyond this view factor the fluxes are < 1e-6 of their surface values;
/// skip the term entirely (spec §4: skip, don't multiply by zero).
const VIEW_FACTOR_FLOOR: f64 = 1e-6;

/// Bond albedo by NAIF id - the sanctioned in-crate table (see module
/// doc). Values are the standard planetary Bond albedos; a constant per
/// body (time/cloud variability is out of scope, spec §4.9).
pub(crate) fn bond_albedo(naif_id: i32) -> Option<f64> {
    Some(match naif_id {
        199 => 0.088, // Mercury
        299 => 0.76,  // Venus - the extreme case
        399 => 0.306, // Earth
        301 => 0.11,  // Luna
        4 => 0.25,    // Mars
        5 => 0.503,   // Jupiter
        6 => 0.342,   // Saturn
        7 => 0.300,   // Uranus
        8 => 0.290,   // Neptune
        _ => return None,
    })
}

/// Albedo + thermal IR of the central body on the cannonball spacecraft.
pub(crate) struct PlanetaryRadiation {
    pub spacecraft: SpacecraftModel,
    pub body_radius_m: f64,
    pub bond_albedo: f64,
}

impl ForceModel for PlanetaryRadiation {
    fn acceleration_can(
        &self,
        ctx: &EvalContext,
        r_can: DVec3,
        _v_can: DVec3,
    ) -> Result<DVec3, String> {
        let r_m = ctx.units.length_to_m(r_can);
        let r = r_m.length();
        let view = (self.body_radius_m / r).powi(2);
        if view < VIEW_FACTOR_FLOOR {
            return Ok(DVec3::ZERO); // negligible beyond a few radii - skip
        }
        let radial = r_m / r;

        let sun_m = ctx.geocentric_pos_m(Body::Sol)?;
        // Solar flux AT the central body, scaled from 1 AU.
        let local_solar = SOLAR_CONSTANT_W_M2 * (AU_M / sun_m.length()).powi(2);

        // Albedo: reflected sunlight, scaled by the view factor and the
        // sunlit fraction of the visible disk ((1 + cos phase)/2 - exactly
        // zero when only the night side is visible, which also makes the
        // term vanish through an eclipse transit).
        let phase = 0.5 * (1.0 + radial.dot(sun_m / sun_m.length()));
        let albedo_flux = self.bond_albedo * local_solar * view * phase;

        // Thermal IR: the body's own emission, roughly isotropic over the
        // disk and present on the night side too (it does NOT vanish in
        // eclipse). Radiative balance ties the exitance to the absorbed
        // quarter-flux: (1 - albedo) S / 4.
        let ir_flux = (1.0 - self.bond_albedo) / 4.0 * local_solar * view;

        let a_m_s2 = (albedo_flux + ir_flux) / SPEED_OF_LIGHT_M_S
            * self.spacecraft.c_r
            * self.spacecraft.area_m2(radial)
            / self.spacecraft.mass_kg
            * radial;
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

    fn force() -> PlanetaryRadiation {
        PlanetaryRadiation {
            spacecraft: SpacecraftModel {
                mass_kg: 157.0,
                radius_m: 1.0,
                c_r: 1.3,
                c_d: 2.2,
            },
            body_radius_m: 6.378e6,
            bond_albedo: bond_albedo(399).unwrap(),
        }
    }

    /// In LEO over the subsolar region, albedo + IR together are the
    /// spec's 10-30% of direct SRP, pointing radially outward; over the
    /// night side only IR remains (nonzero!), and far away the term
    /// vanishes entirely.
    #[test]
    fn magnitudes_directions_and_cutoff() {
        let ctx = context();
        let force = force();
        let sun_dir = ctx
            .geocentric_pos_m(crate::ephemeris::Body::Sol)
            .unwrap()
            .normalize();
        let srp_scale = super::super::srp::SOLAR_PRESSURE_N_M2 * 1.3 * std::f64::consts::PI / 157.0;

        let dayside = ctx.units.length_to_can(sun_dir * 7.0e6);
        let a_day =
            force.acceleration_can(&ctx, dayside, DVec3::ZERO).unwrap() * ctx.units.acu_m_s2();
        assert!(
            (0.05..0.5).contains(&(a_day.length() / srp_scale)),
            "dayside albedo+IR is {:.2}x SRP",
            a_day.length() / srp_scale
        );
        assert!(
            a_day.normalize().dot(sun_dir) > 0.999,
            "must point radially outward"
        );

        let nightside = ctx.units.length_to_can(-sun_dir * 7.0e6);
        let a_night = force
            .acceleration_can(&ctx, nightside, DVec3::ZERO)
            .unwrap()
            * ctx.units.acu_m_s2();
        assert!(
            a_night.length() > 0.0 && a_night.length() < a_day.length(),
            "night side keeps IR only: {:.3e} vs day {:.3e}",
            a_night.length(),
            a_day.length()
        );
        assert!(a_night.normalize().dot(-sun_dir) > 0.999);

        let far = ctx.units.length_to_can(sun_dir * 1.0e10);
        assert_eq!(
            force.acceleration_can(&ctx, far, DVec3::ZERO).unwrap(),
            DVec3::ZERO,
            "far-field cutoff skips the term"
        );
    }

    /// The sanctioned albedo table covers every impostor body; Venus is
    /// the extreme case the spec calls out.
    #[test]
    fn albedo_table_covers_the_bodies() {
        for id in [199, 299, 399, 301, 4, 5, 6, 7, 8] {
            let albedo = bond_albedo(id).unwrap();
            assert!((0.0..1.0).contains(&albedo), "albedo({id}) = {albedo}");
        }
        assert_eq!(bond_albedo(299), Some(0.76));
        assert_eq!(bond_albedo(999), None);
    }
}
