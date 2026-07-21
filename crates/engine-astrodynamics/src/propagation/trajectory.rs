//! The crate-owned dense layer (plan §5): `differential-equations` retains
//! no interpolant after `solve()`, so knots captured during the solve
//! (state + derivative at the solver's dense-output points) are
//! re-interpolated here - quintic Hermite for position (r, v, a at both
//! ends), cubic Hermite for velocity (v, a). A [`Trajectory`] stitches
//! segments and answers arbitrary-epoch queries; it carries one segment
//! until the event/switching machinery lands (P5/P7).

use glam::DVec3;
use hifitime::Epoch;

use super::integrator::RawArc;
use super::units::CanonicalUnits;

/// Span slack for endpoint queries, canonical time (~1 ms geocentric):
/// interpolating at exactly `end` must never fail to float roundoff.
const SPAN_SLACK_CAN: f64 = 1e-6;

/// One integration arc: anchor epoch, unit set, and knots ascending in
/// canonical time offset from the anchor.
pub(crate) struct Segment {
    pub anchor: Epoch,
    pub units: CanonicalUnits,
    t: Vec<f64>,
    r: Vec<DVec3>,
    v: Vec<DVec3>,
    a: Vec<DVec3>,
}

impl Segment {
    pub(crate) fn from_arc(anchor: Epoch, units: CanonicalUnits, arc: &RawArc<6>) -> Self {
        let split =
            |y: &nalgebra::SVector<f64, 6>, at: usize| DVec3::new(y[at], y[at + 1], y[at + 2]);
        Self {
            anchor,
            units,
            t: arc.t.clone(),
            r: arc.y.iter().map(|y| split(y, 0)).collect(),
            v: arc.y.iter().map(|y| split(y, 3)).collect(),
            a: arc.ydot.iter().map(|y| split(y, 3)).collect(),
        }
    }

    fn state_at_can(&self, t_can: f64) -> Result<(DVec3, DVec3), String> {
        let (first, last) = (*self.t.first().unwrap(), *self.t.last().unwrap());
        if t_can < first - SPAN_SLACK_CAN || t_can > last + SPAN_SLACK_CAN {
            return Err(format!(
                "epoch outside the propagated span ({t_can} not in [{first}, {last}] canonical)"
            ));
        }
        let t_can = t_can.clamp(first, last);
        if self.t.len() == 1 {
            return Ok((self.r[0], self.v[0]));
        }
        // Knot interval containing t_can: i is the last index with
        // t[i] <= t_can (capped so [i, i+1] stays in range).
        let i = self
            .t
            .partition_point(|&knot| knot <= t_can)
            .saturating_sub(1)
            .min(self.t.len() - 2);
        let h = self.t[i + 1] - self.t[i];
        let tau = (t_can - self.t[i]) / h;

        // Quintic Hermite position from (r, v, a) at both ends.
        let (t2, t3) = (tau * tau, tau * tau * tau);
        let (t4, t5) = (t3 * tau, t3 * tau * tau);
        let h0 = 1.0 - 10.0 * t3 + 15.0 * t4 - 6.0 * t5;
        let h1 = tau - 6.0 * t3 + 8.0 * t4 - 3.0 * t5;
        let h2 = 0.5 * t2 - 1.5 * t3 + 1.5 * t4 - 0.5 * t5;
        let h3 = 10.0 * t3 - 15.0 * t4 + 6.0 * t5;
        let h4 = -4.0 * t3 + 7.0 * t4 - 3.0 * t5;
        let h5 = 0.5 * t3 - t4 + 0.5 * t5;
        let position = h0 * self.r[i]
            + h1 * h * self.v[i]
            + h2 * h * h * self.a[i]
            + h3 * self.r[i + 1]
            + h4 * h * self.v[i + 1]
            + h5 * h * h * self.a[i + 1];

        // Cubic Hermite velocity from (v, a) at both ends.
        let c00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let c10 = t3 - 2.0 * t2 + tau;
        let c01 = -2.0 * t3 + 3.0 * t2;
        let c11 = t3 - t2;
        let velocity =
            c00 * self.v[i] + c10 * h * self.a[i] + c01 * self.v[i + 1] + c11 * h * self.a[i + 1];

        Ok((position, velocity))
    }

    /// The exact solver state at one end of the span (no interpolation).
    fn end_knot(&self, at_last: bool) -> (DVec3, DVec3) {
        let i = if at_last { self.t.len() - 1 } else { 0 };
        (self.r[i], self.v[i])
    }
}

/// A propagated trajectory: arbitrary-epoch state queries in SI units.
pub(crate) struct Trajectory {
    segment: Segment,
}

impl Trajectory {
    pub(crate) fn new(segment: Segment) -> Self {
        Self { segment }
    }

    /// GCRF (position m, velocity m/s) at `epoch`, which must lie within
    /// the propagated span.
    pub(crate) fn state_at(&self, epoch: Epoch) -> Result<(DVec3, DVec3), String> {
        let segment = &self.segment;
        let t_can = segment
            .units
            .time_to_can((epoch - segment.anchor).to_seconds());
        let (r_can, v_can) = segment.state_at_can(t_can)?;
        Ok((
            segment.units.length_to_m(r_can),
            segment.units.velocity_to_m_s(v_can),
        ))
    }

