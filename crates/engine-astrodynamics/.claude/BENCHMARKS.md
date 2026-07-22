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
explicit-`AstroData` refactor (no lazy loading; data passed as the first
argument).

## Results (2026-07-22, post-AstroData refactor)

| Group | crate | satkit | crate/satkit |
|---|---|---|---|
| `ephemeris_luna_geocentric_state` | 2.25 µs | 83.7 ns | 26.9x slower |
| `frames_qgcrf2itrf` | 2.88 µs | 46.4 µs | **16.1x faster** |
| `frames_qteme2gcrf` | 8.55 µs | 437 ns | 19.6x slower |
| `geodetic_from_itrf` | 70.7 ns | 328 ns | **4.6x faster** |
| `kepler_from_pv` (e = 0.7) | 291 ns | 43.9 ns | 6.6x slower |
| `sgp4_iss_113_samples` | 21.3 µs | 22.7 µs | parity |
| `propagate_leo_one_orbit` (4×4 + sun/moon + rel, 1e-8) | 2.17 ms | 408 µs | 5.3x slower |
| `interp_batch_500_samples` | 25.1 µs | 22.4 µs | ~parity |

## Reading the numbers (2026-07-22 analysis)

- **The explicit-`AstroData` refactor moved nothing** (same-day re-run,
  all rows within noise of the pre-refactor measurements; the largest
  shifts — qgcrf2itrf 3.14→2.88 µs, satkit's qteme2gcrf 370→437 ns — are
  environment jitter, not code). Expected: the refactor only replaced a
  `LazyLock` deref with a passed reference; anise's per-call resolution
  (below) is untouched.
- **`propagate` is ~5x, not the ~150x once recorded.** The old "~75 ms"
  probe figure included the one-time lazy anise kernel parse inside the
  first timed call; criterion's warm-up excludes it (satkit's figure was
  unchanged by the fix, confirming the attribution). The remaining ~5x is
  believed dominated by anise per-evaluation `translate`/`rotate` frame
  resolution (3 calls per derivative — same root cause as the ephemeris
  row); per-step Chebyshev caching of third-body states is the anticipated
  remedy (CLAUDE.md decision: not until propagate cost is user-visible).
- **Single ephemeris lookups are ~27x slower** (anise Almanac resolution
  vs satkit's direct Chebyshev evaluation) and **TEME is ~24x slower**
  (anise full frame graph vs satkit's closed-form equinox-based chain).
  Per-call costs are microseconds — only hot loops care.
- **Luna-geocentric split, measured 2026-07-22** (throwaway `Instant`
  probe over the crate's own almanac, 200k iters, release; parts sum to
  the whole): full `translate(301→399)` 2267 ns = `frame_info` 46 ns
  (pca hash lookup + `PlanetaryData` clone) + `common_ephemeris_path`
  779 ns (two `ephemeris_path_to_root` walks, each re-running
  `try_find_ephemeris_root` — a 125 ns sweep re-parsing every summary of
  BOTH loaded SPKs — plus indexed `spk_summary_at_epoch` calls at 130 ns
  each; nothing cached between calls) + 2 × `translate_to_parent` 732 ns
  (the SPK tree stores 301-wrt-3 and 399-wrt-3, so geocentric Luna is
  two Chebyshev legs; each leg = indexed summary search + zerocopy
  re-derivation of the segment view from raw DAF bytes + the eval).
  satkit's 85 ns is a Moon fast path: DE440's native format stores Luna
  geocentrically, so `geocentric_state(Moon)` short-circuits to ONE
  Chebyshev evaluation over a preloaded dense coefficient matrix — no
  tree, no search, no per-call parsing. Net: 2 evals instead of 1, ~8x
  packaging overhead per eval, plus ~800 ns/call of frame-tree
  resolution satkit never does. All inside anise's `translate`; the only
  crate-side lever is caching results (the per-step third-body cache
  already anticipated for propagate).
- **GCRF↔ITRF is ~15x faster** than satkit's IERS-2010 series evaluation,
  and **geodetic ~5x faster** (Vermeille closed form vs satkit's
  approach).
- **SGP4 and `interp_batch` (the engine's per-frame trail path) are at
  parity** — the paths the windowed app would hit per frame are fine.
