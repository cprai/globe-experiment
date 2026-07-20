# Deep-Space Orbital Propagator — Implementation Specification

**Target language:** Rust (stable)
**Core dependencies:** `hifitime`, `anise`, `differential-equations`, `nalgebra`
**Audience:** an implementing agent with no prior context on this design
**Revision:** C — accuracy target, Earth-arc scope, and relativity defaults
decided; supersedes Rev B

---

## 0. Scope and intent

Build a high-precision deep-space orbit propagator built around:

1. **Non-dimensionalized (canonical) units** for conditioning.
2. **Cowell's method as the default formulation** — direct integration of the
   full Cartesian state. Simple, robust, no failure modes of its own.
3. **Full KS regularization**, switched in automatically for close planetary
   approaches and high-eccentricity arcs, where Cowell's step size collapses and
   its error estimate degrades.
4. **A high-fidelity SRP model** — conical shadow with penumbra, plus planetary
   albedo and thermal IR.
5. **Regime-appropriate gravity** — EGM2008 spherical harmonics near Earth,
   point-mass near every other body.
6. **NRLMSISE-00 atmospheric drag** near Earth.
7. **A first-order relativistic (Schwarzschild) correction** — a few lines of
   code, on by default everywhere; needed at the accuracy target below and for
   ephemeris cross-checks (§4.8).
8. **A cannonball (spherical) spacecraft model** shared by SRP and drag, so no
   attitude state is needed anywhere.

**The propagator must work for any body in the solar system.** Earth-specific
models are runtime-selected plug-ins, never branches in shared code. See §4.0 —
this constraint outranks convenience everywhere it applies.

**Long-duration Earth-orbit propagation (LEO/GEO/MEO operational arcs) is in
scope** — a first-class use case, not just the departure and flyby phases of
deep-space missions. The formulation-switch triggers (§3a) are designed so
these arcs stay in Cowell.

### Accuracy target (decided)

**10–100 m position error over multi-year arcs**, attributed to the propagator
itself: numerics plus the fidelity of the implemented force models, evaluated
with correct spacecraft parameters (`C_r`, `C_d`, `A/m`) and correct
environment inputs. Parameter and environment *knowledge* error is outside the
propagator's budget — in particular, for drag-dominated LEO the target is
meaningful only in reconstruction mode (historical space weather); in
prediction mode, density forecast uncertainty dominates everything else within
weeks, and no propagator can promise 10 m over years there. Every §7 threshold
derives from this number; §9's per-regime knobs (truncation degree, drag
cutoff, albedo discretization) are tuned against it. The 1e-12 integrator
tolerances (§5) are justified by it: at this target, numerical error must stay
an order of magnitude below model error.

**Encke's method is explicitly NOT part of this build.** It is a step-count
optimization for weakly-perturbed cruise, not an accuracy technique, and DOP853's
adaptivity already recovers much of its benefit. It carries a silent-failure mode
(see Appendix A) and should only be revisited if profiling shows the propagator is
step-count-bound during long cruise arcs.

All of it runs on **DOP853** (8th-order Dormand–Prince, adaptive, dense output)
from the `differential-equations` crate, with event detection used to handle
shadow boundaries cleanly.

### Numerical-type decisions (already settled — do not revisit)

- **State (position/velocity): `f64`.** Floating point gives uniform *relative*
  precision across the enormous dynamic range of deep-space trajectories. f64
  resolves ~1 mm at Neptune's distance and ~3 mm at 100 AU, which is ~10 orders
  of magnitude finer than real ephemeris uncertainty. Do not use fixed-point
  integers for the dynamics.
- **Time: `hifitime`'s integer representation.** Time needs uniform *absolute*
  precision and exact, associative arithmetic. `hifitime::Epoch` and `Duration`
  store centuries + nanoseconds as integers. Never flatten epochs to f64 seconds
  for storage or accumulation.
- **`f128` is out of scope.** Still nightly-only in Rust as of 1.95, and
  software-emulated (10–100× slower). If f64 ever proves insufficient, the
  escalation path is the `twofloat` crate (double-double, ~106-bit mantissa),
  not quad. In practice canonical units (§1) keep f64 comfortably inside its
  precision budget, so neither escalation should ever be needed.

### The one thing that dominates accuracy

Integrator *order* and *force model fidelity* dominate the error budget by many
orders of magnitude over arithmetic precision. RK4 truncation error at any
practical step size dwarfs f64 roundoff. This is why the spec is built on
DOP853, not on a precision-chasing number type.

---

## 1. Units and non-dimensionalization

### Requirement

All integration happens in **canonical units** chosen so that `μ = 1` for the
current central body. This puts positions, velocities, and accelerations near
order 1, removes a multiply from the innermost loop, and dramatically improves
conditioning.

### Definition

Given a chosen distance unit `DU` and central-body gravitational parameter `μ`:

```
TU  = sqrt(DU³ / μ)
VU  = DU / TU
ACU = DU / TU²
```

(`ACU` is the canonical acceleration unit. Do not name it `AU_*` — "AU"
collides with the astronomical unit, which §4.4 uses as `AU_ref`.)

**Heliocentric set (primary use case):**

| Quantity | Value |
|---|---|
| DU | 1 AU = 1.495978707×10¹¹ m (exact by IAU definition) |
| μ_sun | 1.32712440018×10²⁰ m³/s² |
| TU | ≈ 58.1324 days |
| VU | ≈ 29.7847 km/s |
| Circular orbit at 1 DU | period = 2π, speed = 1 |

