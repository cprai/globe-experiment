//! Ephemeris query timing, crate vs reference satkit vs reference
//! astrodyn. Luna geocentric state: the fastest-moving standard query
//! (the celestial sphere's per-frame kind of lookup). Timing only — the
//! `cargo test` comparisons in `lib.rs` prove the sides solve the same
//! problem. Inputs are prepared outside the timed closures; the satkit
//! side is seeded once (`data::seed_satkit`) and the crate/astrodyn data
//! is loaded eagerly before the timed loops (`data::astro` /
//! `data::astrodyn_eph` — astrodyn runs anise's generic almanac over the
//! same DE440 bytes, so its row is what the crate's pre-resolved segment
//! fast path is measured against).

use std::hint::black_box;

use astrodyn_ephemeris::EphemerisBody;
use criterion::{Criterion, criterion_group, criterion_main};
use engine_astrodynamics::Epoch;
use engine_astrodynamics::ephemeris::{self, Body};
use engine_astrodynamics_tests::data::{astro, astrodyn_eph, seed_satkit};
use engine_astrodynamics_tests::support::satkit_instant_at_tdb;

fn bench_ephemeris(c: &mut Criterion) {
    seed_satkit();
    let data = astro();
    let reference = astrodyn_eph();
    let epoch = Epoch::from_gregorian_utc(2024, 1, 15, 12, 30, 0, 0);
    let instant = satkit_instant_at_tdb(epoch);
    let mut group = c.benchmark_group("ephemeris_luna_geocentric_state");
    group.bench_function("crate", |b| {
        b.iter(|| ephemeris::geocentric_state(data, Body::Luna, black_box(epoch)).unwrap());
    });
    group.bench_function("satkit", |b| {
        b.iter(|| {
            satkit::jplephem::geocentric_state(satkit::SolarSystem::Moon, black_box(&instant))
                .unwrap()
        });
    });
    group.bench_function("astrodyn", |b| {
        b.iter(|| {
            reference
                .get_state_typed_epoch(EphemerisBody::Moon, EphemerisBody::Earth, black_box(epoch))
                .unwrap()
        });
    });
    group.finish();
}

criterion_group!(benches, bench_ephemeris);
criterion_main!(benches);
