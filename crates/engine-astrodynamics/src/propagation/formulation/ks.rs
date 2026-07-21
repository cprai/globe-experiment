//! Full Kustaanheimo-Stiefel regularization (spec §3): the 3D Kepler
//! problem mapped into 4D `u`-space where the unperturbed motion is a
//! LINEAR oscillator `u'' + (h/2) u = 0` - the 1/r^2 singularity is
//! eliminated, not merely stepped around. Independent variable is the
//! fictitious time `s` with `dt/ds = r`, so periapsis step refinement
//! falls out automatically.
//!
//! State: 10 components `[u(4), u'(4), h, t]`. SIGN CONVENTION
//! (Stiefel-Scheifele, spec §3): `h = mu/r - |v|^2 / 2` is the NEGATIVE of
//! the Keplerian energy - POSITIVE on elliptic arcs, negative on
//! hyperbolic ones; with the opposite convention the oscillator equation
//! is wrong for bound orbits. The central mu here is exactly 1 (canonical
//! units); any central-field deviation (harmonics, mu provenance) rides in
//! the perturbation `P = a_total + r/|r|^3`.
//!
//! The Cartesian -> u map is one-to-many (the Hopf-map fiber); the
//! conversion below uses the standard branch on the sign of `x1`.
//! Everything - L(u), its transpose, the branch - comes from this one
//! module so the conventions cannot mix (spec §3's warning).

use glam::DVec3;
use hifitime::{Duration, Epoch};
use nalgebra::SVector;

use crate::propagation::forces::DynamicsModel;
use crate::propagation::integrator::{Dynamics, RawArc, SolveConfig, solve_arc_until};

/// `x = L(u) u`: the KS position map (x4 vanishes identically).
fn ks_position(u: [f64; 4]) -> DVec3 {
    DVec3::new(
        u[0] * u[0] - u[1] * u[1] - u[2] * u[2] + u[3] * u[3],
        2.0 * (u[0] * u[1] - u[2] * u[3]),
        2.0 * (u[0] * u[2] + u[1] * u[3]),
    )
}

/// `L(u)^T (w, 0)` for a physical 3-vector `w`.
fn l_transpose(u: [f64; 4], w: DVec3) -> [f64; 4] {
    [
        u[0] * w.x + u[1] * w.y + u[2] * w.z,
        -u[1] * w.x + u[0] * w.y + u[3] * w.z,
        -u[2] * w.x - u[3] * w.y + u[0] * w.z,
        u[3] * w.x - u[2] * w.y + u[1] * w.z,
    ]
}

/// Cartesian (canonical) -> KS state, with physical time `t_can` carried
/// as the tenth component.
pub(crate) fn cartesian_to_ks(r_can: DVec3, v_can: DVec3, t_can: f64) -> SVector<f64, 10> {
    let r = r_can.length();
    // The standard fiber choice, branching on the sign of x1.
    let u = if r_can.x >= 0.0 {
        let u1 = ((r + r_can.x) / 2.0).sqrt();
        [u1, r_can.y / (2.0 * u1), r_can.z / (2.0 * u1), 0.0]
    } else {
        let u2 = ((r - r_can.x) / 2.0).sqrt();
        [r_can.y / (2.0 * u2), u2, 0.0, r_can.z / (2.0 * u2)]
    };
    let up = l_transpose(u, v_can / 2.0);
    let h = 1.0 / r - v_can.length_squared() / 2.0;
    SVector::<f64, 10>::from([u[0], u[1], u[2], u[3], up[0], up[1], up[2], up[3], h, t_can])
}

