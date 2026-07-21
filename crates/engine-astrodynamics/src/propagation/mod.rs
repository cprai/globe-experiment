//! Numerical orbit propagation around Terra over the crate's own
//! deep-space propagator core (spec Rev C): Cowell's method on DOP853 in
//! geocentric canonical units, EGM2008 harmonics with degree-2 solid
//! tides, Sun/Moon third-body gravity in Battin's cancellation-safe form,
//! and the Schwarzschild relativistic correction - behind the same thin
//! facade the satkit era exposed (glam state vectors, GCRF meters).

mod bodies;
mod forces;
mod formulation;
mod integrator;
mod spacecraft;
mod trajectory;
mod units;

pub use spacecraft::SpacecraftModel;

use anise::constants::frames::{EARTH_J2000, MOON_J2000, SUN_J2000};
use glam::DVec3;
use hifitime::Epoch;

use crate::data::context;
use crate::ephemeris::Body;
use bodies::CentralBody;
use forces::central::CentralGravity;
use forces::relativity::Schwarzschild;
use forces::third_body::ThirdBodyGravity;
use forces::{DynamicsModel, ForceModel};
use formulation::cowell::{CowellSystem, pack};
use integrator::{SolveConfig, solve_arc};
use trajectory::{Segment, Trajectory};
use units::CanonicalUnits;

/// A GCRF state vector: position in meters, velocity in m/s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitState {
    pub pos_gcrf_m: DVec3,
    pub vel_gcrf_m_s: DVec3,
}

/// Force-model and integrator knobs. Defaults mirror the satkit-era
/// surface the engine consumes; the spec's validation configurations set
/// their own tighter tolerances.
#[derive(Clone, Debug)]
pub struct Settings {
    /// Spherical-harmonic gravity degree/order (EGM2008, up to 360).
    /// Below degree 2 the field degrades to point-mass.
    pub gravity_degree: u16,
    pub gravity_order: u16,
    /// Adaptive-integrator error tolerances (DOP853 abs/rel).
    pub abs_error: f64,
    pub rel_error: f64,
    pub use_sun_gravity: bool,
    pub use_moon_gravity: bool,
    pub use_relativistic_correction: bool,
    /// Physical spacecraft parameters for the non-gravitational forces.
    /// `None` (the default) skips SRP/drag/albedo entirely - today's
    /// parameter-less behavior. The forces themselves land at P5/P6; the
    /// field is part of the surface now so the API is final.
    pub spacecraft: Option<SpacecraftModel>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            gravity_degree: 4,
            gravity_order: 4,
            abs_error: 1e-8,
            rel_error: 1e-8,
            use_sun_gravity: true,
            use_moon_gravity: true,
            use_relativistic_correction: true,
            spacecraft: None,
        }
    }
}

/// Propagation failure (integrator abort, ephemeris/rotation coverage, or
/// a time outside the dense span on interpolation).
#[derive(Debug)]
pub struct PropagationError(String);

impl std::fmt::Display for PropagationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "propagation failed: {}", self.0)
    }
}

impl std::error::Error for PropagationError {}

pub type Result<T> = core::result::Result<T, PropagationError>;

/// Dense output of one [`propagate`] call over `[begin, end]`.
pub struct Propagation {
    trajectory: Trajectory,
    end_state: OrbitState,
    begin: Epoch,
    end: Epoch,
}

impl Propagation {
    pub fn time_begin(&self) -> Epoch {
        self.begin
    }

    pub fn time_end(&self) -> Epoch {
        self.end
    }

    pub fn state_end(&self) -> OrbitState {
        self.end_state
    }

    /// Interpolates the dense output at `epoch` (within the propagated span).
    pub fn interp(&self, epoch: Epoch) -> Result<OrbitState> {
        let (pos_gcrf_m, vel_gcrf_m_s) =
            self.trajectory.state_at(epoch).map_err(PropagationError)?;
        Ok(OrbitState {
            pos_gcrf_m,
            vel_gcrf_m_s,
        })
    }

    /// One dense-output interpolation per instant - use this over an
    /// [`interp`](Self::interp) loop.
    pub fn interp_batch(&self, epochs: &[Epoch]) -> Result<Vec<OrbitState>> {
        epochs.iter().map(|&epoch| self.interp(epoch)).collect()
    }
}

