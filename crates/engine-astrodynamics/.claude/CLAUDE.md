# engine-astrodynamics

Standalone orbit-math crate, fully off satkit (completed 2026-07-21, phases
P0–P8): **hifitime** (time), **anise** (DE440 ephemeris, frames, planetary
constants), **differential-equations** (DOP853), **sgp4** (SGP4/TLE),
**tobari** (NRLMSISE-00), **sofars** (direct IAU-76/FK5 TEME chain — the
same SOFA crate anise uses, called once per query; see segments.rs notes
in BENCHMARKS.md), **zerocopy** (the DAF `[u8]→[f64]` cast at load), plus
a crate-owned deep-space propagator. This
file is the permanent record of the spec (Rev C), the refactor decisions,
and the session findings — the original spec/plan documents were deleted
after being folded in here. `engine` still calls satkit directly; its
migration onto this crate is future work (see "Engine migration notes").
Code comments citing "spec §N" refer to that Rev C spec; every cited
section's content is preserved in this file.

Public API: glam `DVec3`/`DQuat`, SI meters, `hifitime::{Epoch, Duration,
TimeScale}` re-exported, every `Epoch` by value (`Copy`). nalgebra and
chrono never appear in public signatures. No process-global state and NO
LAZY LOADING (owner decision 2026-07-22, replacing the earlier
`LazyLock`+`init()` design): `AstroData::load()` eagerly parses every
embedded kernel and table (almanac + EGM2008 + space weather, ~100 ms
once), and every data-dependent API function takes `&AstroData` as its
FIRST argument — `ephemeris::*`, `frames::q*`, `Kepler::from_pv`,
`propagate`. `sgp4`/`tle`/`geodetic` are pure and take no data. The caller
owns the one `AstroData`. Unit tests share one instance via the
`cfg(test)`-only `data::test_data()` (`OnceLock` — harness twin:
`tests data::astro()`); production paths have no global. The old "must not
share a process with the engine's `init_satkit`" rule is DEAD for this
crate; it survives only inside `tests/` (the harness seeds reference
satkit itself).

## Accuracy target and numeric policy (spec §0, settled — do not revisit)

- **10–100 m position error over multi-year arcs**, attributed to the
  propagator (numerics + implemented-model fidelity with correct inputs).
  Parameter/environment KNOWLEDGE error is outside the budget — for
  drag-dominated LEO the target is only meaningful in reconstruction mode
  (historical space weather); in prediction mode density uncertainty
  dominates within weeks.
- State f64 (resolves ~mm at Neptune; canonical units keep it comfortable);
  time = hifitime integer-backed `Epoch`/`Duration`, never flattened to f64
  absolute seconds — the integrator variable is always a canonical offset
  from a segment-anchor `Epoch`. `f128` out of scope; escalation path would
  be `twofloat`, never needed.
- Integrator order + force fidelity dominate the error budget by orders of
  magnitude over arithmetic precision — hence DOP853, not precision-chasing
  number types. Validation tolerances: 1e-12 for spec tests; facade default
  1e-8 (satkit-era parity for the engine).
- **Encke's method deliberately excluded** (a step-count optimization, not
  an accuracy one; DOP853 adaptivity recovers most of it; it carries a
  silent-failure mode — naive evaluation of near-cancelling inverse-cube
  differences instead of Battin's F(q)). Revisit only if profiling shows
  step-count-bound heliocentric cruise; it would be a third `Formulation`
  sibling.

## Canonical units (propagation/units.rs)

All integration in canonical units with mu = 1 for the central body:
`TU = sqrt(DU^3/mu)`, `VU = DU/TU`, `ACU = DU/TU^2` (never named "AU" —
that collides with the astronomical unit used by SRP). Geocentric facade:
DU = the pca's Earth equatorial radius, mu from the pca — derived from the
SAME mu the dynamics use, or mu = 1 is silently violated. Unit discipline
is by naming convention (`_can` vs `_m`/`_m_s`/`_s` suffixes), the spec's
sanctioned alternative to newtypes. anise speaks km: convert km→m once at
the ephemeris boundary, SI→canonical once at the propagation boundary.
Reference scales: heliocentric TU ≈ 58.1324 d, geocentric TU ≈ 806.81 s.

## Formulations (propagation/formulation/)

