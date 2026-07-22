//! Osculating-elements timing, crate vs reference satkit: Keplerian
//! elements from a position/velocity state. Timing only — the `cargo test`
//! comparison in `lib.rs` proves the pairing (and documents the mu
//! provenance gap between the two).

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use engine_astrodynamics::kepler::Kepler;
use engine_astrodynamics_tests::data::seed_satkit;
use engine_astrodynamics_tests::support::satkit_vec;
use glam::DVec3;

fn bench_kepler(c: &mut Criterion) {
    seed_satkit();
    // Perigee state at e = 0.7 (same construction as the comparison test):
    // v_perigee = sqrt(mu (1 + e) / r).
    let mu_m3_s2 = 3.986004418e14_f64;
    let radius_m = 6_778_000.0_f64;
    let pos = DVec3::new(radius_m, 0.0, 0.0);
    let vel = DVec3::new(0.0, (mu_m3_s2 * 1.7 / radius_m).sqrt(), 0.0);
    let (reference_pos, reference_vel) = (satkit_vec(pos), satkit_vec(vel));
    let mut group = c.benchmark_group("kepler_from_pv");
    group.bench_function("crate", |b| {
        b.iter(|| Kepler::from_pv(black_box(pos), black_box(vel)).unwrap());
    });
    group.bench_function("satkit", |b| {
        b.iter(|| {
            satkit::Kepler::from_pv(black_box(reference_pos), black_box(reference_vel)).unwrap()
        });
    });
    group.finish();
}

criterion_group!(benches, bench_kepler);
criterion_main!(benches);
