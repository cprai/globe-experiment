//! Frame-rotation timing, crate vs reference satkit: the GCRF->ITRF and
//! TEME->GCRF quaternions at one modern epoch. Timing only — the
//! `cargo test` comparisons in `lib.rs` prove the pairings; inputs are
//! prepared outside the timed closures and both data stacks are loaded
//! before measurement (satkit seeding + the crate's eager `data::astro`).

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use engine_astrodynamics::{Epoch, frames};
use engine_astrodynamics_tests::data::{astro, seed_satkit};
use engine_astrodynamics_tests::support::{astrodyn_rnp_inputs, satkit_instant};

fn bench_frames(c: &mut Criterion) {
    seed_satkit();
    let data = astro();
    let epoch = Epoch::from_gregorian_utc(2024, 1, 15, 12, 30, 0, 0);
    let instant = satkit_instant(epoch);

    let mut group = c.benchmark_group("frames_qgcrf2itrf");
    group.bench_function("crate", |b| {
        b.iter(|| frames::qgcrf2itrf(data, black_box(epoch)));
    });
    group.bench_function("satkit", |b| {
        b.iter(|| satkit::frametransform::qgcrf2itrf(black_box(&instant)));
    });
    // The full JEOD RNP (precession + 1980 nutation + GAST + polar
    // motion). The timed closure includes the per-epoch EOP lookup and
    // GMST/TT derivation (`astrodyn_rnp_inputs`) because astrodyn's API
    // demands those as caller-supplied inputs — that is the honest
    // per-epoch cost of driving it, mirroring the EOP work the other two
    // rows do internally.
    group.bench_function("astrodyn", |b| {
        b.iter(|| {
            let (gmst_seconds, tt_centuries, polar) = astrodyn_rnp_inputs(black_box(epoch));
            astrodyn_frames::rotation_j2000::compute_t_parent_this_with_polar(
                gmst_seconds,
                tt_centuries,
                Some(polar),
            )
        });
    });
    group.finish();

    let mut group = c.benchmark_group("frames_qteme2gcrf");
    group.bench_function("crate", |b| {
        b.iter(|| frames::qteme2gcrf(data, black_box(epoch)));
    });
    group.bench_function("satkit", |b| {
        b.iter(|| satkit::frametransform::qteme2gcrf(black_box(&instant)));
    });
    group.finish();
}

criterion_group!(benches, bench_frames);
criterion_main!(benches);
