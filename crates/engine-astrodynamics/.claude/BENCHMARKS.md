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
satkit seeded, anise kernels parsed before measurement. Bench profile
(inherits release). Measured 2026-07-22.

## Results (2026-07-22)

| Group | crate | satkit | crate/satkit |
|---|---|---|---|
| `ephemeris_luna_geocentric_state` | 2.27 µs | 85.0 ns | 26.7x slower |
| `frames_qgcrf2itrf` | 3.14 µs | 46.2 µs | **14.7x faster** |
| `frames_qteme2gcrf` | 8.77 µs | 370 ns | 23.7x slower |
| `geodetic_from_itrf` | 69.9 ns | 328 ns | **4.7x faster** |
| `kepler_from_pv` (e = 0.7) | 299 ns | 43.8 ns | 6.8x slower |
| `sgp4_iss_113_samples` | 21.8 µs | 22.8 µs | parity |
| `propagate_leo_one_orbit` (4×4 + sun/moon + rel, 1e-8) | 2.18 ms | 413 µs | 5.3x slower |
| `interp_batch_500_samples` | 24.9 µs | 22.6 µs | ~parity |

## Reading the numbers (2026-07-22 analysis)

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
- **GCRF↔ITRF is ~15x faster** than satkit's IERS-2010 series evaluation,
  and **geodetic ~5x faster** (Vermeille closed form vs satkit's
  approach).
- **SGP4 and `interp_batch` (the engine's per-frame trail path) are at
  parity** — the paths the windowed app would hit per frame are fine.
