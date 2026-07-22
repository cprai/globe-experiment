//! Crate-vs-reference numerical-propagation comparisons.

/// The satkit reference side.
mod satkit {
    use engine_astrodynamics::propagation::{OrbitState, Settings, propagate};
    use engine_astrodynamics::{Duration, Epoch};
    use glam::DVec3;
    use satkit::orbitprop::{PropSettings, SimpleState};

    use crate::support::satkit_instant;

    /// The full-model bound over a 1-day LEO arc. Honest physical budget,
    /// not a delegation check: EGM2008 vs EGM96 coefficients, our degree-2
    /// solid tides (satkit has none), different integrators at 1e-8
    /// tolerances, satkit's TT-as-TDB third-body positions, and different
    /// Earth-orientation realizations for the harmonic rotation.
    /// Measured 2026-07-21: 1.63 m (1-day full model), 0.72 m (6 h
    /// backward).
    const FULL_MODEL_TOL_M: f64 = 10.0;
    /// Reduced configuration (degree 2, no third bodies, no relativity):
    /// nearly the same physics on both sides - J2-coefficient and
    /// orientation-realization differences remain.
    /// Measured 2026-07-21: 1.09 m.
    const REDUCED_MODEL_TOL_M: f64 = 5.0;

    /// An inclined circular-ish LEO, deliberately not equatorial so J2
    /// secular terms act on the node.
    fn leo_state() -> OrbitState {
        let speed = 7_668.6;
        let inclination = 51.6_f64.to_radians();
        OrbitState {
            pos_gcrf_m: DVec3::new(6_778_000.0, 0.0, 0.0),
            vel_gcrf_m_s: DVec3::new(0.0, speed * inclination.cos(), speed * inclination.sin()),
        }
    }

    fn satkit_end_state(
        state: &OrbitState,
        begin: Epoch,
        end: Epoch,
        settings: &PropSettings,
    ) -> (DVec3, DVec3) {
        let mut packed = SimpleState::zeros();
        packed[0] = state.pos_gcrf_m.x;
        packed[1] = state.pos_gcrf_m.y;
        packed[2] = state.pos_gcrf_m.z;
        packed[3] = state.vel_gcrf_m_s.x;
        packed[4] = state.vel_gcrf_m_s.y;
        packed[5] = state.vel_gcrf_m_s.z;
        let result = satkit::orbitprop::propagate(
            &packed,
            &satkit_instant(begin),
            &satkit_instant(end),
            settings,
            None,
        )
        .expect("satkit propagation");
        let y = result.state_end;
        (DVec3::new(y[0], y[1], y[2]), DVec3::new(y[3], y[4], y[5]))
    }

    fn satkit_settings(degree: u16, sun_moon: bool, relativity: bool) -> PropSettings {
        PropSettings {
            gravity_degree: degree,
            gravity_order: degree,
            abs_error: 1e-10,
            rel_error: 1e-10,
            use_sun_gravity: sun_moon,
            use_moon_gravity: sun_moon,
            use_relativistic_correction: relativity,
            // Keeps satkit's non-embedded space-weather loader unreachable.
            use_spaceweather: false,
            ..PropSettings::default()
        }
    }

    fn crate_settings(degree: u16, sun_moon: bool, relativity: bool) -> Settings {
        Settings {
            gravity_degree: degree,
            gravity_order: degree,
            abs_error: 1e-10,
            rel_error: 1e-10,
            use_sun_gravity: sun_moon,
            use_moon_gravity: sun_moon,
            use_relativistic_correction: relativity,
            spacecraft: None,
        }
    }

