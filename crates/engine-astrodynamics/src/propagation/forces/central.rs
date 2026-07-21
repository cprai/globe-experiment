//! Central-body gravity as a force term: canonical <-> SI conversion at
//! the model boundary, plus the inertial <-> body-fixed rotation when the
//! field needs it (harmonics; spec §4.1).

use glam::DVec3;

use super::{EvalContext, ForceModel};
use crate::propagation::bodies::GravityField;

/// Below this central distance the `1/r^3` terms are meaningless and about
/// to overflow; erring here (spec §2) beats a silent NaN inside DOP853.
const MIN_RADIUS_M: f64 = 1.0;

pub(crate) struct CentralGravity {
    pub field: Box<dyn GravityField>,
}

impl CentralGravity {
    fn body_fixed_rotation(&self, epoch: hifitime::Epoch) -> Result<glam::DQuat, String> {
        use anise::constants::frames::{EARTH_ITRF93, EARTH_J2000};
        let dcm = crate::data::context()
            .almanac
            .rotate(EARTH_J2000, EARTH_ITRF93, epoch)
            .map_err(|error| format!("body-fixed rotation at {epoch}: {error}"))?;
        Ok(crate::frames::dquat(&dcm))
    }
}

impl ForceModel for CentralGravity {
    fn acceleration_can(
        &self,
        ctx: &EvalContext,
        r_can: DVec3,
        _v_can: DVec3,
    ) -> Result<DVec3, String> {
        let r_m = ctx.units.length_to_m(r_can);
        if r_m.length_squared() < MIN_RADIUS_M * MIN_RADIUS_M {
            return Err(format!(
                "central-body singularity: |r| = {} m",
                r_m.length()
            ));
        }
        let a_m_s2 = if self.field.needs_body_fixed() {
            let q = self.body_fixed_rotation(ctx.epoch)?;
            if self.field.is_time_dependent() {
                let sun_m = ctx.geocentric_pos_m(crate::ephemeris::Body::Sol)?;
                let moon_m = ctx.geocentric_pos_m(crate::ephemeris::Body::Luna)?;
                self.field.update_time_dependence(q * sun_m, q * moon_m);
            }
            q.inverse() * self.field.acceleration_m_s2(q * r_m)
        } else {
            self.field.acceleration_m_s2(r_m)
        };
        Ok(ctx.units.accel_to_can(a_m_s2))
    }
}
