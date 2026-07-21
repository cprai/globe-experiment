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

use hifitime::Duration;
use nalgebra::SVector;

use crate::data::context;
use crate::ephemeris::Body;
use bodies::CentralBody;
use forces::albedo_ir::{PlanetaryRadiation, bond_albedo};
use forces::atmosphere::atmosphere_for;
use forces::central::CentralGravity;
use forces::drag::AtmosphericDrag;
use forces::relativity::Schwarzschild;
use forces::srp::{Occulter, SolarRadiationPressure};
use forces::third_body::ThirdBodyGravity;
use forces::{DynamicsModel, EvalContext, ForceModel};
use formulation::cowell::{CowellSystem, pack, unpack};
use integrator::{RawArc, SolveConfig, solve_arc, solve_arc_until};
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
    shadow_boundaries: Vec<Epoch>,
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

    /// Epochs where the arc crossed a shadow boundary (penumbra or umbra
    /// edge of any configured occulter), in integration order - boundary
    /// telemetry, recorded because the integrator stops and restarts at
    /// each crossing rather than stepping through the discontinuity.
    /// Empty without a spacecraft model (no SRP, no shadow).
    pub fn shadow_boundaries(&self) -> &[Epoch] {
        &self.shadow_boundaries
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
/// relativity, plus - with a [`SpacecraftModel`] - SRP behind the conical
/// shadow, planetary albedo/IR, and NRLMSISE-00 drag (observed space
/// weather only; epochs past the embedded snapshot's observed span fail
/// loudly). Adaptive dense-output integrator. The returned [`Propagation`]
/// carries the end state and interpolation over the span.
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
    // Non-gravitational forces exist only with physical spacecraft
    // parameters (owner decision §9-Q3b: nullable end to end).
    let srp = match settings.spacecraft {
        Some(spacecraft) => {
            let moon = almanac.frame_info(MOON_J2000).map_err(err)?;
            let srp = SolarRadiationPressure {
                spacecraft,
                // Central body always; Luna is a mandatory occulter
                // candidate for Earth orbiters (spec §4.5).
                occulters: vec![
                    Occulter::Central {
                        radius_m: central.reference_radius_m,
                    },
                    Occulter::Luna {
                        radius_m: moon.mean_equatorial_radius_km().map_err(err)? * 1e3,
                    },
                ],
            };
            perturbations.push(Box::new(srp.clone()));
            perturbations.push(Box::new(PlanetaryRadiation {
                spacecraft,
                body_radius_m: central.reference_radius_m,
                bond_albedo: bond_albedo(central.naif_id).unwrap_or(0.3),
            }));
            perturbations.push(Box::new(AtmosphericDrag {
                spacecraft,
                atmosphere: atmosphere_for(&central),
            }));
            Some(srp)
        }
        None => None,
    };
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
    let (arc, shadow_boundaries) = match &srp {
        Some(srp) => solve_with_shadow_events(&system, tf_can, y0, &config, srp, units, begin)?,
        None => (
            solve_arc(&system, 0.0, tf_can, y0, &config).map_err(PropagationError)?,
            Vec::new(),
        ),
    };

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
        shadow_boundaries,
    })
}