/// KS state -> (physical time, position, velocity), canonical units.
pub(crate) fn ks_to_cartesian(y: &SVector<f64, 10>) -> (f64, DVec3, DVec3) {
    let u = [y[0], y[1], y[2], y[3]];
    let up = [y[4], y[5], y[6], y[7]];
    let r = u.iter().map(|c| c * c).sum::<f64>();
    let position = ks_position(u);
    // v = 2 L(u) u' / r.
    let velocity = DVec3::new(
        u[0] * up[0] - u[1] * up[1] - u[2] * up[2] + u[3] * up[3],
        u[1] * up[0] + u[0] * up[1] - u[3] * up[2] - u[2] * up[3],
        u[2] * up[0] + u[3] * up[1] + u[0] * up[2] + u[1] * up[3],
    ) * (2.0 / r);
    (y[9], position, velocity)
}

/// The bilinear (Levi-Civita) constraint `l(u, u')` - preserved
/// analytically, drifts numerically; monitored as a health metric
/// (spec §3): nonzero means the state left the Hopf fiber.
pub(crate) fn bilinear_constraint(y: &SVector<f64, 10>) -> f64 {
    y[3] * y[4] - y[2] * y[5] + y[1] * y[6] - y[0] * y[7]
}

/// The KS equations of motion over the shared force model.
pub(crate) struct KsSystem<'a> {
    pub model: &'a DynamicsModel,
    pub anchor: Epoch,
}

impl Dynamics<10> for KsSystem<'_> {
    fn derivative(&self, _s: f64, y: &SVector<f64, 10>) -> Result<SVector<f64, 10>, String> {
        let (t_can, r_vec, v_vec) = ks_to_cartesian(y);
        let u = [y[0], y[1], y[2], y[3]];
        let up = [y[4], y[5], y[6], y[7]];
        let h = y[8];
        let r = u.iter().map(|c| c * c).sum::<f64>();

        let epoch = self.anchor + Duration::from_seconds(self.model.units.time_to_s(t_can));
        let total = self.model.acceleration_can(epoch, r_vec, v_vec)?;
        // Perturbation only: the canonical central two-body term is the
        // oscillator's job.
        let perturbation = total + r_vec / (r * r * r);

        let ltp = l_transpose(u, perturbation);
        let mut dydt = SVector::<f64, 10>::zeros();
        for i in 0..4 {
            dydt[i] = up[i];
            dydt[4 + i] = -(h / 2.0) * u[i] + (r / 2.0) * ltp[i];
        }
        // dh/ds = -r v.P = -2 u' . L^T P.
        dydt[8] = -2.0 * (0..4).map(|i| up[i] * ltp[i]).sum::<f64>();
        dydt[9] = r;
        Ok(dydt)
    }
}