**Cowell (`cowell.rs`, default)**: direct integration of `[r, v]`
canonical; the correctness oracle for everything else. `|r| < 1 m` errs
instead of producing NaN (a silent NaN inside DOP853 corrupts the whole
arc before anything notices — the poison-cell adapter in integrator.rs
enforces this since the solver's `diff` is infallible).

**KS regularization (`ks.rs`, switched in)**: full Kustaanheimo–Stiefel,
not Sundman-only — the 1/r^2 singularity is ELIMINATED (unperturbed motion
becomes the linear oscillator `u'' + (h/2)u = 0`), not just re-stepped.
State: 10 components `[u(4), u'(4), h, t]`, fictitious time s with
`dt/ds = r = |u|^2`. SIGN CONVENTION (classic KS bug if flipped):
`h = mu/r − |v|^2/2` is the NEGATIVE Keplerian energy — positive elliptic,
negative hyperbolic. The KS matrix used (L(u), first column convention):
`x1 = u1²−u2²−u3²+u4²; x2 = 2(u1u2−u3u4); x3 = 2(u1u3+u2u4)`;
`L(u)^T (w,0) = (u1w1+u2w2+u3w3, −u2w1+u1w2+u4w3, −u3w1−u4w2+u1w3,
u4w1−u3w2+u2w3)`; `L^{-1} = L^T/r`. Velocity: `v = 2 L(u) u' / r`;
inverse `u' = L(u)^T v / 2`. Cartesian→u branches on sign(x1) (the
standard Hopf-fiber choice): x1 ≥ 0 → `u1 = sqrt((r+x1)/2), u2 = y/2u1,
u3 = z/2u1, u4 = 0`; else `u2 = sqrt((r−x1)/2), u1 = y/2u2, u4 = z/2u2,
u3 = 0`. Perturbed EOM (derived; P = total accel + r/r³, i.e. everything
beyond the canonical mu=1 central term, so central-mu provenance gaps and
harmonics ride in P): `u'' = −(h/2)u + (r/2) L^T(P,0)`;
`h' = −2 u'·L^T(P,0)` (= −r v·P); `t' = r`. Bilinear (Levi-Civita)
constraint `l(u,u') = u4u1'−u3u2'+u2u3'−u1u4'` = 0 analytically, drifts
numerically; `bilinear_constraint()` exists as a health metric (runtime
monitoring/re-projection not wired — optional per spec). All conventions
live in ks.rs ONLY so they cannot mix. KS arcs return TIME-domain
Cartesian knots, so the trajectory layer is formulation-agnostic. KS is
forward-only (close-approach tool; backward arcs stay Cowell).

**Switching (`formulation/mod.rs`)**: enter KS when `e > 0.9` OR
(`e > 0.6` AND `r_p < 3 R_ref`); exit only when `e < 0.85` AND NOT
(`e > 0.55` AND `r_p < 3.5 R_ref`) — hysteresis bands are mandatory (bare
thresholds chatter and end up less accurate than either formulation run
straight). Deliberately NOT SOI-fraction-triggered: Earth's SOI (~924 Mm)
contains every Earth orbiter, which would make KS the de-facto formulation
for benign near-circular orbits. Triggers are integration events
(min/max-composed signed functions, positive while the current formulation
should continue); after each switch: convert state, dwell 1 TU with no
trigger events (the time guard), restart the integrator fresh — step-size
history is never carried across a switch. Switches are recorded as
telemetry (`SwitchRecord`); a high count is a bug signal. Thresholds are
constants in `trigger()` — spec wanted them configurable; residual.

## Force model (propagation/forces/, spec §4)

`a = central + third-body + relativity (+ SRP + albedo/IR + drag with a
spacecraft)`. Every term implements `ForceModel::acceleration_can(ctx, r,
v)`; terms that don't apply are SKIPPED, never evaluated-times-zero —
ephemeris queries are the hot path. `EvalContext` is built once per
derivative evaluation and caches (OnceCell) the Sol/Luna positions
relative to the segment's central body and the Earth rotation pair
`(DQuat gcrf→itrf, omega_gcrf)` — shared by tides, third-body, SRP, and
drag.

**Body-agnostic registries (§4.0, non-negotiable)**: central identity is a
NAIF id; `harmonics::field_for()` returns EGM2008 for 399 (degree ≥ 2)
else `PointMass`; `atmosphere::atmosphere_for()` returns NRLMSISE-00 for
399 else `Vacuum`. Adding a body-specific model = one new registry arm,
zero changes elsewhere (proven by the multi-body genericity test). Only
exception to "constants from the pca": the Bond-albedo table in
albedo_ir.rs — anise's pca carries no albedo anywhere (verified), so the
in-crate table is the sanctioned deviation. Note giant-planet J2 (~1.5e-2,
10x Earth's) is unmodeled: close giant flybys carry real documented error
until a J2-only field is registered.

**EGM2008 harmonics (`harmonics.rs`)**: fully normalized Pines evaluation,
regular at the poles (unnormalized recursions overflow f64 around degree
~85 — normalization is mandatory, not an optimization). Derivation used
(documented in the module): with s,t,u = r̂ components, `R_m + i I_m =
(s+it)^m`, derived Legendre `A_nm = P_nm/(1−u²)^{m/2}` (key identity
`A'_nm = A_{n,m+1}`), `q_n = (GM/r)(a/r)^n`:
`a1 = Σ (q_n/r) m A_nm E_nm`, `a2 = Σ (q_n/r) m A_nm F_nm`,
`a3 = Σ (q_n/r) A_{n,m+1} D_nm`,
`a4 = Σ (q_n/r) [(n+m+1)A_nm + u A_{n,m+1}] D_nm`,
accel = `(a1 − s·a4, a2 − t·a4, a3 − u·a4)` where `D = C R_m + S I_m`,
`E = C R_{m−1} + S I_{m−1}`, `F = S R_{m−1} − C I_{m−1}`. Normalized
pairing `A_nm C_nm = Ā_nm C̄_nm`; the one cross-order product needs
`Λ_nm = N_nm/N_{n,m+1}` = `sqrt(n(n+1)/2)` at m=0 else
`sqrt((n−m)(n+m+1))`. Ā recursions: `Ā00=1, Ā10=u√3, Ā11=√3` (δ_m0 breaks
the diagonal pattern at m=0→1, so Ā11 is explicit), diagonal
`Ā_nn = sqrt((2n+1)/2n) Ā_{n−1,n−1}`, sub-diagonal
`Ā_{n,n−1} = u sqrt(2n+1) Ā_{n−1,n−1}`, column
`Ā_nm = c1 u Ā_{n−1,m} − c2 Ā_{n−2,m}` with
`c1 = sqrt((2n+1)(2n−1)/((n−m)(n+m)))`,
`c2 = sqrt((2n+1)(n−m−1)(n+m−1)/((n−m)(n+m)(2n−3)))`. Evaluated with the
MODEL'S OWN defining constants from the packed header (GM = 3.986004415e14,
a = 6378136.3, tide-free, fully normalized) — never the canonical mu; the
two legitimately differ in low digits. Body-fixed evaluation: rotate
r into ITRF (the shared EvalContext rotation), evaluate, rotate the accel
back with the inverse — positions-only rotation is correct for a static
field. **Degree-2 solid tides** (frequency-independent, IERS 2010 eq 6.6):
`ΔC̄2m − iΔS̄2m = (k2m/5) Σ_j (GM_j/GM_E)(a/r_j)³ P̄2m(sinΦ_j)e^{−imλ_j}`
over Sun+Moon body-fixed positions, elastic Love numbers k20=0.29525,
k21=0.29470, k22=0.29801, applied on the tide-free baseline via a
Cell-stored delta refreshed per evaluation (the trait's time-dependence
hook). Normalized P̄20 = √5(3z²−1)/2, P̄21 e^{−iλ} = √15 z(x−iy),
P̄22 e^{−2iλ} = (√15/2)(x−iy)² over the perturber's body-fixed unit
vector. Ocean tides / frequency-dependent corrections are out of scope
(below the accuracy floor). GM ratios: Sun/Earth 332946.0487,
Moon/Earth 1/81.30056822149722 (DE440 EMRAT).

**Third bodies (`third_body.rs`)**: always point-mass, Battin's
cancellation-free form (when |r| << |r3| the direct and indirect terms
nearly cancel): `q = r·(r − 2r3)/|r3|²`,
`F(q) = q(3+3q+q²)/(1+(1+q)^{3/2})` (identically `(1+q)^{3/2} − 1`; texts
fold factors differently — take the whole formulation from one source),
`a = −mu3/|r−r3|³ (r + F(q) r3)`. Positions relative to the segment's
central body via `ephemeris::relative_state`.

**Relativity (`relativity.rs`)**: 1PN Schwarzschild, IERS form, β=γ=1:
`a = (mu/(c²r³))[(4mu/r − v²)r + 4(r·v)v]`, mu = 1 canonical, c converted
to canonical velocity once at construction. DEFAULT ON — DE440 dynamics
are relativistic, so ephemeris cross-checks diverge without it (Mercury:
43"/century ≈ 5.02e-7 rad/orbit, verified by test); the config switch
exists for A/B only. Lense–Thirring, de Sitter, third-body relativity out
of scope.

**Spacecraft (`spacecraft.rs`)**: cannonball — `A = πr²` from every
direction, no attitude state anywhere. `area_m2(&self, direction)` takes
the direction on purpose (owner decision): the stated future is a
wgpu-computed projected-area model behind exactly that signature. C_r ∈
[1,2] and C_d (~2.2 free-molecular sphere) are estimable parameters that
absorb the shape error. `Settings.spacecraft: Option<SpacecraftModel>` —
None (default) skips SRP/albedo/IR/drag entirely, the parameter-less
tracked-satellite behavior; scenes supply Some per body.

**SRP (`srp.rs`)**: `a = ν P_sun (AU/d)² C_r (A/m) r̂_sun` with
P_sun = 4.5398e-6 N/m² pairing with 1361 W/m² TSI (the legacy 4.56e-6
pairs with 1367 — never mix pairs), and r̂_sun FROM Sun TO spacecraft
(pushes away; sign stated once, tested). Occulters per segment: central
body always, Luna mandatory candidate for Earth orbiters; `ν = Π ν_i`
(exact while occulters don't overlap the solar disk). SRP significance
scales as `mu_p^(-1/3)` at fixed Hill-radius fraction — planet mass, not
solar distance, governs whether it matters (first-class at small bodies).

**Shadow (`shadow.rs`)**: conical with penumbra from apparent radii
(`asin(R/d)`) — full sun / penumbra (circle-circle lens overlap area over
the solar disk) / umbra / antumbra (annular residual `1 − (a_occ/a_sun)²`).
The Sun's ~0.267° disk makes the penumbra a smooth ramp; a cylindrical
model's C0 step wrecks DOP853's error estimator. Spherical occulters;
ellipsoidal-Earth refinement (boundary shifts tens of km ≈ seconds of
transit timing) deliberately deferred, as are refraction/ozone.

**Albedo + IR (`albedo_ir.rs`)**: both act radially OUTWARD (unlike SRP).
Minimum-viable single-disk model (spec-sanctioned; ring discretization is
the recorded upgrade): view factor `(R/r)²`, albedo flux
`a_B · S_local · view · (1+cosΦ)/2` (zero over the night side, hence
naturally zero through eclipse transit), IR from radiative balance
`(1−a_B)S_local/4 · view` (nonzero in eclipse — deliberate). S_local
scales 1361 W/m² by the body's actual solar distance. Together 10–30% of
SRP in LEO (tested); skipped below view factor 1e-6.

**Drag (`drag.rs` + `atmosphere.rs`)**: `a = −½ρ|v_rel|v_rel C_d (A/m)`
with `v_rel = v − ω×r` — omitting co-rotation is a classic ~465–490 m/s
error; ω comes from the rotation-matrix derivative (skew of `Ṙᵀ R` where
R: gcrf→itrf), the SAME BPC-driven rotation the harmonics use, never a
hand-rolled rate. NRLMSISE-00 via tobari's LOW-LEVEL `Nrlmsise00Input`
(plain numbers; its arika time types and provider plumbing never enter the
crate). Inputs honor the model's conventions: daily F10.7 = PREVIOUS day,
81-day mean CENTERED, 7-element 3-hourly Ap history to 57 h back, geodetic
(not geocentric) coordinates via the crate's own WGS84, local solar time
from UT + longitude/15. 1000 km validity ceiling skips the term (also the
performance cutoff — the model is the expensive part of a derivative).
Drag is non-conservative: energy checks must exclude drag-bearing arcs.
**Observed-only space weather policy (owner-accepted)**: the embedded
`SW-All.csv` is parsed by header-name indexing (drift-safe), contiguity-
checked, OBS/INT rows only — the first PRD row ends the usable span and
epochs outside FAIL LOUDLY (never silently substitute; delete the cached
file in OUT_DIR and rebuild to refresh).

## Integrator layer (propagation/integrator.rs) — the d-e firewall

`differential-equations = "=0.6.1"` EXACT PIN (0.5→0.6 renamed the core
builder); every `use differential_equations::` lives in integrator.rs so
0.x churn stays contained. nalgebra 0.34 for `SVector` state (anise
carries its own 0.35 — two copies compile, neither reaches public API).
Three UPSTREAM DEFECTS found and worked around (all pinned in comments):

1. **The DOP853 dense interpolant is only accurate at step MIDPOINTS**
   (measured ~0.87 m at quarter-step points vs ~5e-4 m at midpoints on a
   1e-12 two-body arc; worse at loose tolerances). Therefore
   `dense_points_per_step = 2` everywhere (endpoints + midpoints are the
   only trustworthy knots), and NOTHING may trust an interpolated state:
   event states are re-derived by a plain mini-solve from the last good
   knot (skipping this injected tens of meters per event restart), and
   event TIMES keep ~ms-level skew (fine for telemetry, never for
   dynamics).
2. **`DenseSolout` rejects backward spans** (its step-interval check
   assumes forward ordering). Backward arcs integrate as negated forward
   problems (`τ = t0 − t`, derivative sign flipped — the spec's own
   fallback), then knots are remapped/reversed and ẏ re-derived on the
   true axis.
3. **Events root-find on the flawed interpolant**, so a terminal event can
   fire EARLY (the KS arrival on the time component undershoots tf by up
   to ~1% of a step) or re-fire when restarted exactly on a root. Rules:
   drivers loop epsilon-closed until PHYSICAL time truly arrives (1e-9
   canonical slack) — never trust a single event; and after locating a
   root, hop past it with a short plain solve (20 ms guard for shadows —
   physically exact, and no shadow feature is narrower than the ~10 s LEO
   penumbra transit) before re-arming events. Also: the event solout is
   blind inside a solve's FIRST step (its previous-sample initialization),
   so restart-heavy drivers pass a small `h0` (~1 s).

Event pattern for shadows: TWO separate signed boundary functions per
occulter family — outer edge `g1 = sep − (a_sun + a_occ)`, inner edge
`g2 = sep − |a_occ − a_sun|`, min-combined across occulters, chained as
two terminal events (`.dense(2).event(&e1).event(&e2)` — wrappers compose,
inner solout runs first so the terminating step's knots past the crossing
must be truncated). A single product-form function is SIGN-BLIND when one
integrator step swallows the whole penumbra (full sun and umbra share the
sign) — this was a real bug. Segment driver: solve → record boundary →
guard-hop → restart; boundaries recorded on
`Propagation::shadow_boundaries()`. Restart error accumulation is
tolerance-proportional (measured ~7 m over 16 restarts at 1e-8, mm at
1e-11). Event termination = `Status::Interrupted`.

## Trajectory dense layer (propagation/trajectory.rs)

d-e retains NO interpolant after `solve()` (`Solution` is discrete t/y),
so the dense layer is crate-owned by necessity: knots `(t, r, v, a)` (a
recomputed via the derivative at each knot), quintic Hermite position
(r,v,a at both ends; basis H0..H5 = 1−10τ³+15τ⁴−6τ⁵, τ−6τ³+8τ⁴−3τ⁵,
τ²/2−3τ³/2+3τ⁴/2−τ⁵/2, 10τ³−15τ⁴+6τ⁵, −4τ³+7τ⁴−3τ⁵, τ³/2−τ⁴+τ⁵/2 with h
and h² weights), cubic Hermite velocity (v,a). Measured fidelity ~1.3 mm
worst vs analytic truth over an eccentric orbit — four orders under the
target; knot density is the knob. Endpoint queries get ~1 ms canonical
slack (clamped) so interpolating at exactly `end` never fails to roundoff.
`state_end` returns the exact solver knot (last ascending for forward,
first for backward), never an interpolation. Single stitched segment today
(arcs merged with junction-knot dedupe); a real multi-segment list waits
for SOI-driven central-body switching.

## Central-body switches / SOI (partially wired — residual)

The physical-acceleration continuity requirement is tested at machine
precision: total PHYSICAL acceleration (model accel + the center's own
model acceleration) is identical whether expressed about Earth (Sun as
third body) or about the Sun (Earth as third body) — algebraically exact
through Battin; any jump means central/third-body bookkeeping is wrong.
State re-expression uses `ephemeris::relative_state`; `Body::Terra` exists
so Earth can be a target/third body. A fully automated SOI-crossing driver
(radius-boundary event → re-center → new segment) is NOT composed yet —
all pieces exist and are tested individually; build it when a deep-space
scene needs it. A central-body change re-expresses the same physical
state; it is NOT a change of physics. Reduced-mass caveat proven by test:
relative motion about a center needs `mu_center + mu_orbiter` in the
central term (omitting Mercury's mu costs ~20 km/30 days vs the
ephemeris).

## Data pipeline (build.rs + data.rs)

Download-once into OUT_DIR, `include_bytes!`, per-file rerun-if-changed,
delete-to-refresh. Assets: `de440s.bsp` (31 MiB, 1849–2150) + full
`de440.bsp` (114 MiB, 1550–2650) — BOTH embedded (owner decision): they
are byte-identical in the overlap (de440s is an excerpt), and anise
verifiably searches SPKs newest-loaded-first with per-file epoch
fall-through, so LOAD ORDER IS THE SELECTION RULE (load de440 then de440s;
the excerpt serves the overlap, the full file the rest — zero
query-boundary code). `earth_1962_250826_2125_combined.bpc` (30 MiB,
ITRF93, frame class 3000): the ONLY single kernel covering 1962→build
date; NAIF renames it ~annually (`earth_1962_<lastdatum>_2125_combined`) —
bump the URL + delete cache; staleness only degrades the predict tail.
(`earth_latest_high_prec.bpc` starts at 2000 — cannot serve past scenes;
the historical-only variant ends at its last datum leaving recent scenes
uncovered — both rejected.) `pck11.pca` (38 KiB, HTTP-only mirror — pinned
by raw-byte crc32 0x1edb3eac via `crc32fast::hash`; anise's
`DataSet::crc32()` hashes only the inner payload, NOT file bytes — not
comparable). **EGM2008**: the ICGEM `.gfc` is 252 MB text sorted ascending
by degree, so build.rs streams and aborts after degree 360 (~8 MB
transferred; 'D' Fortran exponents normalized; header asserted
fully_normalized + tide-free + defining constants), packing into
`egm2008_n360.le64`: 40-byte header (magic `EGM2008\0`, u32 version=1,
u32 n_max, f64 GM, f64 a, u64 reserved) + C̄ then S̄ triangular arrays
f64 LE, (n,m) degree-major, n = 2..=360, index `n(n+1)/2 − 3 + m`. The
ICGEM hash-URL has no permanence guarantee — if 404, re-locate via the
icgem.gfz.de model listing. `SW-All.csv` (space weather; see drag). anise
DAF-from-bytes requires an 8-aligned base: the `Align8` wrapper covers the
`.bsp`/`.bpc` embeds (`SPK/BPC::from_static`); `.pca` is DER, no alignment
need. The kernel embeds MUST stay NAMED statics (the `aligned_kernel!`
macro): a promoted `&Align8(*include_bytes!(..))` temporary is an
anonymous allocation that rustc duplicates into every codegen unit
reading it — with both data.rs and segments.rs as readers that doubled
LLVM's constant memory and OOM-killed release builds (rustc peak ~16 GB;
~13.9 GB with named statics). anise is used with `default-features = false` (defaults pull
metaload/analysis: ureq/rayon/csv — unneeded for embedded parsing).

## Facade contract (propagation/mod.rs) + engine migration notes

`propagate(&AstroData, &OrbitState, begin: Epoch, end: Epoch, &Settings) →
Propagation { time_begin, time_end, state_end, interp, interp_batch,
shadow_boundaries }`; GCRF meters; backward spans first-class; zero-
duration returns the input exactly. (`Propagation` is self-contained —
interpolation needs no `AstroData`.) `Settings { gravity_degree/order
(EGM2008 ≤ 360; < 2 degrades to point-mass), abs/rel_error (default 1e-8),
use_sun/moon_gravity, use_relativistic_correction, spacecraft }`; solid
tides always on. Facade is Cowell-only (LEO never trips the switch
triggers). API differences the engine migration must absorb:
- **Explicit data**: the engine constructs one `AstroData` (via
  `AstroData::load()`, eager ~100 ms — do it at scene init, not per
  frame) and passes `&AstroData` as the first argument to every
  ephemeris/frames/kepler/propagate call. There is no `init()` and no
  global to warm.
- Time: `Epoch` by value everywhere; satkit `Instant` gone.
- `sgp4(&Tle, &[Epoch])` — `&mut` gone (Constants built at parse; element
  errors surface at LOAD, earlier than satkit); a sub-surface sample errs
  ("decayed", WGS72 radius 6378135 m) — the sgp4 crate itself happily
  returns garbage sub-surface states, and it VALIDATES TLE checksums
  (satkit never did; fixtures need real digits: digit sum with '-' = 1,
  mod 10).
- `Tle::name() → Option<&str>` (None for 2-line sets).
- `ephemeris::barycentric_*` are TRUE solar-system-barycenter states.
  satkit returned the RAW stored DE series — for Luna that is a
  GEOCENTRIC vector under a "barycentric" name. Engine code relying on
  the quirk must be fixed at migration.
- `kepler` errs for e ≥ 1 / a ≤ 0 by a crate-owned gate BEFORE `period()`
  (anise returns Ok(ZERO) for hyperbolic — the engine's `orbit_shape()`
  None fallback depends on this Err).
- `frames::{qgcrf2itrf, qitrf2gcrf, qteme2gcrf, qteme2itrf}` panic outside
  the BPC span 1962–2125 (infallible surface; scenes are EOP-gated well
  inside). TEME is anise's `EARTH_TEME_LEGACY_FRAME` (IAU-76/FK5 + 1980
  nutation, the SGP4 convention) — NOT the IAU2006-class
  `EARTH_TEME_FRAME`. anise DCM applies `v_to = rot_mat · v_from`; glam
  `DMat3::from_cols` takes COLUMNS — feed rows and you build the inverse
  rotation (the chirality test catches it: a fixed GCRF direction must
  sweep WESTWARD in ITRF).
- `geodetic` is crate-owned WGS84 Vermeille closed form (a = 6378137,
  1/f = 298.257223563) — deliberately NOT anise `latlongalt()` (pca uses
  the IAU ellipsoid a = 6378136.6, which would disagree with the engine's
  `planet.rs`).
- Body NAIF map: Sol 10, Terra 399, Mercury 199, Venus 299, EMB 3,
  Luna 301, Mars..Pluto 4..9 (system barycenters — DE440 has outer
  planets only as barycenters; same semantics as the satkit era).

Performance: measured criterion numbers live in `.claude/BENCHMARKS.md` —
the agent-facing cache layer, since the benches take minutes. READ that
file instead of re-running; UPDATE it any time a bench target is run; and
RE-RUN the affected targets (then update it) after any change that could
plausibly move performance (force model, integrator, trajectory/interp,
ephemeris/frame plumbing, dependency bumps). The benches are one criterion
target per domain in the harness:
`cargo bench -p engine-astrodynamics-tests --bench
<ephemeris|frames|geodetic|kepler|sgp4|propagation>` (omit `--bench` to
run all; sources at `tests/src/benches/*.rs`). Headline (2026-07-22, warm,
1-orbit LEO, default 4×4): `propagate` ~370 µs vs satkit ~409 µs —
FASTER, after the segments.rs fast-path landing (load-time
pre-resolution of every SPK/BPC segment + direct sofars TEME + one-pass
kepler; full record with root causes in BENCHMARKS.md). No dynamic
caching anywhere (owner constraint 2026-07-22): everything
epoch-independent is precomputed at `AstroData::load`, everything
epoch-dependent is recomputed exactly per call, and the fast paths are
pinned to the anise almanac by machine-agreement unit tests (the almanac
stays loaded as the oracle). The once-anticipated per-step third-body
cache is dead — exact evaluation is now cheap enough. `interp_batch`
(the engine's per-frame trail path) is at parity.

## Verification

In-crate: the spec's full validation battery as unit tests with recorded
budgets — two-body vs closed-form Kepler (near machine), energy/momentum
drift over 50 orbits (DOP853 is not symplectic; drift bounded by
tolerance), KS↔Cartesian round trip at machine precision (FIRST, before
trusting any KS integration), KS-vs-Cowell agreement + >2x step-count win
at e = 0.95, flyby three ways (Cowell/KS/switched must agree; the switched
run being the outlier means conversion/restart bugs), hysteresis
no-chatter, forward-backward round trip (also proves tf < t0), shadow
continuity/exactness + event epochs on boundary-function roots,
physical-acceleration continuity across a central-body re-expression
(machine), Mercury vs the ephemeris over 30 days with Schwarzschild
demonstrably required, Mercury perihelion 43"/cy
(`Δω = 6πμ/(c²a(1−e²))` per orbit), unit round-trips, EGM degree-0 ≡
point-mass + analytic J2 acceleration anchor (J2 = −√5 C̄20 ≈ 1.0826e-3)
+ J2 secular nodal regression (−3π J2 (Re/p)² cos i per orbit, 5%) +
Pines-vs-potential numerical-gradient consistency (an independent
normalized-Legendre potential evaluation, differentiated centrally) + pole
regularity + surface-g envelope, NRLMSISE plausibility / solar-cycle
contrast / co-rotation asymmetry (retro/pro ≈ (v+ωR)²/(v−ωR)² ≈ 1.29) /
orbit decay, multi-body genericity (Earth harmonics / Mars / Jupiter /
small body through one code path).

Harness (`tests/`, see also `.claude/rules/testing.md`): cross-
implementation comparisons vs reference satkit 0.18, which the harness
seeds ITSELF (`data::seed_satkit`, embedded copies via `tests/src/build.rs`
— the manifest `build` key dodges the parent's `tests/*.rs` auto-discovery
trap). Bounds are measured-and-dated (2026-07-21): ephemeris 1e-10 rel
(measured ~2.3e-12 worst), GCRF↔ITRF 0.05" (measured 0.015"), TEME 0.5"
(measured 0.22"), geodetic 1e-9 rad, kepler bounds scale as
`(1+e)/(1−e) × 1.6e-8` (the pca-vs-satkit mu provenance gap amplifies
into a and T at a fixed perigee state), SGP4 10 m over ±1 week,
propagation 10 m full model / 5 m reduced over 1 day (measured
1.63/1.09 m). **Reference-side satkit quirks the harness compensates
(proven from its source, do not re-derive)**: satkit evaluates jplephem at
TT, not TDB (±1.7 ms; the harness hands it an instant whose TT equals our
epoch's TDB so both sides evaluate the same argument); satkit
`barycentric_*` return raw DE storage (the harness assembles true-SSB Luna
as `EMB + r_geo · EMRAT/(1+EMRAT)`); satkit treats pre-1972 TAI−UTC as
ZERO while the BPC carries true rubber-second time — the 1965 frame
comparison PINS the predicted 57.7" divergence (15.041"/s × 3.836 s)
instead of asserting closeness; satkit never validated TLE checksums.
Reference-side times always go through the TAI-MJD bridge (~1.3 µs
roundoff), which stays valid pre-1972 where UTC labels diverge between
the libraries.

Second reference (added 2026-07-22): the **astrodyn 0.2 family** (pure-
Rust NASA JEOD port), `astrodyn` harness modules (one per domain test
file, sibling to each `satkit` module) + bench rows. It
shares anise with the crate, so its ephemeris comparison (same de440s
bytes via the harness's own `.bsp` embed) checks WIRING at near-machine
tightness (1e-13 bound, measured 2.2e-16); the genuinely independent
sides are JEOD's RNP (IAU-76/FK5 + Aoki GMST + polar motion, driven by
satkit's EOP through `support::astrodyn_rnp_inputs`; bound 0.2", measured
0.048"), Borkowski geodetic (1e-9 rad), JEOD elements (satkit-style
mu-provenance-scaled bounds; its |e−1| ≤ 1e-2 parabolic band nulls a — a
pinned contract difference, so element comparisons stop at e = 0.95), and
a mu-matched two-body day through the `astrodyn_runner` RK4 pipeline
(0.1 m bound, measured 7.4 mm — the crate's mu recovered from its own
`Kepler` output since the pca value is not exposed). astrodyn speaks
glam 0.30 (bridged in `support`) and one shared hifitime lets `Epoch`
cross directly. No astrodyn SGP4 (none exists) and no full-model
propagation comparison (its field is GGM05C, not EGM2008).

## Residuals (deliberate, revisit when needed)

Automated SOI segment driver (pieces tested, not composed); switch
thresholds / drag cutoff / Earth far-field point-mass fallback are
constants not knobs; KS constraint runtime monitor / re-projection
unwired; EGM2008 content-validated (header + C̄20) rather than
checksum-pinned; single-segment `Trajectory`; trajectory/segment
persistence out of scope until a use case appears (owner). Lunar PA/ME
frames via `moon_pa_de440_200625.bpc` are a natural engine-migration
follow-up (the engine hand-rolls IAU lunar orientation today).
