//! Numerical-propagation timing, crate vs reference satkit: the recorded
//! performance scenario (crate CLAUDE.md) — a 1-orbit LEO arc under the
//! default-equivalent 4x4 model with sun/moon + relativity at 1e-8,
//! spelled out explicitly on both sides so the physics stay matched — plus
//! the engine's per-frame trail path (501 dense samples interpolated from
//! an already-computed propagation). Timing only — the `cargo test`
//! comparisons in `lib.rs` prove the pairing. This is the slowest bench
//! target (~1 min): the propagate iterations run milliseconds (~2.2 ms
//! crate vs ~0.4 ms satkit measured 2026-07-22), so that group gets a
//! longer measurement window than criterion's 5 s default.

use std::hint::black_box;
use std::time::Duration as StdDuration;

use criterion::{Criterion, criterion_group, criterion_main};
use engine_astrodynamics::propagation::{OrbitState, Settings, propagate};
use engine_astrodynamics::{Duration, Epoch};
use engine_astrodynamics_tests::data::seed_satkit;
use engine_astrodynamics_tests::support::satkit_instant;
use glam::DVec3;

fn bench_propagation(c: &mut Criterion) {
    seed_satkit();
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
        b.iter(|| propagate(&state, begin, end, black_box(&crate_settings)).unwrap());
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
    let ours = propagate(&state, begin, end, &crate_settings).expect("crate propagation");
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
}

criterion_group!(benches, bench_propagation);
criterion_main!(benches);