**Geocentric set (for planetary phases):**

| Quantity | Value |
|---|---|
| DU | R⊕ = 6378.137 km |
| μ_earth | 3.986004418×10¹⁴ m³/s² |
| TU | ≈ 806.81 s |
| VU | ≈ 7.9054 km/s |

### Implementation notes

- The tables above are illustrative (the textbook value TU ≈ 58.132821 d comes
  from older constants; with the DU and μ stated here, TU ≈ 58.1324 d). At
  runtime, derive the canonical scale factors from the **same μ that ANISE
  supplies to the dynamics** (§4.0): if the two come from different sources
  that disagree in the low digits, the `μ = 1` assumption inside the integrator
  is silently violated. Note also that EGM2008's defining GM differs from the
  WGS84 μ⊕ above in the last digits — see §4.1 for how that is handled.
- Implement a `CanonicalUnits { du_m: f64, tu_s: f64, mu: f64 }` struct with
  `to_canonical` / `from_canonical` for position, velocity, acceleration, and
  time.
- **Use newtype wrappers** (`CanonicalLength(f64)`, `SiLength(f64)`, etc.) or at
  minimum a strict naming convention. Unit-mixing is the single most likely
  source of silent bugs in this codebase.
- ANISE returns SI (km, km/s). Convert **once** at the ANISE boundary, and
  convert back only for output. The integrator must never see SI.
- **Serialization / determinism caveat:** conversion factors like 1 AU in meters
  are not exactly representable in binary, so every round-trip introduces
  roundoff. If bit-exact round-tripping is needed for snapshots or networking,
  use a power-of-two scale factor for *serialization only* (e.g. 2⁻³⁷), keeping
  canonical units for the dynamics.

---

## 2. Cowell's method (default formulation)

### Why it's the default

Cowell integrates the full Cartesian state directly:

```
dr/dt = v
dv/dt = -μ·r/|r|³ + a_pert(r, v, t)
```

That's it. No reference conic, no rectification, no state-space transformation,
no linearization that can degrade. It accepts any force model and any
eccentricity, and it is the correctness oracle for every other formulation.

Paired with DOP853 at rtol/atol 1e-12 and canonical units, Cowell is sufficient
for the overwhelming majority of deep-space propagation. Its weaknesses are
specific and bounded: as `r → 0` the `1/|r|³` term blows up, forcing the adaptive
step size to collapse and degrading the embedded error estimator. That is the
sole trigger for switching to KS.

### Implementation

- State: 6-component `SVector<f64, 6>` — position and velocity in canonical
  units, in the current central body's inertial frame.
- Compute `1/|r|³` once per derivative evaluation and reuse; do not recompute
  the norm.
- Guard against `|r|` underflow with an explicit check that returns an error
  rather than producing `inf`/`NaN` — a silent NaN inside DOP853 will propagate
  through the whole trajectory before anything notices.

---

## 3. KS regularization (switched-in formulation)

### Why full KS, and when

Kustaanheimo–Stiefel regularization maps 3D motion into a 4D space where the
unperturbed Kepler problem becomes a **linear harmonic oscillator**, eliminating
the 1/r² singularity entirely rather than merely coping with it. Near periapsis
this is qualitatively better than Cowell, not just faster: the problem becomes
well-conditioned instead of stiff.

The spec calls for **full KS**, not the simpler Sundman-only time transformation.
Sundman gives you the step-control benefit (`dt/ds = r`) but leaves the singular
`1/r³` term in the equations of motion; KS removes it. Since the entire reason for
the second formulation is close-approach and high-eccentricity robustness, take
the full version.

### Formulation

- **State: 10 components.** 4D position `u`, 4D velocity `u'`, the energy
  variable `h`, and physical time `t`. **Fix the sign convention and document
  it:** in the Stiefel–Scheifele convention `h` is the *negative* of the
  Keplerian orbital energy, `h = μ/r − |v|²/2`, so `h > 0` on elliptic arcs
  and `h < 0` on hyperbolic ones. Getting this sign wrong is the classic KS
  bug — with the opposite convention, `u'' + (h/2)·u = 0` is not an oscillator
  for bound orbits.
- `r = |u|²`. The KS matrix `L(u)` maps `u → r` — see Stiefel & Scheifele,
  *Linear and Regular Celestial Mechanics*, for the canonical construction.
- Independent variable is fictitious time `s`, with `dt/ds = r`. Periapsis step
  refinement falls out automatically.
- Unperturbed equations become `u'' + (h/2)·u = 0`: with `h > 0` (elliptic), a
  harmonic oscillator of frequency `√(h/2)`; with `h < 0` (hyperbolic flyby),
  the same linear equation with exponential solutions. Both are regular — the
  singularity is gone either way. Perturbations enter through the `L(u)`
  transpose mapping.
- **Energy `h` and physical time `t` are dependent variables integrated
  alongside the state.** `t` must be pushed back into `hifitime::Epoch` for every
  ephemeris query — this is the main source of integration friction with ANISE
  and where most bugs will appear.

### Implementation warnings

- The `u → r` map is **not one-to-one**: there is a one-parameter family of `u`
  for any `r` (the fiber of the Hopf map). Pick a consistent convention when
  converting Cartesian → KS. The standard choice branches on whether
  `x ≥ 0`; use it and document it.
