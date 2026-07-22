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
  vs satkit's direct Chebyshev evaluation) and **TEME is ~20x slower**
  (root cause pinned below — anise triple-evaluates the full IAU-76/FK5
  series chain per call; NOT primarily graph overhead). Per-call costs
  are microseconds — only hot loops care.
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

## Slow-case root causes + candidate fixes (2026-07-22, source-verified)

Each slow row traced through the crate and dependency source (anise
0.10.4, sofars 0.6.1, reference satkit 0.18.1). Fixes are CANDIDATES —
none is scheduled; the recorded decision (CLAUDE.md) is no caching until
propagate cost is user-visible, and none of these rows sits on the
engine's per-frame hot path today.

### `ephemeris_luna_geocentric_state` — 26.9x (2.25 µs vs 84 ns)

Root cause (measured split in the bullet above): anise's `translate`
re-resolves the frame tree on EVERY call — `try_find_ephemeris_root`
re-sweeps every summary of both loaded SPKs (~125 ns), two
`ephemeris_path_to_root` walks (~780 ns total), then two Chebyshev legs
(301-wrt-3, 399-wrt-3) each re-deriving the segment view from raw DAF
bytes (~730 ns/leg). Nothing is cached between calls; satkit
short-circuits Luna to one Chebyshev eval over a preloaded matrix.
Candidate fixes:

1. **Caller-level caching** (crate-side, no anise surgery): the per-step
   third-body cache anticipated for propagate; engine-side, a per-frame
   memo (one epoch serves the whole frame). Cheapest and already the
   sanctioned direction.
2. **Load-time fast path**: pre-resolve each `Body`'s segment chain in
   `AstroData::load` (the DE440 tree is static: 301→3, 399→3, planets→0)
   and evaluate the legs via anise's lower-level per-segment APIs,
   skipping root-find + path walk (~800 ns/call). The remaining per-leg
   packaging (summary search + zerocopy view re-derivation) would need a
   cached segment view per (segment, record) to close the rest.
3. **Upstream**: contribute root/path memoization to anise — both are
   invariants of the loaded kernel set; recomputing per call only serves
   post-load kernel swaps, which `AstroData` never does.

### `frames_qteme2gcrf` — 19.6x (8.55 µs vs 437 ns)

Root cause (anise `orientations/dynamic.rs`): for dynamic frames anise
builds the DCM's TIME DERIVATIVE by ±1 s central finite difference, so
`rotation_to_parent_dynamic` evaluates `teme_rot_mat` THREE times per
call — and each evaluation runs `nutm80` (the full 106-term IAU-1980
series via sofars `nut80`) + `pmat76` + `eqeq94`, and sofars' `eqeq94`
internally re-runs `nut80` AGAIN. Net: **6 full nutation-series
evaluations + 3 precession matrices per call**, plus per-call orientation
path resolution (`try_find_orientation_root` sweeps every BPC summary
block). Our `qteme2gcrf` then discards the derivative (positions-only
quaternion). satkit's 437 ns is Vallado's ~1-arcsec 2-term nutation
approximation + polynomial GMST — cheaper AND lower-fidelity, so the
ratio is not like-for-like (the harness measured 0.22" agreement with
the crate as the full-series side). Candidate fixes:

1. **Bypass `almanac.rotate` for TEME** (crate-side): build the matrix
   directly from sofars — `pmat76` + ONE `nut80` evaluation feeding both
   the nutation matrix and the equation of equinoxes
   (eqeq94 ≈ Δψ·cos ε + the 1994 correction terms). One series eval
   instead of six, no graph walk, no discarded derivative: est. ≥6x,
   landing near satkit's figure at full fidelity. Keep the anise path in
   the harness as the correctness oracle.
2. **Upstream**: a rotation-only variant of
   `rotation_to_parent_dynamic` that skips the finite-difference
   `rot_mat_dt` (the API currently always pays for it), and/or deriving
   eqeq from the already-computed Δψ inside `teme_rot_mat`.
3. **Accept**: TEME converts SGP4 output; one quaternion per epoch
   serves every satellite in the frame — 8.5 µs/frame is invisible.

### `kepler_from_pv` — 6.6x (291 ns vs 44 ns)

Root cause: per-call `frame_info(EARTH_J2000)` re-scans the planetary
dataset and rebuilds a `Frame` (~46 ns, measured); then anise `Orbit`
accessors recompute shared intermediates independently — `ecc()` builds
the full eccentricity vector (two cross products + guards), `sma_km()`
computes energy, `period()` recomputes `sma_km()` → energy again — each
behind `PhysicsResult` guard machinery, plus m↔km scaling and a
`Duration` round-trip. satkit is a single-pass closed form against a
compiled-in mu with zero lookups. Candidate fixes:

1. **Resolve the frame once in `AstroData::load`** (store the
   `EARTH_J2000` `Frame` — or just its mu — on `AstroData`): kills the
   per-call scan for free with no accuracy change.
2. **Single-pass local element computation**: a, e, T from r, v, mu is
   ~10 lines (h, ξ = v²/2 − mu/r, e-vec); keep anise solely as the mu
   source. Removes the redundant accessor chains and Result plumbing;
   should land within ~2x of satkit.
3. **Accept**: 291 ns is irrelevant at any plausible call rate; the
   `(1+e)/(1−e)` harness-bound note is about accuracy, not cost — the
   timing is state-independent.

### `propagate_leo_one_orbit` — 5.3x (2.17 ms vs 408 µs)

Attribution now closes numerically: with 4×4 + sun/moon + relativity,
every derivative evaluation makes 3 almanac calls (`EvalContext`
dedupes within an evaluation, nothing across): Sol translate + Luna
translate (~2.3 µs each, the ephemeris row's cost) + GCRF→ITRF rotate
for harmonics/omega (~2.9 µs, the qgcrf2itrf row) ≈ 7.5 µs/derivative.
2.17 ms / 7.5 µs ≈ 290 evaluations — consistent with DOP853 (12
stages/step) over ~24 steps for one LEO orbit at 1e-8. The anise calls
ARE essentially the whole gap; the force math itself (Pines 4×4,
Battin, Schwarzschild) is minor. satkit's per-evaluation cost is its
~84 ns-class direct Chebyshev plus closed-form frames. Candidate fixes:

1. **Per-step third-body caching** (the anticipated remedy, decision
   recorded: not until user-visible): Sol/Luna geocentric states are
   smooth over a step — cache per step, or better fit a local
   Chebyshev/Hermite over the arc from a few anise samples and evaluate
   per derivative in ~ns. Same treatment fits the Earth rotation pair
   (the BPC angles are equally smooth). Ceiling if all three calls are
   interpolated: ~0.3 ms/orbit-class, i.e. parity or better.
2. **Cheapen the per-call constant** via the ephemeris/frames fixes
   above (load-time path resolution) — helps ~2-3x without any
   caching-policy change.
