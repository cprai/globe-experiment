//! Ephemeris query timing, crate vs reference satkit. Luna geocentric
//! state: the fastest-moving standard query (the celestial sphere's
//! per-frame kind of lookup). Timing only — the `cargo test` comparisons
//! in `lib.rs` prove the two sides solve the same problem. Inputs are
//! prepared outside the timed closures; the reference side is seeded once
//! (`data::seed_satkit`) and the crate side's lazily-parsed kernels warm
//! up inside criterion's warm-up phase.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use engine_astrodynamics::Epoch;
use engine_astrodynamics::ephemeris::{self, Body};
use engine_astrodynamics_tests::data::seed_satkit;
use engine_astrodynamics_tests::support::satkit_instant_at_tdb;

fn bench_ephemeris(c: &mut Criterion) {
    seed_satkit();
    let epoch = Epoch::from_gregorian_utc(2024, 1, 15, 12, 30, 0, 0);
    let instant = satkit_instant_at_tdb(epoch);
    let mut group = c.benchmark_group("ephemeris_luna_geocentric_state");
    group.bench_function("crate", |b| {
        b.iter(|| ephemeris::geocentric_state(Body::Luna, black_box(epoch)).unwrap());
    });
    group.bench_function("satkit", |b| {
        b.iter(|| {
            satkit::jplephem::geocentric_state(satkit::SolarSystem::Moon, black_box(&instant))
                .unwrap()
        });
    });
    group.finish();
}

criterion_group!(benches, bench_ephemeris);
criterion_main!(benches);