/// Numerically propagates `state` from `begin` to `end` (either
/// direction): EGM2008 + solid tides + optional Sun/Moon third-body +
/// relativity; adaptive dense-output integrator. The returned
/// [`Propagation`] carries the end state and interpolation over the span.
pub fn propagate(
    state: &OrbitState,
    begin: Epoch,
    end: Epoch,
    settings: &Settings,
) -> Result<Propagation> {
    let almanac = &context().almanac;
    let mu_of = |frame| -> Result<f64> {
        let info = almanac.frame_info(frame).map_err(err)?;
        Ok(info.mu_km3_s2().map_err(err)? * 1e9)
    };
    let earth = almanac.frame_info(EARTH_J2000).map_err(err)?;
    let central = CentralBody {
        naif_id: 399,
        mu_m3_s2: earth.mu_km3_s2().map_err(err)? * 1e9,
        reference_radius_m: earth.mean_equatorial_radius_km().map_err(err)? * 1e3,
    };
    let units = CanonicalUnits::new(central.mu_m3_s2, central.reference_radius_m);

    let mut perturbations: Vec<Box<dyn ForceModel>> = Vec::new();
    if settings.use_sun_gravity {
        perturbations.push(Box::new(ThirdBodyGravity {
            body: Body::Sol,
            mu_m3_s2: mu_of(SUN_J2000)?,
        }));
    }
    if settings.use_moon_gravity {
        perturbations.push(Box::new(ThirdBodyGravity {
            body: Body::Luna,
            mu_m3_s2: mu_of(MOON_J2000)?,
        }));
    }
    if settings.use_relativistic_correction {
        perturbations.push(Box::new(Schwarzschild::new(&units)));
    }
    let model = DynamicsModel {
        units,
        central: CentralGravity {
            field: forces::harmonics::field_for(
                &central,
                settings.gravity_degree,
                settings.gravity_order,
                true, // degree-2 solid tides, spec §4.1 - in scope, always on
            ),
        },
        perturbations,
    };

    let system = CowellSystem {
        model: &model,
        anchor: begin,
    };
    let y0 = pack(
        units.length_to_can(state.pos_gcrf_m),
        units.velocity_to_can(state.vel_gcrf_m_s),
    );
    let tf_can = units.time_to_can((end - begin).to_seconds());
    let config = SolveConfig {
        rtol: settings.rel_error,
        atol: settings.abs_error,
        dense_points_per_step: 2,
    };
    let arc = solve_arc(&system, 0.0, tf_can, y0, &config).map_err(PropagationError)?;

    let trajectory = Trajectory::new(Segment::from_arc(begin, units, &arc));
    let (end_pos, end_vel) = trajectory.end_state(end >= begin);
    Ok(Propagation {
        trajectory,
        end_state: OrbitState {
            pos_gcrf_m: end_pos,
            vel_gcrf_m_s: end_vel,
        },
        begin,
        end,
    })
}

fn err<E: std::fmt::Display>(error: E) -> PropagationError {
    PropagationError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Duration;
    use crate::init;

    /// Terra's GM (m^3/s^2) - only to construct the test state's
    /// circular speed; the propagator brings its own force model.
    const MU_M3_S2: f64 = 3.986004418e14;
    /// Test orbit radius, meters (~400 km altitude LEO).
    const RADIUS_M: f64 = 6_778_000.0;

    fn circular_leo() -> (OrbitState, Epoch) {
        let speed = (MU_M3_S2 / RADIUS_M).sqrt();
        let state = OrbitState {
            pos_gcrf_m: DVec3::new(RADIUS_M, 0.0, 0.0),
            vel_gcrf_m_s: DVec3::new(0.0, speed, 0.0),
        };
        let time = Epoch::from_gregorian_utc(2024, 1, 1, 12, 0, 0, 0);
        (state, time)
    }

    /// Ten minutes of propagation must hold a circular LEO's radius and
    /// speed (loose km-scale tolerances absorb the full force model vs the
    /// two-body construction) while moving well along track.
    #[test]
    fn propagation_holds_circular_leo() {
        init();
        let (state, t0) = circular_leo();
        let t1 = t0 + Duration::from_seconds(600.0);

        let end = propagate(&state, t0, t1, &Settings::default())
            .expect("LEO propagation")
            .state_end();
        assert!(
            (end.pos_gcrf_m.length() - RADIUS_M).abs() < 30_000.0,
            "radius drifted to {:.1} km",
            end.pos_gcrf_m.length() / 1000.0
        );
        assert!(
            (end.vel_gcrf_m_s.length() - state.vel_gcrf_m_s.length()).abs() < 50.0,
            "speed drifted to {:.1} m/s",
            end.vel_gcrf_m_s.length()
        );
        assert!(
            (end.pos_gcrf_m - state.pos_gcrf_m).length() > 1_000_000.0,
            "propagation should move well along track in 600 s"
        );
    }

    /// Dense-output sampling: every interpolated instant stays on the orbit
    /// radius, and the span endpoints match the propagation's own states.
    #[test]
    fn dense_output_interpolates_within_span() {
        init();
        let (state, t0) = circular_leo();
        let t1 = t0 + Duration::from_seconds(600.0);
        let result = propagate(&state, t0, t1, &Settings::default()).expect("LEO propagation");

        let times: Vec<Epoch> = (0..=10)
            .map(|i| t0 + Duration::from_seconds(60.0 * f64::from(i)))
            .collect();
        let samples = result.interp_batch(&times).expect("dense sampling");
        assert_eq!(samples.len(), times.len());
        for sample in &samples {
            assert!(
                (sample.pos_gcrf_m.length() - RADIUS_M).abs() < 30_000.0,
                "sample off the orbit radius"
            );
        }
        assert!((samples[0].pos_gcrf_m - state.pos_gcrf_m).length() < 1.0);
        let single = result.interp(t1).expect("single interpolation");
        assert!((single.pos_gcrf_m - result.state_end().pos_gcrf_m).length() < 1.0);
    }

    /// A zero-duration propagation returns the initial state unchanged.
    #[test]
    fn zero_duration_returns_initial_state() {
        init();
        let (state, t0) = circular_leo();
        let end = propagate(&state, t0, t0, &Settings::default())
            .expect("zero-duration propagation")
            .state_end();
        assert_eq!(end, state);
    }

    /// Backward spans are first-class: propagate back, then forward again,
    /// and land on the starting state to sub-meter agreement.
    #[test]
    fn backward_span_round_trips() {
        init();
        let (state, t0) = circular_leo();
        let t_back = t0 - Duration::from_seconds(1800.0);
        let back = propagate(&state, t0, t_back, &Settings::default())
            .expect("backward propagation")
            .state_end();
        let forward = propagate(&back, t_back, t0, &Settings::default())
            .expect("forward propagation")
            .state_end();
        assert!(
            (forward.pos_gcrf_m - state.pos_gcrf_m).length() < 1.0,
            "round trip drifted {} m",
            (forward.pos_gcrf_m - state.pos_gcrf_m).length()
        );
    }
}
