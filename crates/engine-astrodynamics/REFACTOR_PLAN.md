# engine-astrodynamics refactor: satkit → hifitime + anise + differential-equations

**Status: P0–P6 landed (2026-07-21). P6: NRLMSISE-00 drag via the pinned
tobari crate, driven through its low-level numeric input (no arika types
enter the crate); crate-owned space-weather table parsed from the embedded
`SW-All.csv` (header-indexed columns, contiguity-checked, OBS/INT rows
only - the first PRD row ends the usable span and later epochs FAIL LOUDLY
per the observed-only policy); previous-day F10.7 + centered 81-day mean +
7-element 3-hourly Ap history per the model's conventions; geodetic inputs
via the crate's own WGS84; co-rotation `v_rel = v - omega x r` with omega
from the skew of `Rdot^T R` (the same BPC-driven rotation the harmonic
field uses, shared once per evaluation through the EvalContext); 1000 km
ceiling skips the term entirely. AtmosphereModel registry: vacuum default,
Earth -> NRLMSISE-00. Verified: |omega| = sidereal rate about +Z,
retro/prograde drag asymmetry ~1.29, solar max/min density contrast > 3x,
6 h at 300 km shrinks the semi-major axis in the predicted envelope.
Next: P7 (KS + switching + segments). Earlier: P0–P5 landed (2026-07-21). P5: SRP with conical-penumbra shadow
(spec §4.4/4.5; spherical occulters, oblateness deferred as documented),
albedo + thermal IR single-disk model with the sanctioned in-crate Bond
albedo table (§4.6), and shadow-boundary event detection - two separate
signed edge functions per occulter family (a single product form is
sign-blind when one integrator step swallows the whole ~10 s penumbra
transit), stop-and-restart segment driver with a 20 ms plain-solve guard
hop past each located root, and event states re-derived by mini-solve
because the upstream interpolant's off-midpoint error would otherwise
inject tens of meters at each restart. Boundary epochs are recorded on
`Propagation::shadow_boundaries()` (telemetry). Restart accumulation is
tolerance-proportional (measured: ~7 m over 16 restarts at 1e-8, mm at
1e-11). `Settings.spacecraft = None` keeps the exact P4 path. Next: P6
(drag). Earlier phases: P0–P4 landed (2026-07-21) — satkit is OUT of the shipped crate
(survives only in the tests/ harness as the reference, seeded by the
harness's own `data::seed_satkit`; its assets moved to `tests/src/build.rs`
with `build = "src/build.rs"` dodging the parent's tests/*.rs
auto-discovery). P4 core: canonical units, `GravityField`/`ForceModel`
registries, Cowell on DOP853 (spec tests §7.1/2/7/11/12 in-crate), Battin
third-body, Schwarzschild, EGM2008 normalized-Pines + frequency-independent
degree-2 solid tides (§7.13/14), crate-owned quintic/cubic-Hermite
`Trajectory`, facade with `spacecraft: Option<SpacecraftModel>`. Harness:
full-model 1-day LEO vs satkit ≤ 50 m, reduced ≤ 10 m, backward ≤ 50 m.
Upstream findings pinned in code: differential-equations 0.6.1's DOP853
interpolant is only accurate at step MIDPOINTS (knots use
dense_points_per_step = 2; ~0.87 m at quarter-points measured) and its
DenseSolout rejects backward spans (backward arcs integrate as negated
forward problems, spec §7.7 fallback). P4-gate perf (release, warm, 1-orbit
LEO 4x4): propagate ~75 ms vs satkit ~0.5 ms (anise per-eval
translate/rotate overhead — P8 profiles before any caching, spec §9);
interp_batch 26 us vs 31 us (the engine's per-frame path is FASTER).
Earlier-phase status/deviations: P1 ephemeris has true-SSB barycentric
semantics (satkit returns raw DE storage — geocentric Luna) and the harness
compensates satkit's TT-as-TDB ephemeris argument; P2 kepler bounds scale
as (1+e)/(1-e) x the 1.6e-8 mu-provenance gap and the pre-1972 frames
comparison pins the predicted 57.7" rubber-second divergence; P3 corrected
the fixture TLE checksums (satkit never validated them). Next: P5.**

Companion spec: `deep-space-propagator-spec-revised.md` (Rev C) governs the
propagator internals (formulations, force models, integrator policy,
validation thresholds — not restated here). This plan covers the whole
crate: every module's backend swap, the data pipeline, sequencing, and the
verification harness. Crate versions, APIs, URLs, and sizes below were
verified live on 2026-07-20.

## 0. Mandate and end state

- Replace every satkit code path in `engine-astrodynamics` with crate-owned
  implementations over **hifitime** (time), **anise** (ephemeris, frames,
  body constants), and **differential-equations** (numerical integration),
  building the spec's deep-space propagator as the new `propagation` core.
- **Standalone**: `engine`, `engine-macros`, and the root bin are untouched
  and keep calling satkit directly. Nothing here changes their builds.
- **Breaking this crate's API is sanctioned** (owner, 2026-07-20): the time
  re-exports become `hifitime::{Epoch, Duration, TimeScale}` (satkit
  `Instant` gone), and signatures may change shape. The engine migrates to
  hifitime later, against whatever surface this refactor lands.
- End state: `satkit` appears nowhere in the shipped crate. It survives only
  inside `engine-astrodynamics-tests` as a reference implementation (that is
  the harness's whole job), alongside future non-satkit references.
- The *"`init` must never share a process with the engine's `init_satkit`"*
  constraint dissolves for this crate: anise has no process-global state
  (`Almanac` is a plain `Clone` struct; verified — no statics). The
  constraint lives on only inside the test harness while it references raw
  satkit (§7).

## 1. Target stack (verified against crates.io/docs.rs/repos, 2026-07-20)

| Crate | Version | Role |
|---|---|---|
| `hifitime` | 4.3.0 | `Epoch`/`Duration`/`TimeScale`, re-exported as the crate's time types. Integer-backed (centuries + ns). Leap-second table embedded by default; TDB/TT/UTC/TAI built in. The `ut1` feature is NOT needed — Earth rotation comes from the binary PCK, not from UT1 math. |
| `anise` | 0.10 (0.10.4) | DE440 ephemeris, GCRF↔ITRF93 and TEME rotations, per-body μ/radii/flattening from `pck11.pca`. Requires hifitime ^4.3. No process globals. |
| `differential-equations` | =0.6.1 (pin exact) | DOP853 (`ExplicitRungeKutta::dop853()`), adaptive, event detection (Brent–Dekker on the interpolant), backward integration (`tf < t0`) supported. API churned 0.5→0.6 (`ODEProblem` → `IVP` builder) — pin exact and firewall all imports in one internal module. |
| `sgp4` | 2.4 | SGP4/TLE backend (pure Rust, Celestrak/Vallado-validated: <2e-7 km vs the reference C++ at 3.5 y past epoch). Brings a mandatory `chrono` dep (default-features off). |
| `tobari` | =0.2.0 (pin exact) | NRLMSISE-00 for Earth drag. Pure Rust clean-room, validated against pymsis (official Fortran) fixtures + an Orekit oracle; space weather injected via a provider trait (fits our embedded table). Young crate — risk + fallback in §8. |
| `glam` | 0.33.1 (kept) | Public-API vector/quat types, unchanged: `DVec3`/`DQuat`, meters. |
| `nalgebra` | 0.35 (via anise), 0.34 (via d-e) | Internal math only. **The two target crates disagree**: anise pins `=0.35`, differential-equations wants `^0.34.2` — two nalgebra copies compile. Acceptable: neither type reaches our public API (glam there), and the only crossings are explicit component copies at the anise/integrator boundaries. Revisit when d-e bumps to 0.35. |

Facts that shaped the design (each verified):

- **anise ships TEME natively** — `EARTH_TEME_LEGACY_FRAME` (IAU-76/FK5
  precession + 1980 nutation, the SGP4-matching convention; use THIS one,
  not the IAU2006-class `EARTH_TEME_FRAME`) as an analytic dynamic frame,
  no kernel needed. The feared hand-rolled TEME chain is unnecessary.
- **anise has no SGP4** (and neither does nyx) — hence the `sgp4` crate.
- **`Orbit::period()` returns `Ok(Duration::ZERO)` for hyperbolic states**,
  it does NOT error. The kepler module must gate on `ecc()`/`sma_km()`
  itself to preserve the e ≥ 1 → `Err` contract the engine's
  `orbit_shape()` `None` fallback relies on.
- **differential-equations retains no interpolant after `solve()`** —
  `Solution` is discrete `(Vec<t>, Vec<y>)`; dense output exists only
  *during* the solve (`.dense(n)` / `.even(dt)` / `.t_eval(times)` solout
  modes). Post-hoc `interp`/`interp_batch` (which the engine's trail
  sampling requires) must be crate-owned — §5, `Trajectory`.
- **anise DAF-from-bytes requires an 8-byte-aligned base** (zerocopy
  `Ref<[u8],[f64]>` cast, errors on misalignment, no copy fallback). Our
  existing `Align8` static wrapper is exactly right — keep it for the
  `.bsp`/`.bpc` embeds and use `SPK::from_static` / `BPC::from_static`
  (zero-copy). `.pca` is DER-encoded, no alignment need
  (`PlanetaryDataSet::try_from_bytes`).
- **No NRLMSISE-00 crate exists under any obvious name** (`nrlmsise00` et
  al. are all 404 — the spec's §9 suggestion doesn't exist). Real options:
  `tobari`, satkit's pure-Rust port (disqualified: the point is removing
  satkit), or an in-crate port of the public-domain C reference. §9.

## 2. Embedded data pipeline (`build.rs`)

Same mechanics as today (download once into `OUT_DIR`, `include_bytes!`,
delete-to-refresh, `cargo::rerun-if-changed` per file), new asset table:

| Asset | Source | Size | Replaces |
|---|---|---|---|
| `de440s.bsp` | `https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/planets/de440s.bsp` | 31 MiB | (with `de440.bsp`) `linux_p1550p2650.440` (98 MiB) |
| `de440.bsp` | `https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/planets/de440.bsp` | 114 MiB | (with `de440s.bsp`) full 1550–2650 span |
| `earth_1962_250826_2125_combined.bpc` | `https://naif.jpl.nasa.gov/pub/naif/generic_kernels/pck/…` | 30 MiB | `EOP-All.csv` + `tab5.2a/b/d.txt` |
| `pck11.pca` | `http://public-data.nyxspace.com/anise/v0.10/pck11.pca` | 38 KiB | (new — body constants) |
| EGM2008, packed | ICGEM `https://icgem.gfz.de/getmodel/gfc/…/EGM2008.gfc`, truncated | ~1 MiB embedded | `EGM96.gfc` |
| `SW-All.csv` | `https://celestrak.org/SpaceData/SW-All.csv` | ~3 MiB | (new — drag space weather, added in P6) |

Notes, all verified:

- **`de440s.bsp` carries the exact same 14 segments as the 114 MiB full
  `de440.bsp`** (barycenters 1–9, Sun 10, Mercury 199, Venus 299, Moon 301,
  Earth 399 — outer planets exist only as system barycenters, same as the
  `.440` satkit reads today), span 1849–2150. **Owner decision (§9-Q1):
  embed BOTH files** and select by epoch at query time in the context —
  within 1849–2150 the two are byte-identical (de440s is an excerpt of
  de440), so selection is pure coverage: `de440s` serves the overlap, full
  `de440` (1550–2650) serves everything outside it — headroom for the
  future deep-space scenes committed in §9-Q6. Scenes stay EOP-gated to
  1962→build date regardless; the wide span is for non-scene propagation.
- **`earth_latest_high_prec.bpc` starts at 2000-01-01 — it cannot serve
  this app.** The combined kernel spans 1962→2125 (historical accuracy
  <3 µrad ≈ 0.6″; low-accuracy predict tail past its last datum) and is the
  only single kernel covering 1962→build date. Type-2 binary PCK, frame
  class 3000 = ITRF93 (parsed from the file and confirmed). Caveat: **NAIF
  renames it roughly annually** (`earth_1962_<lastdatum>_2125_combined`).
  Pin the filename as a const with a comment documenting the bump
  procedure; a stale kernel still covers old scenes (only the predict-tail
  quality of very recent epochs staled), so this is maintenance, not
  breakage. The 12 MiB historical-only variant (`earth_620120_*.bpc`) was
  rejected: it ends at its last datum, leaving scenes between the annual
  refresh and the build date with NO coverage (anise errors outside a
  BPC's span — no extrapolation).
- **EGM2008 streams-then-truncates**: the ICGEM `.gfc` is 252 MB text, but
  records are sorted ascending by degree (verified), so `build.rs` streams
  the response and aborts after degree 360 (~7 MB transferred), then packs
  fully-normalized C̄/S̄ into a little-endian f64 binary (~1.05 MiB) with a
  header carrying the model's own defining constants (verified from the
  file header: GM = 3.986004415e14, a = 6 378 136.3 m, tide-free,
  fully normalized) — which the gravity model must use instead of the
  canonical-unit μ, per spec §4.1. ICGEM's hash-URL has no permanence
  guarantee (it survived their domain move to `icgem.gfz.de`) — §8.
- **`SW-All.csv`** (1957→present + predictions) verifiedly carries every
  NRLMSISE-00 input: daily F10.7 (`F10.7_OBS`), centered 81-day mean
  (`F10.7_OBS_CENTER81`), daily Ap (`AP_AVG`), and the 3-hourly `AP1..AP8`,
  plus an OBS/PRD flag for the observed-only policy (spec §4.7: fail
  loudly outside observed data).
- The nyxspace mirror is HTTP-only (its HTTPS cert mismatches); prefer the
  NAIF/ICGEM/CelesTrak HTTPS URLs, use HTTP for the tiny `pck11.pca` (or
  pin its published crc32 `0x1edb3eac` via `check_then_parse`).
- The satkit-format downloads (`.440`, `tab5.2*`, `EGM96.gfc`,
  `EOP-All.csv`) **move to a new `tests/build.rs`** when satkit leaves the
  shipped crate (P4) — the harness still embeds and seeds them for its
  reference side. Until P4 they stay in the parent `build.rs` beside the
  new assets (both stacks coexist; ~160 MiB OUT_DIR peak, transient).
- Net shipped-crate embed size: ~180 MiB vs ~100 MiB today — the
  dual-ephemeris embed (§9-Q1) dominates; owner-accepted for the wider
  usable span.

## 3. Initialization: from set-once globals to a lazy context

satkit's four process-wide one-shot stores forced today's `init()`-first
API and the process-exclusivity rule. anise needs none of that.

- `data.rs` becomes the embed site plus a crate-internal
  `static CONTEXT: LazyLock<Context>` where `Context` holds the composed
  `Almanac` (both SPKs + BPC + PCA, built via the
  `from_static`/`try_from_bytes` parsers over `Align8` statics — with the
  §9-Q1 epoch-based de440/de440s selection at the query boundary) and the
  unpacked EGM2008 table; the space weather table gets its own `LazyLock`
  (only drag touches it).
- Query modules keep their **free-function API** (recommended, §9-Q4):
  call sites stay `ephemeris::geocentric_pos(body, epoch)`-shaped, which
  keeps the eventual engine migration mechanical. Functions reach the
  context internally.
- `pub fn init()` survives as an eager warm-up (parse kernels now, not at
  first frame) and stays the documented entry point, but is no longer
  *required* before queries, is idempotent, and carries no process
  constraint. Panics on parse failure = broken build, same as today.

## 4. Module-by-module mapping

Public types stay glam + SI meters; anise speaks km — convert once at the
anise boundary (and once more into canonical units inside the propagator,
spec §1). nalgebra never appears in a public signature.

| Module | Today | After |
|---|---|---|
| `lib.rs` re-exports | `satkit::{Duration, Instant, TimeScale}` | `hifitime::{Duration, Epoch, TimeScale}`; every API takes `Epoch` by value (it is `Copy`) |
| `ephemeris` | `satkit::jplephem` | `almanac.translate(target, observer, epoch, Aberration::NONE)`; geocentric = observer `EARTH_J2000`, barycentric = observer `SSB_J2000` (anise treats J2000 orientation ≡ GCRF/ICRF, matching satkit's output frame). `Body` enum unchanged; NAIF mapping Sol→10, Mercury→199, Venus→299, EMB→3, Luna→301, Mars…Pluto→4…9 (system barycenters — same semantics as today's satkit/DE440 lookups, documented). km→m at the boundary. |
| `frametransform` | satkit IERS-2010 + EOP | `almanac.rotate(from, to, epoch)` → DCM → `DQuat`. GCRF↔ITRF from the 1962 combined BPC (ITRF93); TEME from `EARTH_TEME_LEGACY_FRAME` (the SGP4 convention — not the IAU2006 variant). Positions-only contract unchanged (DCM's `rot_mat_dt` is there if velocity transforms are ever wanted). |
| `itrfcoord` | satkit `ITRFCoord` | In-crate WGS84 geodetic conversion (Heikkinen/Vermeille closed form, ~30 lines, existing unit tests carry over). Deliberately NOT anise's `latlongalt()`: that uses the pca ellipsoid (a = 6 378 136.6 m, IAU) — the engine's `planet.rs` and the current API are WGS84 (a = 6 378 137). |
| `kepler` | `satkit::Kepler` | anise `Orbit` (`CartesianState::new(…, EARTH_J2000)` with μ from the pca): read `ecc()`/`sma_km()`, **err when `ecc ≥ 1` or `sma ≤ 0` before touching `period()`** (which silently returns `ZERO` for hyperbolic — the satkit-era `Err` contract must be re-imposed by us). |
| `tle` | `satkit::tle::TLE` | `sgp4::Elements::from_tle(name?, l1, l2)` + `Constants::from_elements` **built at parse time** (element errors surface at load, strictly earlier than satkit's propagate-time). Epoch: `elements.datetime` (UTC) → `Epoch` once at parse. `mean_motion_rev_day` from the parsed elements. |
| `sgp4` | `satkit::sgp4` | `constants.propagate(MinutesSinceEpoch((t − epoch) in minutes))` per instant; TEME km→m. **`&mut Tle` disappears** (the sgp4 crate's `Constants` is immutable — satkit's cached-propagator quirk is gone; engine note for later). Reinstate the reference implementation's decay check ourselves (radius < 1 Earth radius → `Err`): the sgp4 crate propagates sub-surface states without complaint, and the module's contract is "never a silently garbage state". |
| `propagation` | `satkit::orbitprop` | The spec's propagator (§5 below) plus a compat facade preserving today's surface. |
| `data` | seeds 4 satkit stores | §3 context. |

## 5. The propagator (spec Rev C) — crate-integration decisions

The spec is authoritative for formulations, force models, switching policy,
and validation thresholds. Decisions the spec left to the implementer, now
made:

- **Module layout**: `propagation/` grows submodules — `units` (canonical
  units + newtypes, spec §1), `forces/` (`ForceModel` trait; central
  gravity point-mass + EGM2008-Pines with degree-2 solid tides; third-body
  Battin; SRP; conical shadow; albedo/IR; drag; Schwarzschild), `bodies`
  (the `NaifId → GravityField/AtmosphereModel` registries with universal
  point-mass/vacuum defaults, spec §4.0), `formulation/` (`Formulation`
  trait, `cowell`, `ks`, hysteresis switching), `integrator` (the ONLY
  module importing differential-equations — its 0.x API churn stays
  contained), `segment` + `trajectory`, `spacecraft` (cannonball).
- **State types**: `SVector<f64, 6>` (Cowell) / `SVector<f64, 10>` (KS)
  via differential-equations' `nalgebra` feature (0.34). Force-model math
  in glam `DVec3` (the crate convention); explicit converts at the
  integrator and anise (0.35) boundaries only.
- **Time plumbing**: integrator variable = canonical time offset from the
  segment-anchor `Epoch` (spec §5); every ephemeris query converts offset →
  `Epoch` (TDB handled by hifitime/anise internally).
- **Dense output / `Trajectory`** — the differential-equations gap (§1)
  makes this crate-owned by necessity, which spec §5 wanted anyway
  ("stitches segments and interpolates for arbitrary-epoch queries"):
  capture knots during the solve with a dense solout (the crate's own
  7th-order interpolant supplies intermediate points per accepted step),
  store `(t, y, ẏ)` knots per segment (ẏ from the derivative function),
  interpolate quintic-Hermite (position) / cubic-Hermite (velocity)
  between knots. Gate: interpolated states vs a direct `t_eval` solve of
  the same arc must sit well below the §0 accuracy target; knot density is
  the tuning knob if not.
- **Events**: shadow boundary, formulation-switch triggers (with hysteresis
  bands), and SOI central-body switches as differential-equations `Event`s
  (sign-change + precise location) that terminate the segment; the segment
  driver converts state, re-anchors, restarts — never integrates through a
  discontinuity (spec §5).
- **Atmosphere co-rotation** (`v_rel = v − ω×r`, spec §4.7): take the body
  rotation from the anise DCM time derivative (`rot_mat_dt`) rather than a
  scalar ω — same source, no hand-rolled rate constant.
- **Bond albedo**: NOT in the pca (verified — anise has no albedo anywhere),
  so spec §4.0's "albedo from ANISE" is unsatisfiable as written. Deviation:
  one in-crate `NaifId → Bond albedo` const table next to the registry,
  documented as the sanctioned exception.
- **EGM2008 tide system**: the ICGEM file is tide-free; the frequency-
  independent degree-2 solid-tide correction (spec §4.1) is applied
  consistently on that baseline.
- **Compat facade** (what the engine will migrate onto, kept thin over the
  real propagator): `OrbitState` (GCRF m, m/s), `Settings`
  { gravity_degree/order (now EGM2008, ≤360), abs/rel_error,
  use_sun_gravity, use_moon_gravity, use_relativistic_correction,
  **new** `spacecraft: Option<SpacecraftModel>` }, and
  `propagate(&state, begin, end, &settings) -> Propagation` with
  `state_end`/`interp`/`interp_batch`/`time_begin`/`time_end` intact
  (backward spans stay supported — d-e handles `tf < t0`).
  `spacecraft: None` (default) = SRP/drag/albedo skipped, matching today's
  behavior for the engine's parameter-less tracked satellites; geocentric
  canonical units; Cowell only (LEO never trips the §3a triggers).
- **Switch telemetry** (spec §3a "log every switch"): recorded on the
  `Trajectory` as segment boundaries with cause — no logging dep.
- **Spacecraft params** (owner, §9-Q3b): nullable end to end — a scene
  defines a `SpacecraftModel` per body at creation, or passes `None` and
  gets today's parameter-less behavior (SRP/drag/albedo skipped). Keep
  spec §4.3's direction-taking `fn area(&self, direction) -> f64`
  signature even though the cannonball ignores the argument: the stated
  future is a wgpu-computed projected-area model for SRP/drag/albedo,
  which slots in behind exactly that interface.

## 6. Phases and gates

Each phase lands green (`cargo test --workspace`, clippy warning-free,
nightly fmt) and is a natural commit/PR boundary. Spec §7 test numbers in
parentheses. Modules swap one at a time, so satkit and the new stack
coexist inside the crate until P4 — that is deliberate.

- **P0 — foundations.** New deps; `build.rs` gains the anise-format assets
  (keeps satkit's); `data.rs` grows the `Context` beside the satkit seeds;
  one inline sanity test (Luna geocentric via Almanac vs satkit, loose
  bound). Public API untouched.
- **P1 — time + ephemeris.** Re-exports flip to hifitime (**the breaking
  moment** for the harness); `ephemeris` moves to anise. Harness ephemeris
  comparisons re-tolerance per §7.
- **P2 — frames + geodesy + kepler.** `frametransform` → `almanac.rotate`
  (ITRF93 + TEME-legacy), renamed `frames`; `itrfcoord` → in-crate WGS84,
  renamed `geodetic` (§9-Q5 — renames land here, while each module is
  being rewritten anyway); `kepler` → anise `Orbit` with the e ≥ 1 gate.
  Harness grows all three comparisons.
- **P3 — TLE/SGP4.** `sgp4` crate backend, `&mut` dropped, decay check
  reinstated. Harness: satkit-vs-crate SGP4 over a ±1-week grid.
- **P4 — propagator core + facade; satkit exits the crate.** Spec build
  order §8 steps 1–5 reordered for this repo's needs: `units` (12) →
  registries/traits → Cowell on DOP853 (1, 2, 7) → third-body Battin →
  Schwarzschild (11) → EGM2008 loader + Pines + solid tides (13, 14) →
  `Trajectory` dense layer → facade. (SRP/shadow — spec step 6 — moves
  after: the facade doesn't need it, and satkit's exit shouldn't wait on
  the riskiest force model.) Then: satkit leaves `Cargo.toml`, the seeds
  leave `data.rs`, satkit-format assets move to `tests/build.rs`, the
  harness seeds satkit itself (§7). Gate: facade-vs-`orbitprop` bounds
  (§7), plus a two-body-config cross-check at near-machine agreement.
- **P5 — SRP + conical shadow + events; albedo/IR** (8; spec §4.4–4.6).
  Bond-albedo table lands here.
- **P6 — drag** (15): `SW-All.csv` download + parse, observed-only policy,
  tobari behind the `AtmosphereModel` registry, geodetic + co-rotation via
  `rot_mat_dt`, altitude cutoff. Co-rotation A/B check per §7.15.
- **P7 — KS + switching + segments** (3, 4, 5, 6, 9, 10, 16): KS
  round-trip before any KS integration (spec §8.10), hysteresis + dwell,
  SOI central-body switch with acceleration-continuity check, multi-body
  genericity run (Earth/Mars/Jupiter/small body), long-arc ephemeris
  cross-check with Schwarzschild on.
- **P8 — polish.** Tighten harness tolerances to measured; profile the
  ephemeris hot path (caching only if a profile demands — spec §9); prune
  the transitional doc comments; update the crate-describing lines in
  `.claude/` docs (`CLAUDE.md`, `architecture.md`, `testing.md`,
  `simulation.md`'s two-initializers note, `build.md`'s "satkit-only twin")
  — source wins over stale rules in the interim.

P0–P4 are the satkit-replacement critical path. P5–P7 remain natural
pause points, but they are committed work, not optional: the owner chose
the full spec through P7 (§9-Q6) — deep-space scenes are planned.

## 7. Verification harness (`engine-astrodynamics-tests`)

The harness's job inverts: today it proves the wrappers delegate faithfully
(tolerance 1e-12); after, it proves independent implementations agree
within honest physical bounds.

- **Sequencing constraint (satkit is process-set-once):** while the parent
  crate still seeds satkit (P0–P3), the harness MUST keep initializing via
  `engine_astrodynamics::init()` — a second seeder in the same test process
  panics `AlreadyInitialized`. Only at P4 (crate init no longer touches
  satkit) does the harness gain its own embedded copies (`tests/build.rs`)
  and seeder. The lib.rs warning about sharing a process moves into the
  harness at that point.
- The harness's satkit starts at the workspace's 0.18 line (matching
  `engine`); the owner (2026-07-21) sanctioned bumping it (e.g. to 0.20)
  whenever a newer reference helps — the harness is a leaf, so the bump
  never touches the engine's own satkit.
- Starting tolerances (tighten to measured once running; a miss means a
  mapping/frame/unit bug until proven otherwise):

| Comparison | Bound | Rationale |
|---|---|---|
| Ephemeris pos/vel | 1e-9 relative | same DE440 coefficients both sides (`de440s` ⊂ `.440`); expected agreement ~1e-13 — headroom catches body-mapping/unit errors |
| `qgcrf2itrf` | 0.2″ rotation angle (1″ pre-1972) | ITRF93 BPC vs IERS-2010+CelesTrak EOP: different EOP sources/realizations; kernel's own historical claim is <3 µrad |
| `qteme2gcrf` / `qteme2itrf` | 0.5″ | both equinox-based IAU-76/FK5-class chains |
| geodetic, kepler | 1e-9 | same closed-form math + constants |
| SGP4 | 10 m over ±1 week from epoch | both Vallado-reference-validated |
| propagation facade | ~50 m over a 1-day LEO arc | EGM2008-vs-EGM96 + integrator differences; plus a matched two-body config at near-machine precision |

- The spec's §7 battery (1–16) lives as reference-free unit tests inside
  the crate itself (thresholds derived from the §0 target, recorded in the
  test code per spec); the harness only holds cross-implementation
  comparisons.

## 8. Risks and mitigations

- **differential-equations 0.x churn** (proven: 0.5→0.6 renamed the core
  type): exact pin + all usage confined to `propagation/integrator.rs`.
- **No post-solve interpolant** in d-e: crate-owned `Trajectory` (§5) —
  also the spec-required design anyway. Validated against `t_eval` truth.
- **tobari youth** (0.2.0, ~3 months old, single maintainer): exact pin;
  it sits behind the `AtmosphereModel` registry so the blast radius is one
  adapter. Fallback: in-crate port of the public-domain Brodowski C
  reference with its vendor test cases (bounded, ~1.5 kloc). NRLMSISE-00
  vendor cases run against whichever backend ships (spec §7.15).
- **Annual NAIF rename** of the 1962 combined BPC: pinned const +
  documented bump; staleness degrades only the predict tail, never
  breaks existing past scenes older than the kernel's last datum.
- **ICGEM hash-URL permanence unverified**: cached-once in `OUT_DIR`
  (existing refresh workflow); if it moves, re-locate via the
  `icgem.gfz.de` model listing — build-time-only exposure. (NGA's official
  mirror sits behind a WAF that 403s non-browser clients — unusable for
  builds.)
- **Two nalgebra versions** (0.34/0.35): compile-time cost only; no shared
  types cross the boundary. Watch d-e for a 0.35 bump.
- **Facade performance** vs satkit `orbitprop` (the engine samples trails
  per frame): measure at the P4 gate; spec §6/§9 sanctions third-body
  caching only off a profile.
- **`chrono` enters the tree** via sgp4 (mandatory dep, default-features
  off): boundary-only (TLE epoch extraction, converted to `Epoch`
  immediately); no chrono type in any signature.
- **Stale `SW-All.csv`** (downloaded once, cached in `OUT_DIR`): under the
  observed-only policy, drag at an epoch past the cached file's last
  observed datum fails loudly rather than silently extrapolating. Owner-
  accepted (2026-07-21) — delete-to-refresh is the remedy for now; revisit
  (e.g. an age check in `build.rs`) only if a scene actually trips it.

## 9. Owner decisions — resolved 2026-07-21

1. **DE440 span → embed BOTH `de440.bsp` and `de440s.bsp`**, selecting by
   epoch so every date gets the best available coverage. The two are
   byte-identical inside 1849–2150 (de440s is an excerpt), so the rule is
   pure coverage: `de440s` in the overlap, full `de440` (1550–2650)
   outside it. §2 table and embed-size note updated.
2. **`sgp4` crate confirmed** as the SGP4 backend (no in-crate port).
3. **`tobari` confirmed** for NRLMSISE-00; the in-crate port of the
   public-domain C reference stays the documented fallback (§8).
   **Spacecraft params are nullable and scene-defined**: each scene may
   supply a `SpacecraftModel` per body at creation or leave it `None`
   (SRP/drag/albedo skipped). Recorded future direction: a wgpu-computed
   projected-area model will replace the cannonball area for
   SRP/drag/albedo — the §5 facade and spec §4.3's direction-taking
   `area()` signature are shaped for it.
4. **Free functions confirmed** (over the lazy embedded context, §3).
5. **Renames sanctioned** — rename whatever makes sense while breakage is
   free: `itrfcoord` → `geodetic`, `frametransform` → `frames` (landing in
   P2); apply the same judgment to any other name the rewrite touches.
6. **Full spec through P7 confirmed** — deep-space scenes are planned, so
   KS, switching, segments, SOI handling, and albedo/IR all ship.

Also decided in the same round: a stale `SW-All.csv` is acceptable for now
(§8); the harness may bump its reference satkit past 0.18 whenever useful
(§7); segment/trajectory persistence (the spec's §9 serialization
question) stays out of scope until a use case appears (§10).

## 10. Explicitly out of scope here

- Any change to `engine`, `engine-macros`, or the root bin (their satkit
  usage, `init_satkit`, `Instant`-based clock — all future work, to be
  planned against the surface this refactor lands).
- Publishing, Python bindings, lunar PA/ME frames via
  `moon_pa_de440_200625.bpc` + `moon_fk_de440.epa` (natural follow-up: the
  engine hand-rolls IAU lunar orientation today; anise could own it —
  12.9 MiB kernel, deferred until the engine migration).
- Trajectory/segment persistence (spec §9's serialization-format
  question): nothing in the app stores propagated trajectories; deferred
  until a use case appears (owner, 2026-07-21).
- Saturn's rings, and anything in `backlog.md`.