- The **bilinear (Levi-Civita) constraint** `ℓ(u, u') = 0` — equivalently, the
  fourth component of `L(u)·u'` must vanish (its explicit form depends on the
  chosen `L` convention; take both from the same source) — is satisfied by the
  standard initialization and preserved analytically, but drifts numerically.
  Monitor it as a health metric next to `h`: nonzero `ℓ` means the 4D state
  has left the physically meaningful fiber of the Hopf map. Optionally
  re-project when it exceeds a bound.
- Energy `h` drifts slowly. Consider periodic re-derivation of `h` from the
  current state as a stabilization step, and monitor the drift as a health metric.
- Cartesian ↔ KS conversion must be verified as an exact round-trip in tests
  before anything else in this module is trusted.

---

## 3a. Formulation switching policy

Cowell and KS are **alternative** formulations, not layers. Implement both as
siblings behind a common `Formulation` trait.

### Switch triggers (Cowell → KS)

KS exists to handle proximity to the **central singularity** — step collapse
near a low, fast periapsis. Anchor the triggers to that, not to the sphere of
influence: R_SOI measures *third-body dominance*, which already has its own
mechanism (the central-body switch, §5).

A rejected alternative, recorded so it isn't revisited: triggering on a
fraction of R_SOI (e.g. "within 0.25 × R_SOI"). Earth's R_SOI is ~924,000 km,
so that region contains every Earth orbiter from LEO to well beyond GEO — the
trigger would silently make KS the de facto formulation for all bound
planetary orbits, including near-circular ones where Cowell has no pathology
at all.

Enter KS when **either** condition holds, in osculating elements about the
**current central body** (constants from ANISE):

1. **Eccentricity.** `e > 0.9`. Every hyperbolic flyby arc has `e > 1` once
   the central body has switched at the SOI, so this alone catches close
   approaches.
2. **Low periapsis on an eccentric arc.** `e > 0.6` **and** periapsis radius
   `r_p < 3 × R_ref` of the central body. This catches GTO-like arcs —
   eccentric with an atmosphere-grazing periapsis, exactly where the
   derivative varies most violently (§4.7) — that the bare `e` threshold
   misses. Near-circular orbits never trigger, at any altitude: with `e ≈ 0`
   the adaptive steps are already uniform and Cowell is the right tool.

All thresholds are configurable starting estimates; tune from profiling.

Return to Cowell when neither condition holds, subject to hysteresis.

### Hysteresis is mandatory

Every switch is a discontinuity that costs a state conversion, an integrator
restart, and the accumulated step-size history. **A trajectory oscillating across
a bare threshold will switch repeatedly and end up less accurate than either
formulation run straight through.**

- Use separate enter/exit thresholds — enter at `e > 0.9`, exit at `e < 0.85`;
  for the compound trigger, enter at `r_p < 3 R_ref`, exit at `r_p > 3.5 R_ref`
  (with `e > 0.6` in / `e < 0.55` out).
- Enforce a minimum dwell time in a formulation before another switch is allowed.
- Log every switch. A high switch count in telemetry is a bug signal, not
  normal operation.

### Switch mechanics

A switch terminates the current integration segment. Convert the state, start a
fresh integration with a conservatively small initial step, and record the
boundary in the trajectory structure (§5). Never attempt to carry step-size state
across a formulation change.

---

## 4. Force model

Total acceleration in canonical units:

```
a = a_central + a_third_body + a_rel + a_srp + a_albedo + a_ir + a_drag
```

(`a_rel` is the Schwarzschild correction of §4.8 — on by default for every
segment, with a config switch for A/B testing only.)

Each term is an implementation of a common `ForceModel` trait, enabled or
disabled per segment. Terms that don't apply in a given regime (drag in deep
space, albedo far from a body) must be *skipped*, not evaluated and multiplied
by zero — ephemeris and atmosphere queries are the hot path.

### 4.0 Body-agnostic design — non-negotiable

**The propagator must support any body in the solar system as the central body.**
Earth-specific models (EGM2008 gravity, NRLMSISE-00 atmosphere) are *plug-in
implementations selected at runtime for one particular body*, never special
cases branched into shared code.

Concretely:

- Central body identity is `anise::NaifId`, carried in the segment context. No
  `if body == EARTH` branches anywhere outside the model-selection registry.
- All body constants — μ, reference radius, flattening, rotation rate/pole
  orientation, Bond albedo — come from **ANISE's planetary constants kernel**,
  never hardcoded. Adding a new body must require zero code changes to the
  integrator, formulations, or force-model plumbing.
- The gravity and atmosphere interfaces (§4.1, §4.5) are traits. Selection is a
  lookup: given a `NaifId`, return the registered implementation, falling back
  to the universal default.
- **Test this explicitly.** The test suite must propagate close approaches at
  Earth, Mars, Jupiter, and at least one small body, exercising the same code
  path. If Earth works and Jupiter doesn't, the abstraction has leaked.

### 4.1 Gravity — central body

Two interchangeable implementations behind a `GravityField` trait:

```rust
trait GravityField {
    /// Acceleration at `r`. If `needs_body_fixed()`, both `r` and the returned
    /// acceleration are in the body-fixed frame; otherwise both are inertial.
    fn acceleration(&self, r: &Vector3<f64>) -> Vector3<f64>;
    /// Harmonics: true. Point-mass: false — lets the caller skip two frame
    /// rotations per derivative evaluation on the hot path.
    fn needs_body_fixed(&self) -> bool;
    fn reference_radius(&self) -> f64;
    fn mu(&self) -> f64;
}
```