    #[test]
    fn full_model_matches_satkit_over_one_day() {
        crate::data::seed_satkit();
        let state = leo_state();
        let begin = Epoch::from_gregorian_utc(2024, 1, 15, 12, 0, 0, 0);
        let end = begin + Duration::from_seconds(86_400.0);
        let ours = propagate(
            crate::data::astro(),
            &state,
            begin,
            end,
            &crate_settings(4, true, true),
        )
        .expect("crate propagation")
        .state_end();
        let (want_pos, want_vel) =
            satkit_end_state(&state, begin, end, &satkit_settings(4, true, true));
        let pos_diff = (ours.pos_gcrf_m - want_pos).length();
        let vel_diff = (ours.vel_gcrf_m_s - want_vel).length();
        assert!(
            pos_diff <= FULL_MODEL_TOL_M,
            "1-day full-model position differs by {pos_diff:.2} m"
        );
        assert!(
            vel_diff <= FULL_MODEL_TOL_M / 1000.0,
            "1-day full-model velocity differs by {vel_diff:.4} m/s"
        );
    }

    #[test]
    fn reduced_model_matches_satkit_tightly() {
        crate::data::seed_satkit();
        let state = leo_state();
        let begin = Epoch::from_gregorian_utc(2024, 1, 15, 12, 0, 0, 0);
        let end = begin + Duration::from_seconds(86_400.0);
        let ours = propagate(
            crate::data::astro(),
            &state,
            begin,
            end,
            &crate_settings(2, false, false),
        )
        .expect("crate propagation")
        .state_end();
        let (want_pos, _) = satkit_end_state(&state, begin, end, &satkit_settings(2, false, false));
        let pos_diff = (ours.pos_gcrf_m - want_pos).length();
        assert!(
            pos_diff <= REDUCED_MODEL_TOL_M,
            "1-day reduced-model position differs by {pos_diff:.2} m"
        );
    }

    /// Backward spans agree with the reference too (satkit integrates
    /// backward natively; the crate re-poses backward arcs as negated
    /// forward problems).
    #[test]
    fn backward_span_matches_satkit() {
        crate::data::seed_satkit();
        let state = leo_state();
        let begin = Epoch::from_gregorian_utc(2024, 1, 15, 12, 0, 0, 0);
        let end = begin - Duration::from_seconds(6.0 * 3600.0);
        let ours = propagate(
            crate::data::astro(),
            &state,
            begin,
            end,
            &crate_settings(4, true, true),
        )
        .expect("crate propagation")
        .state_end();
        let (want_pos, _) = satkit_end_state(&state, begin, end, &satkit_settings(4, true, true));
        let pos_diff = (ours.pos_gcrf_m - want_pos).length();
        assert!(
            pos_diff <= FULL_MODEL_TOL_M,
            "6-hour backward position differs by {pos_diff:.2} m"
        );
    }
}

/// The astrodyn (JEOD-port) reference side.
mod astrodyn {
    use astrodyn::{
        FrameUid, GravityControl, GravityGradient, SimulationBuilder, TranslationalStateTyped,
        VehicleBuilder,
    };
    use astrodyn_quantities::frame::{Earth, PlanetInertial, RootInertial};
    use astrodyn_quantities::prelude::*;
    use engine_astrodynamics::kepler::Kepler;
    use engine_astrodynamics::propagation::{OrbitState, Settings, propagate};
    use engine_astrodynamics::{Duration, Epoch};
    use glam::DVec3;

    use crate::support::{astrodyn_vec, from_astrodyn_vec};

    /// astrodyn's fixed RK4 step. At 5 s (~1100 steps/orbit) the RK4
    /// truncation is far below the comparison bound, so the bound
    /// measures model agreement, not integrator gap.
    const ASTRODYN_DT_S: f64 = 5.0;

    /// One day, mu-matched two-body problem on both sides (the crate at
    /// point-mass degradation, astrodyn at point-mass gravity with its
    /// source mu overwritten by the crate's own pca-derived value) - the
    /// residual is pure integrator truncation. Measured 2026-07-22:
    /// 0.0074 m position, 8e-6 m/s velocity over the day (~16 orbits).
    const TWO_BODY_TOL_M: f64 = 0.1;

    /// Same inclined LEO as the satkit propagation module.
    fn leo_state() -> OrbitState {
        let speed = 7_668.6;
        let inclination = 51.6_f64.to_radians();
        OrbitState {
            pos_gcrf_m: DVec3::new(6_778_000.0, 0.0, 0.0),
            vel_gcrf_m_s: DVec3::new(0.0, speed * inclination.cos(), speed * inclination.sin()),
        }
    }

