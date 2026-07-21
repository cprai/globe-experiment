//! The force model (spec §4): total acceleration = central gravity + the
//! enabled perturbations, every term an implementation of [`ForceModel`].
//! Terms that do not apply in a regime are SKIPPED (not evaluated and
//! multiplied by zero) - ephemeris queries are the hot path.

pub(crate) mod albedo_ir;
pub(crate) mod atmosphere;
pub(crate) mod central;
pub(crate) mod drag;
pub(crate) mod harmonics;
pub(crate) mod relativity;
pub(crate) mod shadow;
pub(crate) mod srp;
pub(crate) mod third_body;

/// Shared physical constant for the radiation and relativity terms.
pub(crate) const SPEED_OF_LIGHT_M_S: f64 = 299_792_458.0;

use std::cell::OnceCell;

use glam::{DQuat, DVec3};
use hifitime::Epoch;

use super::units::CanonicalUnits;
use central::CentralGravity;

/// Everything a force needs at one derivative evaluation. Sun/Luna
/// geocentric positions are fetched at most once per evaluation and shared
/// (third-body gravity and the solid-tide hook want the same lookups -
/// ephemeris queries are the hot path, spec §4/§6).
pub(crate) struct EvalContext {
    pub units: CanonicalUnits,
    pub epoch: Epoch,
    sun_geocentric_m: OnceCell<Result<DVec3, String>>,
    moon_geocentric_m: OnceCell<Result<DVec3, String>>,
    earth_rotation: OnceCell<Result<(DQuat, DVec3), String>>,
}

impl EvalContext {
    pub(crate) fn new(units: CanonicalUnits, epoch: Epoch) -> Self {
        Self {
            units,
            epoch,
            sun_geocentric_m: OnceCell::new(),
            moon_geocentric_m: OnceCell::new(),
            earth_rotation: OnceCell::new(),
        }
    }

    /// The GCRF -> ITRF rotation and Terra's angular velocity (GCRF,
    /// rad/s) at this epoch, computed at most once per evaluation and
    /// shared between harmonic gravity and drag. Omega comes from the
    /// rotation-matrix derivative - the same source as the rotation
    /// itself, never a hand-rolled rate constant (plan §5): for
    /// `R: gcrf -> itrf`, the skew part of `Rdot^T R` IS `[omega x]`.
    pub(crate) fn earth_rotation(&self) -> Result<(DQuat, DVec3), String> {
        use anise::constants::frames::{EARTH_ITRF93, EARTH_J2000};
        self.earth_rotation
            .get_or_init(|| {
                let dcm = crate::data::context()
                    .almanac
                    .rotate(EARTH_J2000, EARTH_ITRF93, self.epoch)
                    .map_err(|error| format!("body-fixed rotation at {}: {error}", self.epoch))?;
                let q = crate::frames::dquat(&dcm);
                let r = &dcm.rot_mat;
                let r_dot = dcm
                    .rot_mat_dt
                    .ok_or_else(|| format!("rotation at {} carries no derivative", self.epoch))?;
                let w = r_dot.transpose() * r;
                // Antisymmetrized skew components of [omega x].
                let omega = DVec3::new(
                    (w[(2, 1)] - w[(1, 2)]) / 2.0,
                    (w[(0, 2)] - w[(2, 0)]) / 2.0,
                    (w[(1, 0)] - w[(0, 1)]) / 2.0,
                );
                Ok((q, omega))
            })
            .clone()
    }

    /// Geocentric position of `body`, meters - cached for Sol/Luna.
    pub(crate) fn geocentric_pos_m(&self, body: crate::ephemeris::Body) -> Result<DVec3, String> {
        use crate::ephemeris::Body;
        let fetch = || {
            crate::ephemeris::geocentric_pos(body, self.epoch).map_err(|error| error.to_string())
        };
        match body {
            Body::Sol => self.sun_geocentric_m.get_or_init(fetch).clone(),
            Body::Luna => self.moon_geocentric_m.get_or_init(fetch).clone(),
            _ => fetch(),
        }
    }
}

/// One acceleration term, canonical units in and out (spec §4). Errs
/// instead of returning non-finite values.
pub(crate) trait ForceModel {
    fn acceleration_can(
        &self,
        ctx: &EvalContext,
        r_can: DVec3,
        v_can: DVec3,
    ) -> Result<DVec3, String>;
}

/// The assembled dynamics of one segment: canonical units, the central
/// field, and the enabled perturbations.
pub(crate) struct DynamicsModel {
    pub units: CanonicalUnits,
    pub central: CentralGravity,
    pub perturbations: Vec<Box<dyn ForceModel>>,
}

impl DynamicsModel {
    pub(crate) fn acceleration_can(
        &self,
        epoch: Epoch,
        r_can: DVec3,
        v_can: DVec3,
    ) -> Result<DVec3, String> {
        let ctx = EvalContext::new(self.units, epoch);
        let mut total = self.central.acceleration_can(&ctx, r_can, v_can)?;
        for force in &self.perturbations {
            total += force.acceleration_can(&ctx, r_can, v_can)?;
        }
        Ok(total)
    }
}