**Earth: EGM2008 spherical harmonics.**

- Use a truncated EGM2008 coefficient set. Degree/order is **configurable**;
  allow up to 360×360. Cost scales as O(n²) per evaluation and DOP853 makes
  12+ derivative calls per step, so this is the dominant cost near Earth —
  make truncation a tuning knob, not a constant. Regime-based starting
  defaults for the §0 target: **70×70 for LEO**, **~20×20 for MEO** (enough to
  carry the resonant tesserals that drive multi-year MEO evolution), **8×8 for
  GEO**; verify each against a higher-degree reference run per §7 before
  trusting it.
- Evaluate with **fully normalized** associated Legendre functions and a stable
  recursion. The unnormalized formulation exhausts f64's dynamic range around
  degree ~85 (naively computed factorial ratios overflow even earlier in
  intermediate terms) and loses accuracy well before that — treat
  normalization as mandatory, not an optimization. Cunningham/Pines or the Holmes–Featherstone recursion are the standard safe
  choices; **Pines' formulation additionally avoids the singularity at the
  poles** and is preferred.
- Requires an **ITRF ↔ inertial rotation** every evaluation: transform position
  to body-fixed, evaluate, rotate the acceleration back. Get this transform
  from ANISE, not from a hand-rolled rotation — **and load the high-precision
  binary Earth PCK** (`earth_latest_high_prec.bpc`, exposed as the
  `EARTH_ITRF93` frame; ANISE's `BPC::load` reads it directly). The IAU_EARTH
  model from the text PCK is deliberately low-fidelity — no nutation, polar
  motion, or UT1 — and mislocates body-fixed positions by hundreds of meters,
  defeating the purpose of a high-degree field.
- **Degree-2 solid Earth tides are in scope** — a consequence of the §0 target
  combined with long Earth arcs. Apply the IERS frequency-independent
  correction: Sun and Moon positions (already on hand from §4.2) perturb the
  `C₂₀, C₂₁, S₂₁, C₂₂, S₂₂` coefficients through the degree-2 Love numbers
  each evaluation. It is a handful of lines on top of the harmonic model and
  changes nothing architecturally (the `GravityField` implementation gains a
  time-dependent coefficient hook). Ocean tides and the frequency-dependent
  solid-tide corrections stay out (§4.9): they matter below this spec's
  accuracy floor.
- Evaluate the field with the **coefficient set's own defining constants** —
  for EGM2008, `GM = 3.986004415×10¹⁴ m³/s²` and `a = 6 378 136.3 m` — not the
  WGS84 values used for §1's canonical table. The canonical μ and the model GM
  may legitimately differ in the low digits; convert explicitly at the model
  boundary instead of assuming they are equal.
- Ship the coefficient file as a data asset with a documented source and a
  checksum. Do not embed thousands of coefficients in source.

**All other bodies: point-mass (`-μ·r/|r|³`).**

This is deliberate and sufficient for the current scope. The trait design means
adding a Mars (GMM-3) or Moon (GRAIL/GRGM) harmonic model later is a new
implementation plus a registry entry — no changes elsewhere. Document that
choice in code so it reads as a decision rather than an oversight, and record
one consequence explicitly: Jupiter's `J₂ ≈ 1.47×10⁻²` and Saturn's
`≈ 1.63×10⁻²` are an order of magnitude larger than Earth's, so close
giant-planet flybys carry real, documented model error until at least a
`J₂`-only field is registered for them.

**Selection:** a registry mapping `NaifId → Box<dyn GravityField>`, with the
point-mass model as the universal default and EGM2008 registered for Earth.
Harmonics are only worth evaluating close to the body; beyond a few reference
radii, fall back to point-mass even for Earth — make that cutoff configurable.

### 4.2 Gravity — third bodies

- Third-body perturbations from Sun, planets, Moon — query positions via
  **ANISE** (`anise::almanac::Almanac`, loaded with DE440 or DE441 SPK).
- Third bodies are **always point-mass**, regardless of which body it is. No
  harmonics for a perturbing body.
- Use the **Battin / difference-of-cubes** form (the `F(q)` function of
  Appendix A) to avoid catastrophic cancellation **when the perturbation is
  small** — that is, when the spacecraft is much closer to the central body
  than to the perturber (`|r| ≪ |r₃|`: the Sun or Moon perturbing a planetary
  orbiter), where the direct and indirect terms nearly cancel. Near the
  perturbing body the direct term dominates and there is no cancellation —
  that regime is handled by the central-body switch (§5) instead:
  `a = -μ_3 · [ (r - r_3)/|r - r_3|³ + r_3/|r_3|³ ]` — evaluate the bracket with
  the cancellation-safe formulation, not naively.
- Which third bodies to include should be configurable per segment; near Earth
  the Sun and Moon dominate, in deep space the giant planets matter.

### 4.3 Spacecraft model — cannonball sphere

**Both SRP and drag use a single spherical ("cannonball") spacecraft model.**
This is a deliberate simplification and it buys a lot: a sphere presents the same
cross-section from every direction, so the effective area is `A = πR²`
regardless of attitude, and **no attitude state is needed anywhere in the
propagator.**

One shared struct:

```rust
struct SpacecraftModel {
    mass_kg: f64,
    radius_m: f64,      // -> area = PI * radius^2
    c_r: f64,           // SRP reflectivity coefficient, [1, 2]
    c_d: f64,           // drag coefficient, ~2.2 for spheres in free-molecular flow
}
```

Both forces then reduce to a scalar ballistic coefficient times a direction:

```
SRP:  (C_r · A/m) · [pressure] · r̂_sun
Drag: (C_d · A/m) · [dynamic pressure] · (-v̂_rel)
```

Direction conventions, stated once and enforced in code: `r̂_sun` is the unit
vector **from the Sun to the spacecraft** (SRP pushes away from the Sun);
`v̂_rel` is the unit vector of the atmosphere-relative velocity (drag opposes
it). Sign ambiguity here produces plausible-looking but wrong trajectories.

- `C_r` and `C_d` should be **estimable parameters**, not constants. Real
  missions fit them from tracking data, and they absorb the modeling error this
  simplification introduces.
- Consequences to document: no attitude dependence, no per-facet reflectivity, no
  specular/diffuse split, no self-shadowing. For a spacecraft with large solar
  panels the true area can vary several-fold with attitude, so the cannonball
  model is an approximation that a fitted `C_r`/`C_d` partly compensates for.
- **Design the interface so a future attitude-dependent area model can replace
  the scalar area** — take `fn area(&self, direction: &Vector3<f64>) -> f64` even
  though the sphere implementation ignores the argument. This costs nothing now
  and avoids a refactor later.

### 4.4 SRP — direct solar

```
a_srp = ν · P_sun · (AU_ref / d)² · C_r · (A/m) · r̂_sun
```

- `P_sun` ≈ 4.5398×10⁻⁶ N/m² at 1 AU (1361 W/m² total solar irradiance / c).
  The legacy 4.56×10⁻⁶ pairs with the older 1367 W/m² solar constant — use
  one consistent pair, never a mix.
- `AU_ref` is 1 astronomical unit, the reference distance for `P_sun` —
  unrelated to the canonical acceleration unit `ACU` of §1.
- `r̂_sun`: unit vector from the Sun to the spacecraft (sign convention in
  §4.3).
- `C_r` ∈ [1, 2]: 1 = perfect absorber, 2 = perfect specular reflector.
  Typical spacecraft ≈ 1.2–1.5.
- `A/m` from the cannonball model of §4.3 — `πR²/m`, attitude-independent.
  Typical ≈ 0.02 m²/kg.
- `ν` is the shadow illumination fraction ∈ [0,1] — see §4.5.
- Magnitude ≈ 1.2×10⁻⁷ m/s² at 1 AU, falling as 1/d².

**Relative importance:** SRP significance scales as `r²/(μ_p·d²)`. Evaluated at a
fixed fraction of the Hill radius, the `d²` cancels and only `μ_p^(-1/3)`
remains — so **planet mass, not solar distance, governs whether SRP matters.**
Normalized (Earth = 1): Mercury 2.6, Mars 2.1, Venus 1.1, Uranus 0.41,
Neptune 0.39, Saturn 0.22, Jupiter 0.15. At small bodies SRP can reach ~20% of
gravity (Rosetta at 67P) — it is a first-class force there, not a perturbation.

### 4.5 Shadow model — conical with penumbra

**This is the highest-risk part of the force model. Do not use a cylindrical
shadow.**

- Compute apparent angular radii of the Sun and the occulting body as seen from
  the spacecraft, plus their angular separation.
- Three regimes: full sun (`ν = 1`), penumbra (`0 < ν < 1`, from the
  circle-circle lens intersection area of the two apparent disks), umbra
  (`ν = 0`). Antumbra also possible far from the body.
- The Sun's apparent radius at 1 AU is ~0.267°, so the penumbra is a **smooth
  ramp**, not a step. A cylindrical model creates a hard C⁰ discontinuity that
  wrecks DOP853's error estimator.
- **Oblateness:** an ellipsoidal Earth casts a non-circular shadow. Assuming a
  sphere displaces the eclipse boundary by tens of km. Include oblateness for
  precise work.
- **Optional refinement:** atmospheric refraction bends sunlight into the
  geometric umbra and ozone absorbs part of it, softening the true boundary.
  Treat as a later enhancement.
- **Occulting bodies are per-segment configuration**, per §4.0: default to the
  current central body, but the Moon must be a candidate occulter for Earth
  orbiters, and the large moons matter at the giant planets. With several
  candidates, combine as `ν = Π νᵢ` (exact when occulters don't overlap on the
  solar disk — acceptable here).
- Duty cycle context: LEO spends ~35–40% of each orbit in shadow; GEO only
  eclipses in two ~45-day seasons near the equinoxes, max ~72 min.

### 4.6 Planetary albedo and thermal IR

Both act **radially outward from the planet**, unlike direct SRP which acts
anti-sunward. Together typically 10–30% of direct SRP in low orbit; negligible
beyond a few planetary radii (both fall as 1/r²).

- **Albedo** — reflected sunlight. Scale by planetary Bond albedo, the
  spacecraft's view factor of the sunlit portion of the disk, and the phase
  angle. Zero when the spacecraft sees only the night side. Venus is the
  extreme case (albedo ≈ 0.77).
- **Thermal IR** — the planet's own emission. Roughly isotropic over the disk,
  present on the night side too (so it does *not* vanish in eclipse). Mercury's
  dayside IR is strong enough that MESSENGER and BepiColombo modeled it
  explicitly.
- Minimum viable model: single-element disk with view factor. Better: coarse
  spherical-cap ring discretization of the visible planetary disk.

### 4.7 Atmospheric drag — NRLMSISE-00 (Earth)

```
a_drag = -0.5 · ρ · |v_rel| · v_rel · C_d · (A/m)
```

**Density model: NRLMSISE-00**, an empirical model of the neutral atmosphere
valid from the ground to ~1000 km. Use the `nrlmsise00` crate if it meets needs;
otherwise bind the reference C implementation. Verify against the model's own
published test cases before trusting it.

**Inputs the model requires** — these are the awkward part, plan for them:

- Position: altitude, geodetic latitude, longitude (geodetic, not geocentric —
  conversion via the ANISE-supplied flattening).
- Time: day of year and UT seconds, derived from `hifitime::Epoch`.
- **Space weather:** F10.7 solar flux (daily and 81-day average) and Ap
  geomagnetic index. These are *measured historical data*, not computable. Load
  from a space-weather file (CelesTrak SW-All format is standard). Provide a
  clearly-labeled constant-value fallback for testing, and **fail loudly rather
  than silently substituting defaults** in production runs — bad space weather
  input is a common source of quietly-wrong drag. Honor the model's input
  conventions: daily F10.7 is the **previous day's** value, the 81-day average
  is centered on the epoch, and `Ap` may be given either as a daily value or
  as the 7-element 3-hourly `ap` history array (use the array when storm-time
  fidelity matters). Local solar time is also an input, derived from UT and
  longitude.

**Relative velocity must account for atmospheric rotation.** The atmosphere
co-rotates with the body:

```
v_rel = v_inertial - ω_body × r
```

Omitting this is a large error (`ω R⊕` ≈ 465 m/s at Earth's equatorial
surface, nearer 490 m/s at LEO radius) and a frequent bug.
Take `ω_body` from ANISE.

**Other characteristics:**

- `C_d ≈ 2.2` is the standard value for a sphere in free-molecular flow. Keep it
  estimable — it absorbs both density-model error and shape error.
- Density falls off roughly exponentially with a scale height of ~50–60 km in the
  thermosphere, so drag varies by **orders of magnitude within a single orbit**
  for an eccentric one. This makes the derivative function strongly varying near
  periapsis — one more reason the KS switch (§3a) matters.
- **Enable drag only below a configurable altitude cutoff** — default ~1000 km,
  the model's validity ceiling. Above it, skip the evaluation entirely. This is a
  significant performance concern: NRLMSISE-00 is expensive and DOP853 calls the
  derivative function 12+ times per step.
- Drag is **non-conservative and dissipative**. It removes energy monotonically,
  so energy-conservation validation tests must be disabled or reinterpreted on
  any segment where drag is active.

**Other bodies: no atmosphere model.** Density is zero, drag term skipped. The
`AtmosphereModel` trait must be structured so Mars (Mars-GRAM/DTM-Mars), Venus,
or Titan can be registered later without touching the drag force implementation —
same registry pattern as §4.1.

### 4.8 Relativistic correction — Schwarzschild term

The first-order (1PN) Schwarzschild acceleration of the current central body,
in the IERS Conventions form with β = γ = 1 (`r`, `v` relative to that body):

```
a_rel = (μ / (c²·r³)) · [ (4·μ/r − v²)·r + 4·(r·v)·v ]
```

- **Cheap**: no ephemeris query, a handful of flops. Register it as an
  ordinary `ForceModel`; **default on for every segment** (decided). At Earth
  it contributes a secular perigee drift that is visible at the §0 target over
  multi-year arcs; the cost is negligible, so there is no regime where turning
  it off buys anything. Keep a config switch for A/B validation runs only.
  Convert `c` into the segment's canonical units once, at segment setup.
- **Not optional for validation.** DE440/DE441 dynamics are relativistic, so
  the long-arc ephemeris cross-check (§7) diverges without this term:
  Mercury's perihelion advances ~43″/century from it alone (≈5.0×10⁻⁷ rad per
  orbit — tens of km/year of along-track drift), and even Earth accumulates
  km-scale error per year.
