//! Numerical orbit propagation around Terra, delegating to satkit's
//! `orbitprop` behind a crate-owned API (glam state vectors, GCRF meters).
//! All propagation requires [`crate::init`] first.

use glam::DVec3;
use satkit::Instant;
use satkit::orbitprop::{self, PropSettings, PropagationResult, SimpleState};

/// A GCRF state vector: position in meters, velocity in m/s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitState {
    pub pos_gcrf_m: DVec3,
    pub vel_gcrf_m_s: DVec3,
}

/// Force-model and integrator knobs, mirroring the satkit `PropSettings`
/// surface this crate sanctions. Space weather (drag/SRP) is deliberately
/// not exposed: its data is not embedded, and enabling it would reach
/// satkit's runtime data loader.
#[derive(Clone, Debug)]
pub struct Settings {
    /// Spherical-harmonic gravity degree/order (EGM96).
    pub gravity_degree: u16,
    pub gravity_order: u16,
    /// Adaptive-integrator error tolerances.
    pub abs_error: f64,
    pub rel_error: f64,
    pub use_sun_gravity: bool,
    pub use_moon_gravity: bool,
    pub use_relativistic_correction: bool,
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
        }
    }
}

impl Settings {
    fn to_satkit(&self) -> PropSettings {
        PropSettings {
            gravity_degree: self.gravity_degree,
            gravity_order: self.gravity_order,
            abs_error: self.abs_error,
            rel_error: self.rel_error,
            use_sun_gravity: self.use_sun_gravity,
            use_moon_gravity: self.use_moon_gravity,
            use_relativistic_correction: self.use_relativistic_correction,
            // Keeps satkit's non-embedded space-weather loader unreachable;
            // drag/SRP additionally need the satprops we never pass.
            use_spaceweather: false,
            ..PropSettings::default()
        }
    }
}

/// Propagation failure (integrator abort, or a time outside the dense span
/// on interpolation).
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
    result: PropagationResult<1>,
}

impl Propagation {
    pub fn time_begin(&self) -> Instant {
        self.result.time_begin
    }

    pub fn time_end(&self) -> Instant {
        self.result.time_end
    }

    pub fn state_end(&self) -> OrbitState {
        unpack(&self.result.state_end)
    }

    /// Interpolates the dense output at `time` (within the propagated span).
    pub fn interp(&self, time: &Instant) -> Result<OrbitState> {
        self.result
            .interp(time)
            .map(|packed| unpack(&packed))
            .map_err(err)
    }

    /// One dense-output interpolation per instant - use this over an
    /// [`interp`](Self::interp) loop.
    pub fn interp_batch(&self, times: &[Instant]) -> Result<Vec<OrbitState>> {
        self.result
            .interp_batch(times)
            .map(|samples| samples.iter().map(unpack).collect())
            .map_err(err)
    }
}

/// Numerically propagates `state` from `begin` to `end` (EGM96 + optional
/// Sun/Moon third-body + relativity; adaptive dense-output integrator).
/// The returned [`Propagation`] carries the end state and interpolation
/// over the full span.
pub fn propagate(
    state: &OrbitState,
    begin: &Instant,
    end: &Instant,
    settings: &Settings,
) -> Result<Propagation> {
    orbitprop::propagate(&pack(state), begin, end, &settings.to_satkit(), None)
        .map(|result| Propagation { result })
        .map_err(err)
}

/// The state packed as satkit's 6-vector integrator state.
fn pack(state: &OrbitState) -> SimpleState {
    let mut packed = SimpleState::zeros();
    packed[0] = state.pos_gcrf_m.x;
    packed[1] = state.pos_gcrf_m.y;
    packed[2] = state.pos_gcrf_m.z;
    packed[3] = state.vel_gcrf_m_s.x;
    packed[4] = state.vel_gcrf_m_s.y;
    packed[5] = state.vel_gcrf_m_s.z;
    packed
}

fn unpack(packed: &SimpleState) -> OrbitState {
    OrbitState {
        pos_gcrf_m: DVec3::new(packed[0], packed[1], packed[2]),
        vel_gcrf_m_s: DVec3::new(packed[3], packed[4], packed[5]),
    }
}

fn err(error: orbitprop::Error) -> PropagationError {
    PropagationError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init;
    use satkit::Duration;

    /// Terra's GM (EGM96, m^3/s^2) - only to construct the test state's
    /// circular speed; the propagator brings its own force model.
    const MU_M3_S2: f64 = 3.986004418e14;
    /// Test orbit radius, meters (~400 km altitude LEO).
    const RADIUS_M: f64 = 6_778_000.0;

    fn circular_leo() -> (OrbitState, Instant) {
        let speed = (MU_M3_S2 / RADIUS_M).sqrt();
        let state = OrbitState {
            pos_gcrf_m: DVec3::new(RADIUS_M, 0.0, 0.0),
            vel_gcrf_m_s: DVec3::new(0.0, speed, 0.0),
        };
        let time = Instant::from_datetime(2024, 1, 1, 12, 0, 0.0).expect("valid test epoch");
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

        let end = propagate(&state, &t0, &t1, &Settings::default())
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
        let result = propagate(&state, &t0, &t1, &Settings::default()).expect("LEO propagation");

        let times: Vec<Instant> = (0..=10)
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
        let single = result.interp(&t1).expect("single interpolation");
        assert!((single.pos_gcrf_m - result.state_end().pos_gcrf_m).length() < 1.0);
    }

    /// A zero-duration propagation returns the initial state unchanged.
    #[test]
    fn zero_duration_returns_initial_state() {
        init();
        let (state, t0) = circular_leo();
        let end = propagate(&state, &t0, &t0, &Settings::default())
            .expect("zero-duration propagation")
            .state_end();
        assert_eq!(end, state);
    }
}
