//! The crate's ONE window onto `differential-equations` (DOP853): every
//! `use differential_equations::...` lives here, so the pinned 0.x crate's
//! API churn (0.5 -> 0.6 renamed the core builder) stays contained in this
//! file. Callers hand in a [`Dynamics`] and get back knots; they never see
//! solver types.

use std::cell::RefCell;

use differential_equations::prelude::*;
use nalgebra::SVector;

/// The right-hand side dydt = f(t, y) in canonical units. Errs instead of
/// producing inf/NaN (spec §2's underflow guard) - a silent NaN inside
/// DOP853 would corrupt the whole trajectory before anything noticed.
pub(crate) trait Dynamics<const N: usize> {
    fn derivative(&self, t: f64, y: &SVector<f64, N>) -> Result<SVector<f64, N>, String>;
}

/// Integration output: canonical-time knots with state and derivative,
/// ordered by increasing `t` regardless of integration direction. The
/// derivative rides along so the trajectory layer can Hermite-interpolate
/// without re-deriving.
pub(crate) struct RawArc<const N: usize> {
    pub t: Vec<f64>,
    pub y: Vec<SVector<f64, N>>,
    pub ydot: Vec<SVector<f64, N>>,
}

pub(crate) struct SolveConfig {
    pub rtol: f64,
    pub atol: f64,
    /// Dense-output points per accepted step (including endpoints; the
    /// solver's interpolant supplies the interior ones). USE 2: the pinned
    /// crate's DOP853 interpolant is only accurate at step midpoints
    /// (measured ~0.87 m error at quarter-step points vs ~5e-4 m at
    /// midpoints on a 1e-12-tolerance two-body arc), so endpoints +
    /// midpoints are the only trustworthy knots. The trajectory layer's
    /// quintic Hermite reaches ~1e-3 m from those.
    pub dense_points_per_step: usize,
}

/// Bridges [`Dynamics`] onto the solver's infallible `diff`: a failure
/// poisons the solve (recorded here, zeros returned) and is surfaced after.
struct Adapter<'a, const N: usize, D: Dynamics<N>> {
    dynamics: &'a D,
    failure: RefCell<Option<String>>,
}

impl<const N: usize, D: Dynamics<N>> ODE<f64, SVector<f64, N>> for Adapter<'_, N, D> {
    fn diff(&self, t: f64, y: &SVector<f64, N>, dydt: &mut SVector<f64, N>) {
        if self.failure.borrow().is_some() {
            dydt.fill(0.0);
            return;
        }
        match self.dynamics.derivative(t, y) {
            Ok(derivative) => *dydt = derivative,
            Err(failure) => {
                *self.failure.borrow_mut() = Some(failure);
                dydt.fill(0.0);
            }
        }
    }
}

/// Time-reversal wrapper: solves `tau = t0 - t` forward so that dense
/// output stays usable - differential-equations 0.6.1's `DenseSolout`
/// rejects interior points on backward spans (its step-interval check
/// assumes forward ordering), so backward arcs are integrated as a negated
/// forward problem instead (the spec's own §7.7 fallback).
struct TimeReversed<'a, const N: usize, D: Dynamics<N>> {
    inner: &'a D,
    t0: f64,
}

impl<const N: usize, D: Dynamics<N>> Dynamics<N> for TimeReversed<'_, N, D> {
    fn derivative(&self, tau: f64, y: &SVector<f64, N>) -> Result<SVector<f64, N>, String> {
        Ok(-self.inner.derivative(self.t0 - tau, y)?)
    }
}

/// Integrates from `t0` to `tf` (either direction; DOP853, adaptive).
pub(crate) fn solve_arc<const N: usize, D: Dynamics<N>>(
    dynamics: &D,
    t0: f64,
    tf: f64,
    y0: SVector<f64, N>,
    config: &SolveConfig,
) -> Result<RawArc<N>, String> {
    if t0 == tf {
        // Degenerate span: the solver's initial-step heuristic divides by
        // the span sign; short-circuit with a single knot instead.
        let ydot = dynamics.derivative(t0, &y0)?;
        return Ok(RawArc {
            t: vec![t0],
            y: vec![y0],
            ydot: vec![ydot],
        });
    }
    if tf < t0 {
        let reversed = TimeReversed {
            inner: dynamics,
            t0,
        };
        let mut arc = solve_forward(&reversed, 0.0, t0 - tf, y0, config)?;
        for t in &mut arc.t {
            *t = t0 - *t;
        }
        arc.t.reverse();
        arc.y.reverse();
        // Re-derive ydot on the TRUE time axis (the reversed system's
        // derivative carries the wrong sign).
        arc.ydot = derivatives_at(dynamics, &arc.t, &arc.y)?;
        return Ok(arc);
    }
    solve_forward(dynamics, t0, tf, y0, config)
}

fn solve_forward<const N: usize, D: Dynamics<N>>(
    dynamics: &D,
    t0: f64,
    tf: f64,
    y0: SVector<f64, N>,
    config: &SolveConfig,
) -> Result<RawArc<N>, String> {
    let adapter = Adapter {
        dynamics,
        failure: RefCell::new(None),
    };
    let method = ExplicitRungeKutta::dop853()
        .rtol(config.rtol)
        .atol(config.atol);
    let solution = IVP::ode(&adapter, t0, tf, y0)
        .method(method)
        .dense(config.dense_points_per_step)
        .solve()
        .map_err(|error| format!("DOP853: {error:?}"))?;
    if let Some(failure) = adapter.failure.into_inner() {
        return Err(failure);
    }
    let ydot = derivatives_at(dynamics, &solution.t, &solution.y)?;
    Ok(RawArc {
        t: solution.t,
        y: solution.y,
        ydot,
    })
}

fn derivatives_at<const N: usize, D: Dynamics<N>>(
    dynamics: &D,
    t: &[f64],
    y: &[SVector<f64, N>],
) -> Result<Vec<SVector<f64, N>>, String> {
    t.iter()
        .zip(y)
        .map(|(&t, y)| dynamics.derivative(t, y))
        .collect()
}
