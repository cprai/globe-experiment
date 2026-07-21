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

/// Result of an event-terminated solve: the captured arc, plus the
/// crossing time if a boundary stopped the integration before `tf`.
pub(crate) struct EventOutcome<const N: usize> {
    pub arc: RawArc<N>,
    pub event_t: Option<f64>,
}

/// Bridges a plain boundary closure onto the solver's `Event` trait:
/// terminal on the first sign change, either direction (the caller loops
/// solve -> record -> restart per spec §5: detect, stop, restart - never
/// integrate through a discontinuity).
struct BoundaryAdapter<'a, const N: usize, F: Fn(f64, &SVector<f64, N>) -> f64> {
    boundary: &'a F,
}

impl<const N: usize, F: Fn(f64, &SVector<f64, N>) -> f64> Event<f64, SVector<f64, N>>
    for BoundaryAdapter<'_, N, F>
{
    fn config(&self) -> EventConfig {
        EventConfig::default().terminal()
    }

    fn event(&self, t: f64, y: &SVector<f64, N>) -> f64 {
        (self.boundary)(t, y)
    }
}

/// Like [`solve_arc`], but stops at the first sign change of EITHER
/// boundary function (evaluated on the TRUE time axis, both directions
/// supported). Two separate functions are required by the shadow model -
/// see `SolarRadiationPressure::boundary_functions`. When an event fires,
/// the arc's final knot is exactly the crossing state.
///
/// `first_step` bounds the solver's opening step: the event solout cannot
/// detect a crossing inside the very first step of a solve (its previous
/// sample initializes there), so restart-heavy callers pass a step smaller
/// than the narrowest feature between boundaries.
#[allow(clippy::too_many_arguments)] // crate-internal solver plumbing
pub(crate) fn solve_arc_until<const N: usize, D, F1, F2>(
    dynamics: &D,
    t0: f64,
    tf: f64,
    y0: SVector<f64, N>,
    config: &SolveConfig,
    first_step: Option<f64>,
    outer: &F1,
    inner: &F2,
) -> Result<EventOutcome<N>, String>
where
    D: Dynamics<N>,
    F1: Fn(f64, &SVector<f64, N>) -> f64,
    F2: Fn(f64, &SVector<f64, N>) -> f64,
{
    if t0 == tf {
        let ydot = dynamics.derivative(t0, &y0)?;
        return Ok(EventOutcome {
            arc: RawArc {
                t: vec![t0],
                y: vec![y0],
                ydot: vec![ydot],
            },
            event_t: None,
        });
    }
    if tf < t0 {
        let reversed = TimeReversed {
            inner: dynamics,
            t0,
        };
        let reversed_outer = |tau: f64, y: &SVector<f64, N>| outer(t0 - tau, y);
        let reversed_inner = |tau: f64, y: &SVector<f64, N>| inner(t0 - tau, y);
        let mut outcome = solve_forward_until(
            &reversed,
            0.0,
            t0 - tf,
            y0,
            config,
            first_step,
            &reversed_outer,
            &reversed_inner,
        )?;
        for t in &mut outcome.arc.t {
            *t = t0 - *t;
        }
        outcome.arc.t.reverse();
        outcome.arc.y.reverse();
        outcome.arc.ydot = derivatives_at(dynamics, &outcome.arc.t, &outcome.arc.y)?;
        outcome.event_t = outcome.event_t.map(|tau| t0 - tau);
        return Ok(outcome);
    }
    solve_forward_until(dynamics, t0, tf, y0, config, first_step, outer, inner)
}

#[allow(clippy::too_many_arguments)] // crate-internal, mirrored by the public wrapper above
fn solve_forward_until<const N: usize, D, F1, F2>(
    dynamics: &D,
    t0: f64,
    tf: f64,
    y0: SVector<f64, N>,
    config: &SolveConfig,
    first_step: Option<f64>,
    outer: &F1,
    inner: &F2,
) -> Result<EventOutcome<N>, String>
where
    D: Dynamics<N>,
    F1: Fn(f64, &SVector<f64, N>) -> f64,
    F2: Fn(f64, &SVector<f64, N>) -> f64,
{
    let adapter = Adapter {
        dynamics,
        failure: RefCell::new(None),
    };
    let outer_event = BoundaryAdapter { boundary: outer };
    let inner_event = BoundaryAdapter { boundary: inner };
    let mut method = ExplicitRungeKutta::dop853()
        .rtol(config.rtol)
        .atol(config.atol);
    if let Some(h0) = first_step {
        method = method.h0(h0);
    }
    let solution = IVP::ode(&adapter, t0, tf, y0)
        .method(method)
        .dense(config.dense_points_per_step)
        .event(&outer_event)
        .event(&inner_event)
        .solve()
        .map_err(|error| format!("DOP853: {error:?}"))?;
    if let Some(failure) = adapter.failure.into_inner() {
        return Err(failure);
    }

    let interrupted = matches!(solution.status, Status::Interrupted);
    let (mut t, mut y) = (solution.t, solution.y);
    let event_t = if interrupted {
        // The dense solout ran FIRST on the terminating step, so interior
        // knots past the crossing precede the appended event point: drop
        // them and keep the event knot last.
        let te = *t.last().unwrap();
        let mut ye = *y.last().unwrap();
        t.pop();
        y.pop();
        while t.last().is_some_and(|&knot| knot >= te) {
            t.pop();
            y.pop();
        }
        // The appended event state came from the solver's interpolant,
        // whose off-midpoint error is the crate flaw documented on
        // `dense_points_per_step` - tens of meters at loose tolerances.
        // The caller RESTARTS from this state, so re-derive it properly:
        // a plain mini-solve from the last trustworthy knot to te. (The
        // event TIME keeps the interpolant's ~ms-level skew; boundary
        // epochs are telemetry, not dynamics.)
        if let (Some(&tp), Some(&yp)) = (t.last(), y.last())
            && tp < te
        {
            let refined = solve_forward(dynamics, tp, te, yp, config)?;
            ye = *refined.y.last().unwrap();
        }
        t.push(te);
        y.push(ye);
        Some(te)
    } else {
        None
    };
    let ydot = derivatives_at(dynamics, &t, &y)?;
    Ok(EventOutcome {
        arc: RawArc { t, y, ydot },
        event_t,
    })
}
