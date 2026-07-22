//! Numerical-propagation timing, crate vs reference satkit: the recorded
//! performance scenario (crate CLAUDE.md) — a 1-orbit LEO arc under the
//! default-equivalent 4x4 model with sun/moon + relativity at 1e-8,
//! spelled out explicitly on both sides so the physics stay matched — plus
//! the engine's per-frame trail path (501 dense samples interpolated from
//! an already-computed propagation), plus a mu-matched two-body day
//! against reference astrodyn's JEOD pipeline (its comparable surface —
//! its Earth field is GGM05C, so the full-model groups stay satkit-only).
//! Timing only — the `cargo test` comparisons in `src/tests/propagation.rs`
//! prove the pairings. This is the slowest bench target (~1.5 min): the
//! propagate iterations run milliseconds, so those groups get a longer
//! measurement window than criterion's 5 s default.

use std::hint::black_box;
use std::time::Duration as StdDuration;

use criterion::{Criterion, criterion_group, criterion_main};
use engine_astrodynamics::propagation::{OrbitState, Settings, propagate};
use engine_astrodynamics::{Duration, Epoch};
use engine_astrodynamics_tests::data::{astro, seed_satkit};
use engine_astrodynamics_tests::support::satkit_instant;
use glam::DVec3;

fn bench_propagation(c: &mut Criterion) {
    seed_satkit();
    let data = astro();
    let state = OrbitState {
        pos_gcrf_m: DVec3::new(6_778_000.0, 0.0, 0.0),
        vel_gcrf_m_s: DVec3::new(0.0, 4_764.0, 6_009.0),
    };
    let begin = Epoch::from_gregorian_utc(2024, 1, 15, 12, 0, 0, 0);
    let end = begin + Duration::from_seconds(5_400.0); // ~one orbit
    let crate_settings = Settings::default();
    let satkit_settings = satkit::orbitprop::PropSettings {
        gravity_degree: 4,
        gravity_order: 4,
        abs_error: 1e-8,
        rel_error: 1e-8,
        use_sun_gravity: true,
        use_moon_gravity: true,
        use_relativistic_correction: true,
        // Keeps satkit's non-embedded space-weather loader unreachable.
        use_spaceweather: false,
        ..satkit::orbitprop::PropSettings::default()
    };

    let mut packed = satkit::orbitprop::SimpleState::zeros();
    for (i, v) in [
        state.pos_gcrf_m.x,
        state.pos_gcrf_m.y,
        state.pos_gcrf_m.z,
        state.vel_gcrf_m_s.x,
        state.vel_gcrf_m_s.y,
        state.vel_gcrf_m_s.z,
    ]
    .into_iter()
    .enumerate()
    {
        packed[i] = v;
    }
    let (satkit_begin, satkit_end) = (satkit_instant(begin), satkit_instant(end));

    let mut group = c.benchmark_group("propagate_leo_one_orbit");
    group.measurement_time(StdDuration::from_secs(15));
    group.bench_function("crate", |b| {
        b.iter(|| propagate(data, &state, begin, end, black_box(&crate_settings)).unwrap());
    });
    group.bench_function("satkit", |b| {
        b.iter(|| {
            satkit::orbitprop::propagate(
                &packed,
                &satkit_begin,
                &satkit_end,
                black_box(&satkit_settings),
                None,
            )
            .unwrap()
        });
    });
    group.finish();

    let samples: Vec<Epoch> = (0..=500)
        .map(|i| begin + Duration::from_seconds(5_400.0 * f64::from(i) / 500.0))
        .collect();
    let instants: Vec<satkit::Instant> = samples.iter().map(|&e| satkit_instant(e)).collect();
    let ours = propagate(data, &state, begin, end, &crate_settings).expect("crate propagation");
    let theirs =
        satkit::orbitprop::propagate(&packed, &satkit_begin, &satkit_end, &satkit_settings, None)
            .expect("satkit propagation");

    let mut group = c.benchmark_group("interp_batch_500_samples");
    group.bench_function("crate", |b| {
        b.iter(|| ours.interp_batch(black_box(&samples)).unwrap());
    });
    group.bench_function("satkit", |b| {
        b.iter(|| theirs.interp_batch(black_box(&instants)).unwrap());
    });
    group.finish();

    bench_two_body_day(c);
}

/// Crate (adaptive DOP853 at 1e-10) vs astrodyn (JEOD pipeline, fixed
/// 5 s RK4 — the same configuration the `cargo test` comparison proved
/// meter-level equivalent on this problem, so the rows do comparable
/// work). Point-mass gravity only on both sides; each iteration includes
/// each side's full per-propagation setup, astrodyn's simulation build
/// included, mirroring what a caller pays per arc.
fn bench_two_body_day(c: &mut Criterion) {
    use astrodyn::{
        FrameUid, GravityControl, GravityGradient, SimulationBuilder, TranslationalStateTyped,
        VehicleBuilder,
    };
    use astrodyn_quantities::frame::{Earth, PlanetInertial, RootInertial};
    use astrodyn_quantities::prelude::*;
    use engine_astrodynamics_tests::support::astrodyn_vec;

    let data = astro();
    let inclination = 51.6_f64.to_radians();
    let pos = DVec3::new(6_778_000.0, 0.0, 0.0);
    let vel = DVec3::new(
        0.0,
        7_668.6 * inclination.cos(),
        7_668.6 * inclination.sin(),
    );
    let state = OrbitState {
        pos_gcrf_m: pos,
        vel_gcrf_m_s: vel,
    };
    let begin = Epoch::from_gregorian_utc(2024, 1, 15, 12, 0, 0, 0);
    let end = begin + Duration::from_seconds(86_400.0);
    let settings = Settings {
        gravity_degree: 0,
        gravity_order: 0,
        abs_error: 1e-10,
        rel_error: 1e-10,
        use_sun_gravity: false,
        use_moon_gravity: false,
        use_relativistic_correction: false,
        spacecraft: None,
    };

    let mut group = c.benchmark_group("propagate_two_body_one_day");
    group.measurement_time(StdDuration::from_secs(15));
    group.bench_function("crate", |b| {
        b.iter(|| propagate(data, &state, begin, end, black_box(&settings)).unwrap());
    });
    group.bench_function("astrodyn", |b| {
        b.iter(|| {
            let mut builder =
                SimulationBuilder::new(astrodyn::recipes::epoch::j2000(), black_box(5.0));
            builder.add_source("Earth", astrodyn::recipes::earth::point_mass());
            let vehicle = VehicleBuilder::new()
                .vehicle_named("probe")
                .with_translational(TranslationalStateTyped::<RootInertial> {
                    position: astrodyn_vec(pos).m_at::<RootInertial>(),
                    velocity: astrodyn_vec(vel).m_per_s_at::<RootInertial>(),
                })
                .three_dof_point_mass(1.0.kg())
                .rk4()
                .gravity(GravityControl::new_spherical(
                    FrameUid::of::<PlanetInertial<Earth>>(),
                    GravityGradient::Skip,
                ))
                .build();
            builder.add_body(vehicle);
            let mut sim = astrodyn_runner::Simulation::from_builder(builder)
                .expect("valid astrodyn simulation");
            sim.step_until(86_400.0).expect("astrodyn propagation");
            sim.body(0)
        });
    });
    group.finish();
}

criterion_group!(benches, bench_propagation);
criterion_main!(benches);
