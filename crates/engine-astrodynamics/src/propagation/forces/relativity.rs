//! First-order relativistic (Schwarzschild) correction of the central body
//! (spec §4.8), IERS Conventions form with beta = gamma = 1. Default ON
//! for every segment: at Earth it drives a secular perigee drift visible
//! at the accuracy target; for Mercury it is the classic 43"/century
//! perihelion advance the validation battery checks (§7.11).

use glam::DVec3;

use super::{EvalContext, ForceModel, SPEED_OF_LIGHT_M_S};
use crate::propagation::units::CanonicalUnits;

pub(crate) struct Schwarzschild {
    /// The speed of light in the segment's canonical velocity unit,
    /// converted once at setup (spec §4.8).
    c_can: f64,
}

impl Schwarzschild {
    pub(crate) fn new(units: &CanonicalUnits) -> Self {
        Self {
            c_can: SPEED_OF_LIGHT_M_S / units.vu_m_s(),
        }
    }
}

impl ForceModel for Schwarzschild {
    fn acceleration_can(
        &self,
        _ctx: &EvalContext,
        r_can: DVec3,
        v_can: DVec3,
    ) -> Result<DVec3, String> {
        // Central mu in canonical units is 1 by construction; the few-1e-8
        // gap to a harmonic model's own GM is far below this term's size.
        let mu = 1.0;
        let r2 = r_can.length_squared();
        let r = r2.sqrt();
        let c2 = self.c_can * self.c_can;
        let a = (mu / (c2 * r2 * r))
            * ((4.0 * mu / r - v_can.length_squared()) * r_can + 4.0 * r_can.dot(v_can) * v_can);
        Ok(a)
    }
}
