//! Cowell's method (spec §2): direct integration of the Cartesian state
//! `[r, v]` in canonical units. No reference conic, no transformation -
//! it accepts any force model and any eccentricity.

use glam::DVec3;
use hifitime::{Duration, Epoch};
use nalgebra::SVector;

use crate::propagation::forces::DynamicsModel;
use crate::propagation::integrator::Dynamics;

pub(crate) struct CowellSystem<'a> {
    pub model: &'a DynamicsModel,
    /// The segment anchor; the integrator variable is the canonical time
    /// offset from it (spec §5 - never absolute seconds).
    pub anchor: Epoch,
}

pub(crate) fn pack(r_can: DVec3, v_can: DVec3) -> SVector<f64, 6> {
    SVector::<f64, 6>::from([r_can.x, r_can.y, r_can.z, v_can.x, v_can.y, v_can.z])
}

pub(crate) fn unpack(y: &SVector<f64, 6>) -> (DVec3, DVec3) {
    (DVec3::new(y[0], y[1], y[2]), DVec3::new(y[3], y[4], y[5]))
}

impl Dynamics<6> for CowellSystem<'_> {
    fn derivative(&self, t: f64, y: &SVector<f64, 6>) -> Result<SVector<f64, 6>, String> {
        let (r, v) = unpack(y);
        let epoch = self.anchor + Duration::from_seconds(self.model.units.time_to_s(t));
        let a = self.model.acceleration_can(epoch, r, v)?;
        Ok(pack(v, a))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::propagation::bodies::PointMass;
    use crate::propagation::forces::central::CentralGravity;
    use crate::propagation::forces::relativity::Schwarzschild;
    use crate::propagation::integrator::{SolveConfig, solve_arc};
    use crate::propagation::units::CanonicalUnits;

    const MU_EARTH_M3_S2: f64 = 3.986_004_418e14;
    const MU_SUN_M3_S2: f64 = 1.327_124_400_18e20;
    const AU_M: f64 = 1.495_978_707e11;
    const TAU: f64 = std::f64::consts::TAU;

    fn config() -> SolveConfig {
        SolveConfig {
            rtol: 1e-12,
            atol: 1e-12,
            dense_points_per_step: 4,
        }
    }

    fn anchor() -> Epoch {
        Epoch::from_gregorian_utc(2020, 1, 1, 0, 0, 0, 0)
    }

    /// Two-body model in canonical units (mu = 1 by construction).
    fn two_body(mu_m3_s2: f64, du_m: f64) -> DynamicsModel {
        let units = CanonicalUnits::new(mu_m3_s2, du_m);
        DynamicsModel {
            units,
            central: CentralGravity {
                field: Box::new(PointMass {
                    mu_m3_s2,
                    reference_radius_m: du_m,
                }),
            },
            perturbations: Vec::new(),
        }
    }

    /// Perifocal two-body state at time `t` from perigee, mu = 1: solves
    /// Kepler's equation by Newton iteration - the closed-form reference
    /// of spec §7.1.
    fn kepler_closed_form(a: f64, e: f64, t: f64) -> (DVec3, DVec3) {
        let mean_anomaly = t * a.powf(-1.5);
        let mut ecc_anomaly = mean_anomaly;
        for _ in 0..64 {
            let delta = (ecc_anomaly - e * ecc_anomaly.sin() - mean_anomaly)
                / (1.0 - e * ecc_anomaly.cos());
            ecc_anomaly -= delta;
            if delta.abs() < 1e-15 {
                break;
            }
        }
        let (sin_e, cos_e) = ecc_anomaly.sin_cos();
        let b_over_a = (1.0 - e * e).sqrt();
        let radius = a * (1.0 - e * cos_e);
        let position = DVec3::new(a * (cos_e - e), a * b_over_a * sin_e, 0.0);
        let velocity = DVec3::new(-sin_e, b_over_a * cos_e, 0.0) * (a.sqrt() / radius);
        (position, velocity)
    }

    /// Spec §7.1 (circular): 0.37 of a circular period against the exact
    /// rotation, near machine precision.
    #[test]
    fn circular_two_body_matches_closed_form() {
        let model = two_body(MU_EARTH_M3_S2, 7.0e6);
        let system = CowellSystem {
            model: &model,
            anchor: anchor(),
        };
        let t = 0.37 * TAU;
        let arc = solve_arc(&system, 0.0, t, pack(DVec3::X, DVec3::Y), &config()).unwrap();
        let (r, v) = unpack(arc.y.last().unwrap());
        let want_r = DVec3::new(t.cos(), t.sin(), 0.0);
        let want_v = DVec3::new(-t.sin(), t.cos(), 0.0);
        assert!(
            (r - want_r).length() < 1e-10,
            "position {r:?} vs {want_r:?}"
        );
        assert!(
            (v - want_v).length() < 1e-10,
            "velocity {v:?} vs {want_v:?}"
        );
    }

    /// Spec §7.1 (elliptic): e = 0.6 against the Kepler-equation closed
    /// form at an arbitrary fraction of a period.
    #[test]
    fn elliptic_two_body_matches_kepler_solver() {
        let (a, e) = (1.5, 0.6);
        let model = two_body(MU_EARTH_M3_S2, 7.0e6);
        let system = CowellSystem {
            model: &model,
            anchor: anchor(),
        };
        let perigee = pack(
            DVec3::new(a * (1.0 - e), 0.0, 0.0),
            DVec3::new(0.0, ((1.0 + e) / (a * (1.0 - e))).sqrt(), 0.0),
        );
        let t = 0.37 * TAU * a.powf(1.5);
        let arc = solve_arc(&system, 0.0, t, perigee, &config()).unwrap();
        let (r, v) = unpack(arc.y.last().unwrap());
        let (want_r, want_v) = kepler_closed_form(a, e, t);
        assert!((r - want_r).length() < 1e-9, "position {r:?} vs {want_r:?}");
        assert!((v - want_v).length() < 1e-9, "velocity {v:?} vs {want_v:?}");
    }

    /// Spec §7.2: gravity-only long arc; energy and angular momentum may
    /// drift (DOP853 is not symplectic) but only within what the 1e-12
    /// tolerance implies.
    #[test]
    fn energy_and_momentum_drift_bounded_over_fifty_orbits() {
        let (a, e) = (1.2, 0.3);
        let model = two_body(MU_EARTH_M3_S2, 7.0e6);
        let system = CowellSystem {
            model: &model,
            anchor: anchor(),
        };
        let y0 = pack(
            DVec3::new(a * (1.0 - e), 0.0, 0.0),
            DVec3::new(0.0, ((1.0 + e) / (a * (1.0 - e))).sqrt(), 0.0),
        );
        let t = 50.0 * TAU * a.powf(1.5);
        let arc = solve_arc(&system, 0.0, t, y0, &config()).unwrap();

        let invariants = |y: &SVector<f64, 6>| {
            let (r, v) = unpack(y);
            (v.length_squared() / 2.0 - 1.0 / r.length(), r.cross(v))
        };
        let (energy_0, h_0) = invariants(&y0);
        let (energy_end, h_end) = invariants(arc.y.last().unwrap());
        assert!(
            ((energy_end - energy_0) / energy_0).abs() < 1e-9,
            "energy drift {:.3e}",
            ((energy_end - energy_0) / energy_0).abs()
        );
        assert!(
            (h_end - h_0).length() / h_0.length() < 1e-9,
            "momentum drift {:.3e}",
            (h_end - h_0).length() / h_0.length()
        );
    }

    /// Spec §7.7: forward then backward returns the initial state - and
    /// proves the solver accepts tf < t0.
    #[test]
    fn forward_backward_round_trip() {
        let (a, e) = (1.4, 0.7);
        let model = two_body(MU_EARTH_M3_S2, 7.0e6);
        let system = CowellSystem {
            model: &model,
            anchor: anchor(),
        };
        let y0 = pack(
            DVec3::new(a * (1.0 - e), 0.0, 0.0),
            DVec3::new(0.0, ((1.0 + e) / (a * (1.0 - e))).sqrt(), 0.0),
        );
        let t = 2.3 * TAU * a.powf(1.5);
        let forward = solve_arc(&system, 0.0, t, y0, &config()).unwrap();
        let back = solve_arc(&system, t, 0.0, *forward.y.last().unwrap(), &config()).unwrap();
        let returned = back.y.first().unwrap();
        let difference = (returned - y0).norm();
        assert!(difference < 1e-8, "round trip differs by {difference:.3e}");
    }

    /// Spec §7.11: two-body + Schwarzschild with Mercury's elements about
    /// the Sun recovers the ~43"/century perihelion advance
    /// (6 pi mu / (c^2 a (1 - e^2)) ~ 5.02e-7 rad per orbit).
    #[test]
    fn mercury_perihelion_advance() {
        let (a, e) = (0.387_098, 0.205_630);
        let units = CanonicalUnits::new(MU_SUN_M3_S2, AU_M);
        let eccentricity_vector = |y: &SVector<f64, 6>| {
            let (r, v) = unpack(y);
            v.cross(r.cross(v)) - r.normalize()
        };
        let advance = |with_relativity: bool| {
            let mut model = DynamicsModel {
                units,
                central: CentralGravity {
                    field: Box::new(PointMass {
                        mu_m3_s2: MU_SUN_M3_S2,
                        reference_radius_m: 6.957e8,
                    }),
                },
                perturbations: Vec::new(),
            };
            if with_relativity {
                model
                    .perturbations
                    .push(Box::new(Schwarzschild::new(&units)));
            }
            let system = CowellSystem {
                model: &model,
                anchor: anchor(),
            };
            let y0 = pack(
                DVec3::new(a * (1.0 - e), 0.0, 0.0),
                DVec3::new(0.0, ((1.0 + e) / (a * (1.0 - e))).sqrt(), 0.0),
            );
            let period = TAU * a.powf(1.5);
            let arc = solve_arc(&system, 0.0, period, y0, &config()).unwrap();
            let e0 = eccentricity_vector(&y0);
            let e1 = eccentricity_vector(arc.y.last().unwrap());
            e0.cross(e1).length() / (e0.length() * e1.length())
        };

        let predicted = 5.019e-7;
        let measured = advance(true);
        assert!(
            (measured - predicted).abs() < 0.05 * predicted,
            "relativistic advance {measured:.4e} vs {predicted:.4e}"
        );
        let control = advance(false);
        assert!(
            control < 5e-9,
            "two-body control shows spurious advance {control:.3e}"
        );
    }
}
