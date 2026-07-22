//! SGP4 timing, crate vs reference satkit: the ISS TLE over the comparison
//! test's +-1 week window (113 samples). Timing only — the `cargo test`
//! comparison in `src/tests/sgp4.rs` proves the pairing. satkit builds and
//! caches its SGP4 constants inside the TLE on first use (hence `&mut`), so one
//! warm call precedes the loop to match the crate side, which builds constants
//! at parse.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use engine_astrodynamics::{Duration, Epoch, sgp4::sgp4, tle::Tle};
use engine_astrodynamics_tests::data::seed_satkit;
use engine_astrodynamics_tests::support::satkit_instant;

fn bench_sgp4(c: &mut Criterion) {
    seed_satkit();
    // Same harness-owned ISS fixture as the comparison test (real
    // checksums — the `sgp4` crate validates them).
    let iss = [
        "ISS (ZARYA)",
        "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9009",
        "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299350",
    ];
    let ours_tle = Tle::load_3line(iss[0], iss[1], iss[2]).expect("valid TLE");
    let mut theirs_tle = satkit::tle::TLE::load_3line(iss[0], iss[1], iss[2]).expect("valid TLE");

    let t0 = ours_tle.epoch();
    let epochs: Vec<Epoch> = (-56..=56)
        .map(|i| t0 + Duration::from_seconds(3.0 * 3600.0 * f64::from(i)))
        .collect();
    let instants: Vec<satkit::Instant> = epochs.iter().map(|&e| satkit_instant(e)).collect();
    satkit::sgp4::sgp4(&mut theirs_tle, &instants[..1]).expect("satkit sgp4 warm-up");

    let mut group = c.benchmark_group("sgp4_iss_113_samples");
    group.bench_function("crate", |b| {
        b.iter(|| sgp4(black_box(&ours_tle), black_box(&epochs)).unwrap());
    });
    group.bench_function("satkit", |b| {
        b.iter(|| satkit::sgp4::sgp4(&mut theirs_tle, black_box(&instants)).unwrap());
    });
    group.finish();
}

criterion_group!(benches, bench_sgp4);
criterion_main!(benches);
