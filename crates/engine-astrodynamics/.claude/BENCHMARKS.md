# Benchmark record (agent cache — read this instead of re-running)

Criterion comparison benches: `engine-astrodynamics` vs reference satkit
0.18 on the same problems. This file is the cache layer for those numbers
— the benches take minutes to run, so consult this record first.
Maintenance policy (also stated in CLAUDE.md):

- **Any time a bench target is run, update its rows here** (mid estimate
  of criterion's three-value interval), with the date and, if it changed,
  the environment line.
- **Re-run the affected targets after any change that could plausibly move
  performance** (force model, integrator, trajectory/interp, ephemeris or
  frame plumbing, dependency bumps) and record the result — stale numbers
  are worse than none.

How to run (one target at a time; each takes seconds except `propagation`,
~1 min):

```sh
cargo bench -p engine-astrodynamics-tests --bench \
    <ephemeris|frames|geodetic|kepler|sgp4|propagation>
```

Sources: `tests/src/benches/*.rs`. Timing only — correctness of the
pairings is proven by the harness's `cargo test` comparisons.

## Environment

Dev sandbox: x86_64 Linux container (see platform.md; wall-clock noise is
possible — criterion's intervals were tight, ±1% or better). Warm process:
satkit seeded, the crate's `AstroData` loaded eagerly before measurement.
Bench profile (inherits release). Measured 2026-07-22, after the
segments.rs fast-path landing (below); `sgp4`/`geodetic` rows are from the
same-day pre-fast-path run (their pure code paths are untouched).

## Results (2026-07-22, post fast-path)

| Group | crate | satkit | crate/satkit |
|---|---|---|---|
| `ephemeris_luna_geocentric_state` | 356 ns | 82.7 ns | 4.3x slower |
| `frames_qgcrf2itrf` | 352 ns | 46.3 µs | **131x faster** |
| `frames_qteme2gcrf` | 1.26 µs | 377 ns | 3.3x slower (fidelity gap, see below) |
| `geodetic_from_itrf` | 70.7 ns | 328 ns | **4.6x faster** |
| `kepler_from_pv` (e = 0.7) | 31.1 ns | 43.5 ns | **1.4x faster** |
| `sgp4_iss_113_samples` | 21.3 µs | 22.7 µs | parity |
| `propagate_leo_one_orbit` (4×4 + sun/moon + rel, 1e-8) | 369.8 µs | 409 µs | **1.11x faster** |
| `interp_batch_500_samples` | 24.8 µs | 22.7 µs | ~parity |

Prior generation (same day, pre-fast-path), for scale: Luna 2.25 µs
(26.9x slower), qgcrf2itrf 2.88 µs, qteme2gcrf 8.55 µs (19.6x slower),
kepler 291 ns (6.6x slower), propagate 2.17 ms (5.3x slower).

## Reading the numbers (2026-07-22, post fast-path)

- **Every former slow row is fixed; propagate now BEATS satkit** (369.8 µs
  vs 409 µs for the same one-orbit problem) with zero caching and zero
  accuracy change — the fix was precomputation only (see the
  implementation record below). The in-crate machine-agreement tests pin
  the fast paths to the anise almanac results; the harness bounds vs
  satkit are unchanged and pass.
- **Luna geocentric at 4.3x is structural, not waste**: DE440 stores Luna
  geocentrically so satkit's Moon lookup is ONE preloaded Chebyshev eval;
  the crate evaluates the honest two-leg tree (301-wrt-3 + 399-wrt-3),
  pays one Epoch→ET conversion, and returns velocity too. ~356 ns/call
  is far below any hot-loop threshold that matters here.
- **TEME at 3.3x is a fidelity gap, not overhead**: the crate runs the
  full IAU-76/FK5 chain (106-term IAU-1980 nutation series, once);
  satkit's 377 ns is Vallado's ~1-arcsec 2-term nutation approximation.
  Same-accuracy comparison isn't available from satkit; the harness
  measured 0.22" agreement with the crate as the full-series side.
- **GCRF↔ITRF at 131x faster** is the pre-resolved BPC segment evaluation
  (degree-20 Chebyshev over 1-day records) vs satkit's full IERS-2010
  series; **geodetic ~5x faster** (Vermeille closed form).
- **propagate's remaining budget is force math, not plumbing**: the three
  per-derivative lookups (Sol + Luna states, Earth rotation + omega) now
  cost ~1.1 µs combined vs ~7.5 µs before; at ~290 derivative
  evaluations/orbit that is ~0.3 ms of lookups gone. The per-step
  third-body cache once anticipated in CLAUDE.md is unnecessary — exact
  per-call evaluation is now cheap enough.
- **SGP4 and `interp_batch` (the engine's per-frame trail path) are at
  parity** — untouched pure paths.

## Fast-path implementation record (2026-07-22, segments.rs)

The four slow rows above were fixed by LOAD-TIME PRECOMPUTATION — no
dynamic caching layers, no algorithm downgrades. Root causes were traced
in source (anise 0.10.4, sofars 0.6.1, hifitime 4.3, satkit 0.18.1);
each fix reuses the dependency's own math and is pinned to it by a
machine-agreement test. What anise's almanac re-resolved per call, and
what `AstroData::load` now precomputes:

1. **Frame-tree resolution** (`translate`/`rotate`): per-call root sweep
   over every loaded summary + two path walks (~800 ns) + zerocopy
   re-derivation of the segment view from raw DAF bytes per leg.
   → segments.rs flattens every SPK/BPC segment at load into its record
   grid (coefficient slice over the `'static` embeds, same
   `(start_index−1)*8..end_index*8` slice as `DAF::nth_data`); queries
   run anise's own `chebyshev_eval` over anise's own record layout
   (`Type2ChebyshevRecord::from_slice_f64`). The DE440 tree edges
   (301→3, 399→3, planets→0…) are read from the summaries at load.
2. **Epoch conversions in comparisons** — the sneaky one: hifitime's
   `Epoch: Ord` calls `to_tai_duration()` on BOTH sides of every
   compare, a trig-bearing TDB conversion for the summaries' ET-scale
   epochs. A ~60-segment BPC scan paid ~80 such conversions per rotation
   query (this alone was ~1.9 µs of the old 2.88 µs qgcrf2itrf).
   → segment bounds are stored as TAI duration parts (the exact quantity
   `Epoch::cmp` compares), converted once at load; the query epoch is
   converted once per call. anise's `evaluate` also re-ran
   `to_et_seconds()` three times per leg (including the per-segment
   constant `init_epoch`) — now one ET conversion per query.
3. **TEME triple evaluation** (anise `orientations/dynamic.rs`): the
   dynamic-frame DCM builds its time derivative by ±1 s finite
   difference — three full `teme_rot_mat` evaluations per call — and
   sofars' `eqeq94` internally re-runs the 106-term `nut80` series that
   `nutm80` already evaluated: 6 series evaluations + 3 precession
   matrices per call, for a derivative the positions-only quaternion
   discards. → frames.rs calls sofars directly (now a direct dep — the
   same crate anise uses, version-unified by Cargo): `pmat76` + ONE
   `nut80` feeding both the nutation matrix and the 1994 equation of
   the equinoxes. 8.55 µs → 1.26 µs at identical fidelity (pinned to
   anise's `EARTH_TEME_LEGACY_FRAME` by test at < 1e-12 rad).
4. **Kepler accessor recomputation**: per-call `frame_info` dataset scan
   plus anise `Orbit` accessors re-deriving shared intermediates
   (`period()` → `sma_km()` → energy again; `ecc()` → full e-vector),
   each behind Result machinery and a `Duration` round-trip. → Terra's
   GM is resolved once at load onto `AstroData`; `Kepler::from_pv` is a
   single pass using anise's exact expressions (pinned by test against
   the `Orbit` accessor chain).

Notes for future work on this path:

- **The BPC chain is ITRF93-wrt-ECLIPJ2000**, not J2000: anise composes
  a constant obliquity `r1` leg (`J2000_TO_ECLIPJ2000_ANGLE_RAD`).
  segments.rs asserts the parent at load and pre-builds that constant
  matrix; a replacement kernel with a different parent fails the load
  assert loudly.
- **Embed statics must stay NAMED** (`aligned_kernel!` in data.rs): the
  old `&Align8(*include_bytes!(..))` form promoted the kernels into
  anonymous allocations, which rustc duplicates into every codegen unit
  that reads them. When segments.rs became a second reader, release
  LLVM materialized the ~180 MB twice and the build was OOM-killed
  (rustc peak ~16 GB; ~9.3 GB before, ~13.9 GB now — still large, plan
  memory headroom for release builds in this sandbox).
- **Edge semantics**: segment selection mirrors anise's ±100 ns summary
  slack; within that sliver the fast path evaluates the nearest record
  where anise's `evaluate` would err on its own ±1 ns re-check — sub-µs
  band at the kernel span edges, unreachable through the EOP-gated
  scenes.
- The almanac stays loaded as the correctness oracle (`segments.rs` and
  `frames.rs`/`kepler.rs` tests compare against it) and as the
  planetary-constants source; `propagation/mod.rs` still uses
  `frame_info` per `propagate()` call (cold setup, fine).
- Remaining upstream-able waste (not worth crate-side work now): anise
  could memoize frame-tree paths after load and offer a rotation-only
  dynamic-frame variant that skips the finite-difference derivative.