    /// The crate's central-body mu (m^3/s^2), recovered from its own
    /// osculating elements (period = TAU sqrt(a^3/mu)) since the pca
    /// value is not exposed directly. Machine-precision round trip.
    fn crate_mu_m3_s2(state: &OrbitState) -> f64 {
        let kepler = Kepler::from_pv(crate::data::astro(), state.pos_gcrf_m, state.vel_gcrf_m_s)
            .expect("elliptic LEO state");
        let a = kepler.semi_major_axis_m;
        std::f64::consts::TAU.powi(2) * a.powi(3) / kepler.period_s.powi(2)
    }

    /// Point-mass-only crate settings: degree < 2 degrades EGM2008 to
    /// point-mass, sun/moon/relativity off, no spacecraft - a pure
    /// two-body problem.
    fn two_body_settings() -> Settings {
        Settings {
            gravity_degree: 0,
            gravity_order: 0,
            abs_error: 1e-10,
            rel_error: 1e-10,
            use_sun_gravity: false,
            use_moon_gravity: false,
            use_relativistic_correction: false,
            spacecraft: None,
        }
    }

    /// One-day astrodyn two-body end state via the JEOD pipeline runner
    /// (point-mass Earth at origin, RK4, epoch irrelevant to the physics).
    pub(crate) fn astrodyn_two_body_end(
        state: &OrbitState,
        mu_m3_s2: f64,
        span_s: f64,
    ) -> (DVec3, DVec3) {
        let mut entry = astrodyn::recipes::earth::point_mass();
        entry.source.mu = mu_m3_s2;
        let mut builder = SimulationBuilder::new(astrodyn::recipes::epoch::j2000(), ASTRODYN_DT_S);
        builder.add_source("Earth", entry);
        let vehicle = VehicleBuilder::new()
            .vehicle_named("probe")
            .with_translational(TranslationalStateTyped::<RootInertial> {
                position: astrodyn_vec(state.pos_gcrf_m).m_at::<RootInertial>(),
                velocity: astrodyn_vec(state.vel_gcrf_m_s).m_per_s_at::<RootInertial>(),
            })
            .three_dof_point_mass(1.0.kg())
            .rk4()
            .gravity(GravityControl::new_spherical(
                FrameUid::of::<PlanetInertial<Earth>>(),
                GravityGradient::Skip,
            ))
            .build();
        builder.add_body(vehicle);
        let mut sim =
            astrodyn_runner::Simulation::from_builder(builder).expect("valid astrodyn simulation");
        sim.step_until(span_s).expect("astrodyn propagation");
        // With the single source at the origin the integration frame and
        // the root inertial frame coincide, so the integrated state IS
        // the inertial state (no integ-origin shift to apply).
        let out = sim.body(0);
        (
            from_astrodyn_vec(out.trans.position.raw_si()),
            from_astrodyn_vec(out.trans.velocity.raw_si()),
        )
    }

    #[test]
    fn two_body_day_matches_astrodyn() {
        let state = leo_state();
        let begin = Epoch::from_gregorian_utc(2024, 1, 15, 12, 0, 0, 0);
        let end = begin + Duration::from_seconds(86_400.0);
        let ours = propagate(
            crate::data::astro(),
            &state,
            begin,
            end,
            &two_body_settings(),
        )
        .expect("crate propagation")
        .state_end();
        let (want_pos, want_vel) = astrodyn_two_body_end(&state, crate_mu_m3_s2(&state), 86_400.0);
        let pos_diff = (ours.pos_gcrf_m - want_pos).length();
        let vel_diff = (ours.vel_gcrf_m_s - want_vel).length();
        assert!(
            pos_diff <= TWO_BODY_TOL_M,
            "1-day two-body position differs by {pos_diff:.3} m"
        );
        assert!(
            vel_diff <= TWO_BODY_TOL_M / 1000.0,
            "1-day two-body velocity differs by {vel_diff:.5} m/s"
        );
    }
}