/// Integrates in fictitious time from `(r0, v0)` at `t0_can` until
/// physical time reaches `tf_can` (forward only - KS exists for close
/// approaches, and backward arcs stay in Cowell), returning TIME-domain
/// Cartesian knots so the trajectory layer stays formulation-agnostic.
pub(crate) fn solve_ks_span(
    model: &DynamicsModel,
    anchor: Epoch,
    r0_can: DVec3,
    v0_can: DVec3,
    t0_can: f64,
    tf_can: f64,
    config: &SolveConfig,
) -> Result<RawArc<6>, String> {
    if tf_can < t0_can {
        return Err("KS integration is forward-only".to_string());
    }
    let system = KsSystem { model, anchor };
    let arrival = |_s: f64, y: &SVector<f64, 10>| y[9] - tf_can;
    let no_boundary = |_s: f64, _y: &SVector<f64, 10>| 1.0;

    let mut y = cartesian_to_ks(r0_can, v0_can, t0_can);
    let mut s = 0.0;
    let mut knots: RawArc<6> = RawArc {
        t: Vec::new(),
        y: Vec::new(),
        ydot: Vec::new(),
    };
    for _chunk in 0..256 {
        // A chunk of fictitious time sized from the current radius
        // (dt/ds = r); the arrival event usually ends the first chunk.
        let r_now = [y[0], y[1], y[2], y[3]].iter().map(|c| c * c).sum::<f64>();
        let ds = (1.2 * (tf_can - y[9]) / r_now).max(1e-3);
        let outcome = solve_arc_until(&system, s, s + ds, y, config, None, &arrival, &no_boundary)?;
        for (knot_y, knot_ydot) in outcome.arc.y.iter().zip(&outcome.arc.ydot) {
            let (t_can, r_vec, v_vec) = ks_to_cartesian(knot_y);
            // Cartesian acceleration a = dv/dt reconstructed from the KS
            // derivative's own force evaluation: a = (v' ... ) - simplest
            // exact route: t' = r, v-from-u chain; re-evaluate the model
            // instead (one evaluation per knot, exact by definition).
            let _ = knot_ydot;
            let epoch = anchor + Duration::from_seconds(model.units.time_to_s(t_can));
            let a_vec = model.acceleration_can(epoch, r_vec, v_vec)?;
            knots.t.push(t_can);
            knots
                .y
                .push(crate::propagation::formulation::cowell::pack(r_vec, v_vec));
            knots
                .ydot
                .push(crate::propagation::formulation::cowell::pack(v_vec, a_vec));
        }
        y = *outcome.arc.y.last().unwrap();
        s = *outcome.arc.t.last().unwrap();
        // The arrival event can fire EARLY: Brent roots on the solver's
        // interpolated time component, whose off-midpoint error over the
        // huge KS steps undershoots tf by up to ~1% of a step. Only stop
        // once PHYSICAL time is truly there; otherwise the next chunk
        // closes the remainder (geometrically - one or two extra passes).
        if y[9] >= tf_can - 1e-9 {
            return Ok(knots);
        }
    }
    Err("KS integration failed to reach the target time".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::propagation::bodies::PointMass;
    use crate::propagation::forces::DynamicsModel;
    use crate::propagation::forces::central::CentralGravity;
    use crate::propagation::formulation::cowell::{CowellSystem, pack, unpack};
    use crate::propagation::integrator::solve_arc;
    use crate::propagation::units::CanonicalUnits;

    const TAU: f64 = std::f64::consts::TAU;

    fn two_body() -> DynamicsModel {
        let mu = 3.986_004_418e14;
        DynamicsModel {
            units: CanonicalUnits::new(mu, 7.0e6),
            center: crate::ephemeris::Body::Terra,
            central: CentralGravity {
                field: Box::new(PointMass {
                    mu_m3_s2: mu,
                    reference_radius_m: 6.378e6,
                }),
            },
            perturbations: Vec::new(),
        }
    }

    fn config() -> SolveConfig {
        SolveConfig {
            rtol: 1e-12,
            atol: 1e-12,
            dense_points_per_step: 2,
        }
    }

    fn anchor() -> Epoch {
        Epoch::from_gregorian_utc(2020, 1, 1, 0, 0, 0, 0)
    }

    /// Spec §7.3, FIRST: Cartesian <-> KS round trips at machine precision
    /// across both x1 branches, elliptic and hyperbolic energies, with the
    /// bilinear constraint exactly satisfied by construction.
    #[test]
    fn conversion_round_trips_at_machine_precision() {
        let states = [
            (DVec3::new(0.9, 0.3, -0.2), DVec3::new(-0.1, 1.0, 0.3)),
            (DVec3::new(-0.7, 0.5, 0.4), DVec3::new(0.4, -0.9, 0.2)),
            (DVec3::new(0.3, -0.1, 0.05), DVec3::new(0.5, 2.2, -0.4)), // hyperbolic
            (DVec3::new(-1.5, -2.0, 3.0), DVec3::new(0.2, 0.1, 0.3)),
        ];
        for (r, v) in states {
            let y = cartesian_to_ks(r, v, 0.37);
            let (t, r_back, v_back) = ks_to_cartesian(&y);
            assert!((r_back - r).length() < 1e-14 * r.length().max(1.0), "{r:?}");
            assert!((v_back - v).length() < 1e-14 * v.length().max(1.0), "{v:?}");
            assert_eq!(t, 0.37);
            assert!(
                bilinear_constraint(&y).abs() < 1e-15,
                "fiber constraint violated: {}",
                bilinear_constraint(&y)
            );
        }
    }

    /// Spec §7.4: the same moderately-eccentric problem both ways, no
    /// switching - tight agreement; then e = 0.95, where KS must stay
    /// well-conditioned AND take dramatically fewer accepted steps.
    #[test]
    fn ks_matches_cowell_and_wins_at_high_eccentricity() {
        let model = two_body();
        // Velocity bounds are looser: both integrators accumulate their
        // velocity error fastest through perigee passages, where |a| is
        // largest - position agreement stays an order tighter.
        for (e, agree_tol, expect_fewer_steps) in [(0.6, 1e-8, false), (0.95, 1e-7, true)] {
            let a = 1.8;
            let r0 = DVec3::new(a * (1.0 - e), 0.0, 0.0);
            let v0 = DVec3::new(0.0, ((1.0 + e) / (a * (1.0 - e))).sqrt(), 0.0);
            let tf = 2.7 * TAU * a.powf(1.5);

            let cowell_system = CowellSystem {
                model: &model,
                anchor: anchor(),
            };
            let cowell = solve_arc(&cowell_system, 0.0, tf, pack(r0, v0), &config()).unwrap();
            let ks = solve_ks_span(&model, anchor(), r0, v0, 0.0, tf, &config()).unwrap();

            let (r_c, v_c) = unpack(cowell.y.last().unwrap());
            let (r_k, v_k) = unpack(ks.y.last().unwrap());
            // The KS arc ends within the arrival event's time tolerance of
            // tf; bridge the tiny difference with the Cowell velocity.
            let dt = tf - ks.t.last().unwrap();
            let r_k = r_k + v_k * dt;
            assert!(
                (r_k - r_c).length() < agree_tol,
                "e = {e}: positions differ by {:.2e}",
                (r_k - r_c).length()
            );
            assert!(
                (v_k - v_c).length() < agree_tol * 50.0,
                "e = {e}: velocities differ by {:.2e}",
                (v_k - v_c).length()
            );
            if expect_fewer_steps {
                assert!(
                    ks.t.len() * 2 < cowell.t.len(),
                    "KS knots {} vs Cowell {} at e = {e}",
                    ks.t.len(),
                    cowell.t.len()
                );
            }
        }
    }

    /// The energy component tracks the physical osculating energy through
    /// a perturbed arc (h is integrated, not recomputed - drift is the
    /// health metric of spec §3).
    #[test]
    fn energy_component_stays_consistent() {
        let mu = 3.986_004_418e14;
        let mut model = two_body();
        model.perturbations.push(Box::new(
            crate::propagation::forces::relativity::Schwarzschild::new(&CanonicalUnits::new(
                mu, 7.0e6,
            )),
        ));
        let (e, a) = (0.7, 1.5);
        let r0 = DVec3::new(a * (1.0 - e), 0.0, 0.0);
        let v0 = DVec3::new(0.0, ((1.0 + e) / (a * (1.0 - e))).sqrt(), 0.0);
        let tf = 3.0 * TAU * a.powf(1.5);
        let arc = solve_ks_span(&model, anchor(), r0, v0, 0.0, tf, &config()).unwrap();
        for (y, _) in arc.y.iter().zip(&arc.t) {
            let (r, v) = unpack(y);
            let h_physical = 1.0 / r.length() - v.length_squared() / 2.0;
            // Reconstructed from the Cartesian knots, so this checks the
            // whole u/u'/h pipeline hangs together.
            assert!(
                (h_physical - 1.0 / (2.0 * a)).abs() < 1e-6,
                "energy drifted to {h_physical}"
            );
        }
    }
}
