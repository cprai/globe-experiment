//! Geodetic-conversion timing, crate (Vermeille closed form) vs reference
//! satkit, ITRF vector -> latitude/longitude/altitude. Timing only — the
//! `cargo test` grid comparison in `lib.rs` proves the pairing.

use std::hint::black_box;

use astrodyn_math::GeodeticState;
use criterion::{Criterion, criterion_group, criterion_main};
use engine_astrodynamics::geodetic::geodetic_from_itrf;
use engine_astrodynamics_tests::data::seed_satkit;
use engine_astrodynamics_tests::support::{astrodyn_vec, satkit_vec};
use glam::DVec3;

fn bench_geodetic(c: &mut Criterion) {
    seed_satkit();
    // 45 N, 60 E, ~400 km up — an unremarkable point; the crate and
    // satkit closed forms are iteration-free (astrodyn's JEOD Borkowski
    // kernel iterates), so the choice barely matters.
    let v = DVec3::new(3_389_297.0, 5_870_431.0, 4_793_841.0);
    let reference = satkit_vec(v);
    let astrodyn_v = astrodyn_vec(v);
    let mut group = c.benchmark_group("geodetic_from_itrf");
    group.bench_function("crate", |b| {
        b.iter(|| geodetic_from_itrf(black_box(v)));
    });
    group.bench_function("satkit", |b| {
        b.iter(|| {
            satkit::itrfcoord::ITRFCoord::from_vector(black_box(&reference)).to_geodetic_rad()
        });
    });
    group.bench_function("astrodyn", |b| {
        b.iter(|| {
            GeodeticState::from_planet_fixed(
                black_box(astrodyn_v),
                6_378_137.0,
                6_356_752.314_245_179,
            )
        });
    });
    group.finish();
}

criterion_group!(benches, bench_geodetic);
criterion_main!(benches);