    /// The exact solver end state (SI) - `at_last` false for backward
    /// spans, where the requested end sits at the FIRST ascending knot.
    pub(crate) fn end_state(&self, at_last: bool) -> (DVec3, DVec3) {
        let (r_can, v_can) = self.segment.end_knot(at_last);
        (
            self.segment.units.length_to_m(r_can),
            self.segment.units.velocity_to_m_s(v_can),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::propagation::bodies::PointMass;
    use crate::propagation::forces::DynamicsModel;
    use crate::propagation::forces::central::CentralGravity;
    use crate::propagation::formulation::cowell::{CowellSystem, pack};
    use crate::propagation::integrator::{SolveConfig, solve_arc};

    const TAU: f64 = std::f64::consts::TAU;

    /// The plan-§5 fidelity gate, referenced against the ANALYTIC two-body
    /// closed form (not a `t_eval` solve: differential-equations 0.6.1's
    /// DOP853 interpolant is only accurate at step midpoints - measured
    /// ~0.87 m at quarter-step points vs ~5e-4 m at midpoints at rtol
    /// 1e-12 - so a t_eval reference would share the very error being
    /// gated; that finding is also why every SolveConfig in this crate
    /// uses dense_points_per_step = 2, i.e. endpoints + midpoints only).
    #[test]
    fn interpolation_matches_closed_form() {
        let mu = 3.986_004_418e14;
        let units = CanonicalUnits::new(mu, 6.678e6);
        let model = DynamicsModel {
            units,
            central: CentralGravity {
                field: Box::new(PointMass {
                    mu_m3_s2: mu,
                    reference_radius_m: 6.378e6,
                }),
            },
            perturbations: Vec::new(),
        };
        let anchor = Epoch::from_gregorian_utc(2020, 1, 1, 0, 0, 0, 0);
        let system = CowellSystem {
            model: &model,
            anchor,
        };
        let (a, e) = (1.2, 0.4);
        let y0 = pack(
            DVec3::new(a * (1.0 - e), 0.0, 0.0),
            DVec3::new(0.0, ((1.0 + e) / (a * (1.0 - e))).sqrt(), 0.0),
        );
        let period = TAU * a.powf(1.5);
        let config = SolveConfig {
            rtol: 1e-12,
            atol: 1e-12,
            dense_points_per_step: 2,
        };
        let arc = solve_arc(&system, 0.0, period, y0, &config).unwrap();
        let trajectory = Trajectory::new(Segment::from_arc(anchor, units, &arc));

        let closed_form = |t: f64| {
            let m = t * a.powf(-1.5);
            let mut big_e = m;
            for _ in 0..64 {
                big_e -= (big_e - e * big_e.sin() - m) / (1.0 - e * big_e.cos());
            }
            DVec3::new(
                a * (big_e.cos() - e),
                a * (1.0 - e * e).sqrt() * big_e.sin(),
                0.0,
            )
        };
        let mut worst = 0.0_f64;
        for i in 0..=1000 {
            let t_can = period * f64::from(i) / 1000.0;
            let epoch = anchor + crate::Duration::from_seconds(units.time_to_s(t_can));
            let (r_m, _) = trajectory.state_at(epoch).unwrap();
            worst = worst.max((r_m - units.length_to_m(closed_form(t_can))).length());
        }
        // Measured ~1.3e-3 m worst - four orders under the spec-§0 target.
        assert!(
            worst < 0.005,
            "worst interpolation error {worst:.2e} m over one period"
        );
    }

    /// Queries outside the span err; queries at exactly the endpoints and
    /// within roundoff slack of them succeed.
    #[test]
    fn span_policing() {
        let mu = 3.986_004_418e14;
        let units = CanonicalUnits::new(mu, 6.678e6);
        let model = DynamicsModel {
            units,
            central: CentralGravity {
                field: Box::new(PointMass {
                    mu_m3_s2: mu,
                    reference_radius_m: 6.378e6,
                }),
            },
            perturbations: Vec::new(),
        };
        let anchor = Epoch::from_gregorian_utc(2020, 1, 1, 0, 0, 0, 0);
        let system = CowellSystem {
            model: &model,
            anchor,
        };
        let y0 = pack(DVec3::X, DVec3::Y);
        let config = SolveConfig {
            rtol: 1e-12,
            atol: 1e-12,
            dense_points_per_step: 2,
        };
        let arc = solve_arc(&system, 0.0, 1.0, y0, &config).unwrap();
        let trajectory = Trajectory::new(Segment::from_arc(anchor, units, &arc));

        let end = anchor + crate::Duration::from_seconds(units.time_to_s(1.0));
        assert!(trajectory.state_at(anchor).is_ok());
        assert!(trajectory.state_at(end).is_ok());
        let far = anchor + crate::Duration::from_seconds(units.time_to_s(2.0));
        assert!(trajectory.state_at(far).is_err());
        let before = anchor - crate::Duration::from_seconds(60.0);
        assert!(trajectory.state_at(before).is_err());
    }
}