/// The segment driver for shadowed arcs (spec §5): integrate until a
/// shadow-boundary crossing, record it, restart exactly at the crossing
/// state, and stitch the arcs - the integrator never steps through the
/// SRP discontinuity's neighborhood unchecked.
fn solve_with_shadow_events(
    system: &CowellSystem,
    tf_can: f64,
    y0: SVector<f64, 6>,
    config: &SolveConfig,
    srp: &SolarRadiationPressure,
    units: CanonicalUnits,
    anchor: Epoch,
) -> Result<(RawArc<6>, Vec<Epoch>)> {
    // An ephemeris failure inside a boundary function reports "no
    // boundary" - the dynamics hit the same failure on the same epoch and
    // surface the real error through the solve itself.
    let boundary = |t: f64, y: &SVector<f64, 6>, pick_inner: bool| {
        let ctx = EvalContext::new(units, anchor + Duration::from_seconds(units.time_to_s(t)));
        let (r_can, _) = unpack(y);
        match srp.boundary_functions(&ctx, r_can) {
            Ok((outer, inner)) => {
                if pick_inner {
                    inner
                } else {
                    outer
                }
            }
            Err(_) => 1.0,
        }
    };
    let outer = |t: f64, y: &SVector<f64, 6>| boundary(t, y, false);
    let inner = |t: f64, y: &SVector<f64, 6>| boundary(t, y, true);
    // The event solout is blind inside a solve's first step; keep restarts'
    // opening step (~1 s) below the ~10 s LEO penumbra transit so the next
    // edge cannot hide in it.
    let restart_step_can = units.time_to_can(1.0).min(tf_can.abs() / 2.0);
    // After stopping ON a root, the detector would re-trigger on it
    // immediately; a short plain solve carries the state a guard interval
    // past the crossing first (physically exact - it is an ordinary
    // integration; 20 ms cannot hide a shadow edge, the narrowest real
    // feature being the ~10 s penumbra transit).
    let guard_can = units.time_to_can(0.02);

    // LEO worst case is ~4 boundaries per 90-minute orbit; this cap only
    // exists to turn pathological chatter into an error instead of a hang.
    const MAX_BOUNDARIES: usize = 10_000;
    let forward = tf_can >= 0.0;
    let chronological_end = |arc: &RawArc<6>| {
        if forward {
            *arc.y.last().unwrap()
        } else {
            *arc.y.first().unwrap()
        }
    };
    let mut t = 0.0;
    let mut y = y0;
    let mut first_step = None;
    let mut arcs: Vec<RawArc<6>> = Vec::new();
    let mut boundaries = Vec::new();
    loop {
        let outcome = solve_arc_until(system, t, tf_can, y, config, first_step, &outer, &inner)
            .map_err(PropagationError)?;
        // The crossing state sits at the arc's chronological event end:
        // last knot going forward, first going backward (arcs ascend).
        let crossing = chronological_end(&outcome.arc);
        arcs.push(outcome.arc);
        match outcome.event_t {
            Some(event_t) => {
                if boundaries.len() >= MAX_BOUNDARIES {
                    return Err(PropagationError(format!(
                        "shadow-boundary chatter: {MAX_BOUNDARIES} crossings in one arc"
                    )));
                }
                boundaries.push(anchor + Duration::from_seconds(units.time_to_s(event_t)));
                // Guard hop past the located root (see guard_can above).
                let guard_end = if forward {
                    (event_t + guard_can).min(tf_can)
                } else {
                    (event_t - guard_can).max(tf_can)
                };
                let hop = solve_arc(system, event_t, guard_end, crossing, config)
                    .map_err(PropagationError)?;
                y = chronological_end(&hop);
                arcs.push(hop);
                t = guard_end;
                if t == tf_can {
                    break;
                }
                first_step = Some(restart_step_can);
            }
            None => break,
        }
    }

    // Stitch ascending: backward runs produced later-time arcs first.
    if !forward {
        arcs.reverse();
    }
    let mut merged = RawArc {
        t: Vec::new(),
        y: Vec::new(),
        ydot: Vec::new(),
    };
    for (i, arc) in arcs.into_iter().enumerate() {
        let skip = usize::from(i > 0); // junction knot duplicates the seam
        merged.t.extend(arc.t.into_iter().skip(skip));
        merged.y.extend(arc.y.into_iter().skip(skip));
        merged.ydot.extend(arc.ydot.into_iter().skip(skip));
    }
    Ok((merged, boundaries))
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

    /// A spacecraft model activates the non-gravitational forces (SRP +
    /// albedo/IR + drag): over a day at 400 km they must displace the arc
    /// by a physically plausible amount versus the parameter-less run.
    #[test]
    fn srp_displaces_a_day_long_arc() {
        init();
        let (state, t0) = circular_leo();
        let t1 = t0 + Duration::from_seconds(86_400.0);
        let without = propagate(&state, t0, t1, &Settings::default())
            .expect("parameter-less propagation")
            .state_end();
        let with_spacecraft = Settings {
            spacecraft: Some(SpacecraftModel {
                mass_kg: 157.0,
                radius_m: 1.0,
                c_r: 1.3,
                c_d: 2.2,
            }),
            ..Settings::default()
        };
        let with = propagate(&state, t0, t1, &with_spacecraft)
            .expect("spacecraft propagation")
            .state_end();
        let displacement = (with.pos_gcrf_m - without.pos_gcrf_m).length();
        assert!(
            (10.0..100_000.0).contains(&displacement),
            "SRP displaced the day arc by {displacement:.1} m"
        );
    }

    /// Spec §7.15: drag decays a low orbit - six hours at 300 km must
    /// lower the osculating semi-major axis measurably versus the
    /// parameter-less run (the other non-gravitational terms move it by
    /// meters at most; the shrink is drag's signature).
    #[test]
    fn drag_decays_a_low_orbit() {
        init();
        let radius = 6_378_137.0 + 300e3;
        let speed = (MU_M3_S2 / radius).sqrt();
        let inclination = 51.6_f64.to_radians();
        let state = OrbitState {
            pos_gcrf_m: DVec3::new(radius, 0.0, 0.0),
            vel_gcrf_m_s: DVec3::new(0.0, speed * inclination.cos(), speed * inclination.sin()),
        };
        let t0 = Epoch::from_gregorian_utc(2024, 1, 15, 12, 0, 0, 0);
        let t1 = t0 + Duration::from_seconds(6.0 * 3600.0);
        let semi_major = |state: &OrbitState| {
            crate::kepler::Kepler::from_pv(state.pos_gcrf_m, state.vel_gcrf_m_s)
                .expect("bound orbit")
                .semi_major_axis_m
        };

        let without = propagate(&state, t0, t1, &Settings::default())
            .expect("drag-free propagation")
            .state_end();
        let with_spacecraft = Settings {
            spacecraft: Some(SpacecraftModel {
                mass_kg: 157.0,
                radius_m: 1.0,
                c_r: 1.3,
                c_d: 2.2,
            }),
            ..Settings::default()
        };
        let with = propagate(&state, t0, t1, &with_spacecraft)
            .expect("dragged propagation")
            .state_end();
        let shrink = semi_major(&without) - semi_major(&with);
        assert!(
            (50.0..20_000.0).contains(&shrink),
            "drag shrank the semi-major axis by {shrink:.1} m over 6 h"
        );
    }

    /// Shadow-boundary events: an orbit whose plane contains the Sun
    /// direction eclipses every revolution - the driver must record ~4
    /// boundaries per orbit (penumbra in/out, umbra in/out), each landing
    /// on a root of the shadow-boundary function.
    #[test]
    fn eclipse_boundaries_are_detected_and_exact() {
        init();
        let t0 = Epoch::from_gregorian_utc(2024, 1, 15, 12, 0, 0, 0);
        let sun_dir = crate::ephemeris::geocentric_pos(Body::Sol, t0)
            .expect("sun position")
            .normalize();
        let radius = 6_778_000.0;
        let speed = (3.986_004_418e14_f64 / radius).sqrt();
        let state = OrbitState {
            pos_gcrf_m: sun_dir * radius,
            vel_gcrf_m_s: sun_dir.cross(DVec3::Z).normalize() * speed,
        };
        let spacecraft = SpacecraftModel {
            mass_kg: 157.0,
            radius_m: 1.0,
            c_r: 1.3,
            c_d: 2.2,
        };
        let settings = Settings {
            spacecraft: Some(spacecraft),
            ..Settings::default()
        };
        let period = std::f64::consts::TAU * (radius.powi(3) / 3.986_004_418e14).sqrt();
        let t1 = t0 + Duration::from_seconds(3.0 * period);
        let result = propagate(&state, t0, t1, &settings).expect("eclipsing propagation");

        let boundaries = result.shadow_boundaries();
        assert!(
            (8..=16).contains(&boundaries.len()),
            "expected ~12 boundaries over 3 orbits, got {}",
            boundaries.len()
        );

        // Rebuild the facade's shadow configuration and check every
        // recorded epoch sits on a root of the boundary function.
        let almanac = &context().almanac;
        let earth = almanac.frame_info(EARTH_J2000).unwrap();
        let moon = almanac.frame_info(MOON_J2000).unwrap();
        let units = CanonicalUnits::new(
            earth.mu_km3_s2().unwrap() * 1e9,
            earth.mean_equatorial_radius_km().unwrap() * 1e3,
        );
        let srp = SolarRadiationPressure {
            spacecraft,
            occulters: vec![
                Occulter::Central {
                    radius_m: earth.mean_equatorial_radius_km().unwrap() * 1e3,
                },
                Occulter::Luna {
                    radius_m: moon.mean_equatorial_radius_km().unwrap() * 1e3,
                },
            ],
        };
        for &epoch in boundaries {
            let sample = result.interp(epoch).expect("state at boundary");
            let ctx = EvalContext::new(units, epoch);
            let (outer, inner) = srp
                .boundary_functions(&ctx, units.length_to_can(sample.pos_gcrf_m))
                .unwrap();
            let nearest = outer.abs().min(inner.abs());
            // The recorded epoch is root-found on the solver's
            // interpolated path, which skews it by up to a few ms of true
            // crossing time (~1e-3 rad/s separation rate x ms = ~1e-5
            // rad) - physically negligible against the ~10 s penumbra.
            assert!(
                nearest < 2e-5,
                "boundary at {epoch} is not a root: (outer, inner) = ({outer:.3e}, {inner:.3e})"
            );
        }
    }

    /// Backward spans stay first-class with the event machinery active.
    /// Tight tolerances on purpose: every shadow boundary restarts the
    /// integrator, and per-restart error scales with the tolerance (at the
    /// default 1e-8 the ~16 restarts of this arc accumulate ~7 m; at 1e-11
    /// they accumulate millimeters - the machinery converges).
    #[test]
    fn backward_span_round_trips_with_spacecraft() {
        init();
        let (state, t0) = circular_leo();
        let settings = Settings {
            abs_error: 1e-11,
            rel_error: 1e-11,
            spacecraft: Some(SpacecraftModel {
                mass_kg: 157.0,
                radius_m: 1.0,
                c_r: 1.3,
                c_d: 2.2,
            }),
            ..Settings::default()
        };
        let t_back = t0 - Duration::from_seconds(3.0 * 3600.0);
        let back = propagate(&state, t0, t_back, &settings)
            .expect("backward propagation")
            .state_end();
        let forward = propagate(&back, t_back, t0, &settings)
            .expect("forward propagation")
            .state_end();
        assert!(
            (forward.pos_gcrf_m - state.pos_gcrf_m).length() < 2.0,
            "round trip drifted {} m",
            (forward.pos_gcrf_m - state.pos_gcrf_m).length()
        );
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