- Deliberately first-order only: Lense–Thirring, de Sitter, and other bodies'
  Schwarzschild terms stay out of scope (§4.9).

### 4.9 Explicitly out of scope for v1

Spacecraft magnetic torque coupling, Lorentz force from spacecraft charging,
thermal re-radiation / "thermal snap," attitude propagation of any kind, and
non-spherical spacecraft geometry (per-facet reflectivity, specular/diffuse
split, self-shadowing). Harmonic gravity for bodies other than Earth, and
atmosphere models for bodies other than Earth — both are registry entries away,
per §4.0. Also out: relativistic terms beyond the central-body Schwarzschild
correction of §4.8 (Lense–Thirring, de Sitter, third-body relativity), ocean
tides and the frequency-dependent solid-tide corrections (the
frequency-independent degree-2 solid tide is in scope — §4.1), and time/cloud
variability of planetary albedo (a constant Bond albedo per body is
assumed).

**Note:** Earth's magnetic field has **no direct effect on SRP** — photons are
uncharged. It couples only indirectly, via magnetic torque changing attitude and
hence sunlit area, which the cannonball model (§4.3) makes moot by construction.

---

## 5. Integrator: DOP853

### Crate usage

Use `differential-equations` with the DOP853 solver. **Verify the current API
against docs.rs before writing code** — the crate is evolving and the trait and
builder names have changed across versions. As of writing the pattern is roughly:

```rust
use differential_equations::prelude::*;
// impl ODE for the system:
//   fn diff(&self, t: f64, y: &SVector<f64, N>, dydt: &mut SVector<f64, N>)
let method = ExplicitRungeKutta::dop853().rtol(1e-12).atol(1e-12);
let solution = IVP::ode(&system, t0, tf, y0).method(method).solve()?;
```

### Configuration

- Statically-sized `nalgebra::SVector` state (this crate's advantage over `ivp`).
- Start with `rtol = atol = 1e-12`. Tighter than ~1e-14 is wasted against f64.
- Independent variable is **canonical time offset from a segment-start epoch**,
  never absolute seconds. Keep the `hifitime::Epoch` as the segment anchor and
  add the canonical offset as a `Duration` when querying ANISE.
- Enable **dense output** — required for event detection and for producing
  output at arbitrary requested epochs.

### Event detection — critical

The crate's event/root-finding support must be used for:

1. **Shadow entry/exit.** Define a signed shadow function; root-find the exact
   crossing via dense output; end the step there and restart. Stepping blindly
   across an eclipse boundary invalidates the embedded error estimate.
2. **Formulation switch triggers** — the eccentricity and periapsis-radius
   conditions of §3a, with their hysteresis bands applied to the event
   functions.
3. **SOI transitions** triggering a central-body change. A central-body change
   re-expresses the same physical state — it is not a change of physics. The
   total physical acceleration must be continuous across the boundary; only
   the central/third-body roles, frame, unit set, and anchor epoch swap.
   Validate that continuity explicitly (§7).

**General rule: never integrate through a discontinuity. Detect, stop, restart.**

### Structure

Model the trajectory as a sequence of **segments**, each with its own central
body, formulation, unit set, and anchor epoch. A segment boundary is any event
above. Provide a `Trajectory` type that stitches segments and interpolates
across them for arbitrary-epoch queries.

---

## 6. Crate integration notes

### hifitime
- `Epoch` and `Duration` are integer-backed and exact. Use them for all epoch
  arithmetic and time-scale conversion (TAI/TT/TDB/UTC).
- Convert to f64 **only** as an offset relative to a segment anchor, never as an
  absolute time since J2000.
- TDB is the correct time scale for planetary ephemeris queries.

### anise
- Modern Rust replacement for SPICE; same author as hifitime.
- Load DE440/DE441 SPK plus a planetary-constants kernel; query via `Almanac`.
- Ephemeris queries are the **hot path** — DOP853 makes 12+ derivative
  evaluations per step. Profile early; consider caching or Chebyshev-interpolating
  third-body positions across a step if it dominates.
- ANISE also supplies frame transformations and body constants (μ, radii,
  flattening) — take them from ANISE rather than hardcoding.
- For Earth body-fixed work, load the high-precision binary Earth PCK
  (`earth_latest_high_prec.bpc` → `EARTH_ITRF93` frame). The text-PCK
  IAU_EARTH rotation is low-fidelity by design — see §4.1.

### differential-equations
- Verify current API against docs.rs first.
- Supports ODE/DAE/DDE, fixed and adaptive solvers, event detection, dense
  output, statically-sized state.

---

## 7. Validation plan

Build these in order; each one catches a distinct class of bug. Every test
needs a numeric pass/fail threshold, derived from the §0 target (10–100 m over
multi-year arcs): pure-numerics tests (1, 2, 4, 7, 12) must sit orders of
magnitude below it — near machine precision where a closed form exists — while
model-fidelity tests (10, 11, 13–15) get budgets allocated from it. Record the
budget allocation in the test code itself.

1. **Two-body closed form.** Propagate a circular and an elliptical orbit one
   period; compare against a universal-variable Kepler solver. Should hit
   near-machine precision.
2. **Energy and angular momentum conservation.** Gravity-only, long arc. DOP853
   is not symplectic so slow drift is expected — quantify it, and check it is
   not growing faster than the error tolerance implies.
3. **KS ↔ Cartesian round-trip.** Convert state out and back; must return to
   machine precision. Do this before trusting anything else in the KS module.
4. **KS vs Cowell agreement.** Same moderately-eccentric problem both ways, no
   switching. Results must agree to tight tolerance, and KS should need
   dramatically fewer steps as eccentricity rises. Then run a high-eccentricity
   case (e = 0.95) where Cowell struggles and confirm KS stays well-conditioned.
5. **Switching consistency.** Run a close flyby three ways: Cowell throughout, KS
   throughout, and with automatic switching. All three should agree. If the
   switched run is the outlier, the conversion or the restart logic is wrong.
6. **Switch-chatter test.** Construct a trajectory that grazes the switch
   threshold and confirm hysteresis prevents repeated toggling.
7. **Round-trip determinism.** Forward-then-backward integration should return
   the initial state to tolerance. (Confirm first that the solver crate
   supports integrating with `tf < t0`; if not, negate the independent
   variable in a wrapper.)
8. **Shadow geometry unit tests.** Verify `ν` is continuous across penumbra
   boundaries, hits exactly 1 and 0 outside/inside, and that event detection
   lands on the crossing epoch and not a step boundary.
9. **Acceleration continuity across a central-body switch.** Evaluate the
   total physical acceleration immediately before and after an SOI-triggered
   central-body change; after unit conversion the two must agree to near
   machine precision. Any jump means the central/third-body bookkeeping
   (§4.2, §5) is wrong.
10. **Ephemeris cross-check.** Propagate a real planet with ANISE-supplied
    initial conditions and compare against ANISE's own ephemeris over a long
    arc, **with the Schwarzschild term (§4.8) enabled** — the reference
    ephemerides are relativistic, and inner planets diverge visibly without
    it.
11. **Relativity sanity check.** Two-body + Schwarzschild about the Sun with
    Mercury's elements must recover the ~43″/century perihelion advance
    (≈5.0×10⁻⁷ rad per orbit).
12. **Unit round-trip property tests.** SI → canonical → SI for all quantities.
13. **EGM2008 degree-0 sanity.** Truncated to degree 0, the harmonic model must
    reproduce point-mass to machine precision. Then confirm the degree-2 term
    reproduces a known J2 secular nodal regression rate.
14. **EGM2008 reference-point check.** Compare computed gravity against published
    EGM2008 values at known geodetic locations. Also confirm no pole singularity —
    evaluate directly over both poles.
15. **NRLMSISE-00 vendor test cases.** The model ships with reference
    input/output pairs; match them before wiring drag in. Separately, verify a
    LEO orbit decays at a plausible rate, and confirm co-rotation is included by
    comparing drag magnitude with and without the `ω × r` term — the difference
    should be large.
16. **Multi-body genericity test — mandatory.** Run the same close-approach
    scenario at Earth, Mars, Jupiter, and a small body, changing only the
    `NaifId` and initial conditions. Any body requiring a code change is a spec
    violation. Confirm Earth picks up EGM2008 and drag while the others correctly
    fall back to point-mass with no atmosphere.

---

## 8. Suggested build order

1. Units module + newtype wrappers + round-trip tests.
2. hifitime/ANISE wrapper: epoch handling, body states, constants, body-fixed
   frame transforms.
3. **Model registry and traits** (`GravityField`, `AtmosphereModel`,
   `ForceModel`) with point-mass and null-atmosphere as the universal defaults.
   Build this before any body-specific model so §4.0 is structural rather than
   retrofitted.
4. Cowell two-body on DOP853. Validate against closed-form Kepler.
5. Third-body gravity (Battin form). Validate against ANISE ephemeris.
6. Cannonball `SpacecraftModel`, then direct SRP with conical shadow + event
   detection.
7. Albedo and planetary IR. Add the Schwarzschild term (§4.8) here as well —
   it is a few lines and unblocks the ephemeris cross-check (§7).
8. EGM2008: coefficient loading, normalized Legendre recursion (Pines),
   body-fixed transform, truncation config. Validate degree-0 against point-mass
   first.
9. NRLMSISE-00 drag: space-weather ingestion, geodetic conversion, co-rotation,
   altitude cutoff.
10. **KS conversion functions**, with round-trip tests, before any KS integration
    is attempted.
11. KS equations of motion; validate against Cowell with switching disabled.
12. Segment/trajectory stitching, formulation-switch events, hysteresis.

Steps 1–7 produce a complete and useful deep-space propagator. Steps 8–9 add the
near-Earth regime; steps 10–12 add close-approach robustness. Each block is
independently deferrable.

---

## 9. Open decisions for the implementer

- Segment state persistence format (and whether power-of-two serialization
  scaling from §1 is needed).
- Whether third-body ephemeris caching is required — decide from a profile, not
  in advance.
- Discretization fineness for the albedo/IR disk model.
- Exact hysteresis band widths in §3a — the given values are starting estimates
  and should be tuned against real mission profiles.
- Final EGM2008 truncation degrees per regime (starting defaults in §4.1) and
  the altitude above which Earth falls back to point-mass. Both are pure
  accuracy-vs-cost tradeoffs against the §0 target; settle them from a profile
  against a reference trajectory, not in advance.
- Space-weather file source, update cadence, and behavior when a requested epoch
  falls outside the available data (predicted values? hard error?).
- Whether the `nrlmsise00` Rust crate is adequate or a C binding is needed
  (the `satkit` crate's NRLMSISE-00 implementation, which bundles automatic
  space-weather updates, is a third option to evaluate).

Decided since Rev B, no longer open: the accuracy target (§0 — 10–100 m over
multi-year arcs, propagator-attributed), long Earth-orbit arcs in scope (§0),
and the Schwarzschild term default-on for all segments (§4.8).

---

## Appendix A — Encke's method (deliberately excluded)

Recorded so the decision isn't silently revisited. Encke integrates the deviation
`δr` from an analytically-propagated reference conic rather than the full state,
concentrating f64 precision on the perturbation and permitting larger steps.

**Why it's out:** it is a step-count optimization — roughly 2–5× on weakly
perturbed cruise — not an accuracy improvement. DOP853's adaptivity already
recovers much of the benefit, and canonical-unit f64 has ample precision headroom
without it. It also adds two failure modes Cowell doesn't have: mandatory
rectification when `|δr|/|r_osc|` grows too large, and a notorious silent bug
where the near-cancelling difference of inverse cubes is evaluated by naive
subtraction instead of via Battin's stable q-function
`F(q) = q·(3 + 3q + q²) / (1 + (1+q)^(3/2))`. (Caution: forms such as
`1 − (1+q)^(−3/2)` differ from `F(q)` by a factor `(1+q)^(3/2)`; texts fold
that factor into the equations differently, so take the complete formulation
from a single source.) The naive evaluation quietly destroys the method's
entire advantage while still producing plausible-looking output.

**Revisit only if** profiling shows the propagator is step-count-bound during long
weakly-perturbed heliocentric cruise. It would slot in as a third sibling under
the `Formulation` trait, active only in the cruise regime, with Cowell retained
as its validation reference.
