# PHASE3 — Project state and technical context

Snapshot at the end of phase 3 (2026-06-15). This is the **canonical
current-state document**: everything an agent or developer needs to make
changes without re-deriving the design. It supersedes
`claude/phase2/PHASE2.md` as the current-state reference. PHASE2 remains
accurate for the host/build architecture (winit + raw wgpu shell, the
BC7/KTX2 + LUT build pipeline, the smoothed-zoom controller) and is
referenced rather than duplicated below; `claude/phase1/PHASE1.md`
remains the deep reference for the rendering/atmosphere math.

**What phase 3 was.** A **shader-only** change to `shaders/globe.wgsl`:
the photographic night map was dropped as the dark-side *color* and
replaced with a day-textured globe that is darkened by sun geometry,
plus **procedurally re-lit, glowing-yellow city lights** that
**dither-dissolve** toward the day side. The night texture is still
sampled — but now only as a per-pixel **luminance mask** to find cities,
never as displayed color. No Rust, `build.rs`, bind-group/layout,
uniform, pipeline, or texture changes. The host shell, build pipeline,
atmosphere, stars, camera, input, and UI are all byte-for-byte as PHASE2
left them (including the 2026-06-13 changes folded in below).

> **Update (2026-06-17).** Three changes since the 06-15 snapshot, each
> detailed in its section below:
> 1. **Emissive lights now bleed past the terminator.** A new
>    `EMISSIVE_FADE_END` constant moves the dissolve's completion point off
>    the terminator (`cos_sun = 0`) and onto the daylit side, so a rim of
>    lit-side pixels keeps its lights. This **supersedes** every "fully gone
>    at the terminator / none bleed onto the day side" statement still
>    phrased that way below — see §2.1, §2.2, §2.4, §4. Mechanism otherwise
>    unchanged; `EMISSIVE_FADE_END = 0` reproduces the old behavior exactly.
> 2. **Repo-wide ASCII-only source.** Every `.rs` file and
>    `shaders/globe.wgsl` were converted to pure ASCII (`—`→`-`, `°`→`deg`,
>    `±`→`+/-`, `≈`→`~`). Comments/strings only; no behavior change (the one
>    visible effect: the egui sun-slider labels now read `deg`, not `°`). The
>    quoted shader comments below were lightly updated to match. The
>    `claude/` docs — including this file — intentionally keep their Unicode.
> 3. **`build.rs` transcode caching changed.** The per-file
>    skip-if-`dest`-exists guard was removed; `transcode()` now runs
>    unconditionally on every build-script execution, with cargo's
>    `rerun-if-changed` (per asset, plus `build.rs` itself) as the sole
>    rerun gate. See §3 for the consequence (any rerun re-encodes all five
>    BC7 textures).

---

## 1. Unchanged since PHASE2 (read PHASE2 for detail)

Phase 3 touched **only** `shaders/globe.wgsl` `fs_main` and its noise
helpers/constants. Everything else is exactly as documented in PHASE2.
The load-bearing facts, in brief — see PHASE2 for the full treatment:

- **Stack.** Rust edition 2024, `winit 0.30` window/event loop, `wgpu 29`
  (direct dep) surface + render loop, `egui 0.34` sun-slider overlay,
  `glam`, `bytemuck`, `ktx2`, `pollster`, **`rayon`**. Package still
  named `iced-test-app` (historical); iced is gone.
- **Host shell** (`src/main.rs`): winit `ApplicationHandler`,
  `ControlFlow::Wait` (idle = zero GPU work), `Gfx` created in
  `resumed()`. Device requests `Features::TEXTURE_COMPRESSION_BC`.
  **Two deliberate surface overrides**: a **non-sRGB** surface format
  (all shader look-tuning constants are calibrated to this darker
  rendition) and `PresentMode::AutoVsync` (paces the animation loop).
  Hidden-until-ready window revealed after the first `present()`; the
  first frame must go through a **direct `redraw()`**, not
  `request_redraw()` (Windows paint-event delivery), plus an `Occluded`
  first-frame guard.
- **Build pipeline** (`build.rs`): downloads 5 solarsystemscope textures
  (8192×4096), BC7→KTX2 transcodes them (sRGB block for day/night/stars,
  UNORM for normal/specular), and bakes 3 atmosphere LUTs to
  `Rgba16Float` KTX2 — all into `OUT_DIR`, `include_bytes!`-ed at
  runtime. **Runtime does no image decode and no LUT bake.**
  *(2026-06-13: the LUT bake source, formerly `src/globe/atmosphere.rs`
  pulled in via `#[path]`, is now **inlined into `build.rs`** as
  `mod atmosphere`; that file is deleted. Texels are bit-identical. The
  atmosphere medium constants are still **duplicated** between that
  inline module and `globe.wgsl` and must stay in sync.)*
- **Renderer** (`src/globe/renderer.rs`): `GlobeRenderer` with
  `new`/`prepare`/`render`. One WGSL module, one shared bind group
  (group 0), one shared `uv_sphere(64,128)` buffer, **three pipelines
  drawn in order — stars, surface, atmosphere** — into a single render
  pass, no depth buffer (draw order does occlusion). `upload_ktx2` is
  the single memcpy-to-GPU upload path; all textures `mip_level_count 1`.
  *(2026-06-13: `GlobeRenderer::new` is **parallelized with rayon** —
  `rayon::join` overlaps shader-module compile with the 8 KTX2 uploads
  (`into_par_iter`) and a nested join compiles the 3 pipelines
  concurrently. Intentional; do not treat as the phase-1 revert.)*
- **Input** (`src/globe/input.rs`): `Controller` — left-drag pan,
  right-drag tilt, flick inertia, and the **exhaustively-iterated
  rate-adaptive smoothed zoom** (velocity bridging, adaptive half-life).
  Tune the named constants; **do not restructure** the glide/coast.
- **UI** (`src/ui.rs`): egui `Area` with sun latitude / longitude
  sliders mutating `&mut Sun` directly.
- **Conventions**: unit sphere at origin, +Y north, lon0/lat0 faces +Z;
  **vertex position = surface normal = world position**; equirectangular
  UVs (u wraps, v clamps); analytic tangent frame
  `east = normalize((n.z,0,−n.x))`, `north = n×east`.
- **Bind group 0 layout is unchanged** (see PHASE2 §"Bind group 0
  layout"). The **night texture is still at binding 3** and still
  sampled every fragment — phase 3 only changed *how its sample is used*.

The rest of this document covers **only** what phase 3 changed.

---

## 2. The phase-3 shader change (`shaders/globe.wgsl`)

### 2.1 Goal and the look it produces

Replace the photographic night-side color with a day-lit, darkened
globe plus procedural city lights that thin out across the terminator
and clear a tunable distance onto the daylit side (`EMISSIVE_FADE_END`,
item 3):

1. **Day map everywhere.** The whole sphere is shaded from
   `day_texture`; the unlit hemisphere is darkened by sun geometry. The
   night map is **never displayed as color**.
2. **Uniform-threshold city mask.** A pixel is a "city" if its night-map
   *luminance* clears a single fixed threshold. (A terminator-varying
   threshold was tried and **rejected** — the night map's city cores are
   clipped plateaus, so raising the threshold makes whole blobs pop out
   instead of shrinking.)
3. **Dither-dissolve across the terminator.** A fixed-grain 3D noise
   dither stochastically erodes the city pixels as they approach — and now
   cross — the lit side, so each city thins into sparkles. The dissolve
   completes at `cos_sun = EMISSIVE_FADE_END`, a tunable constant that, when
   positive (currently `0.15`), sits **on the daylit side of the
   terminator**: a band of just-past-sunrise / pre-sunset land keeps its
   lights instead of them vanishing exactly at `cos_sun = 0`. Setting
   `EMISSIVE_FADE_END = 0` recovers the original terminator-exact cutoff.
   Survivors stay at full, **uniform** brightness — binary keep/discard, no
   dimming.
4. **Bright-yellow additive glow** (LDR; no bloom).

### 2.2 New constants (look-tuning block, `globe.wgsl:67-85`)

Added after `WAVE_STRENGTH`. **These are owner-tuned feel knobs and
drift between sessions — read the file for live values, never trust this
doc for numbers.** Current values as of this snapshot:

```wgsl
const EMISSIVE_THRESHOLD: f32 = 0.05;   // night-map luminance to count as "city"
const EMISSIVE_SOFTNESS: f32  = 0.1;    // smoothstep width above the threshold
const EMISSIVE_COLOR: vec3<f32> = vec3<f32>(1.0, 0.85, 0.3);  // bright yellow
const EMISSIVE_STRENGTH: f32  = 1.5;    // >1 drives the core to clip (LDR)
const EMISSIVE_FADE_START: f32 = -0.15; // sun cosine where dissolve begins
const EMISSIVE_FADE_END: f32   = 0.15;  // sun cosine where dissolve completes; >0 bleeds onto the day side
const DITHER_SCALE: f32       = 400.0;  // noise cells across the unit sphere
const NIGHT_DARKNESS: f32     = 1.2;    // dark-side multiplier on the day map
```

**The `EMISSIVE_FADE_START` / `EMISSIVE_FADE_END` pair** is the sun-cosine
interval the dither dissolve runs over (§2.4). `FADE_START < 0` holds full
cities deep on the night side; `FADE_END` (**new in the 06-17 update**) sets
where the dissolve finishes and is the bleed knob. Because the terminator
(`cos_sun = 0`) is the midpoint of the current `[-0.15, 0.15]` interval, the
mask is ~50 % eroded *at* the terminator and fully cleared by
`cos_sun = 0.15` — sun ≈ 8.6° above the horizon (`acos(0.15) ≈ 81.4°` from
zenith) — so a thin lit-side rim stays lit. `FADE_END = 0` reproduces the
pre-update "gone exactly at the terminator" look; larger values push the
lights deeper into daylight. The pair must satisfy `FADE_START < FADE_END`
(it feeds `smoothstep`, whose edges must be ordered).

> **Divergence from PHASE3_PLAN — current tuning state.** The plan's
> starting values were `EMISSIVE_THRESHOLD = 0.25` and
> `NIGHT_DARKNESS = 0.02` (near-black night, the stated design intent).
> The live shader has `EMISSIVE_THRESHOLD = 0.05` (a much more permissive
> city mask — more of the night map clears it) and, notably,
> **`NIGHT_DARKNESS = 1.2`**. Because the dark-side factor is
> `mix(NIGHT_DARKNESS, 1.0, daylight)`, a value of **1.2 makes the unlit
> hemisphere ~20 % _brighter_ than full daylight**, not near-black — so
> the "darken the night side to near-black" goal is **not currently in
> effect**; the globe reads bright all the way around with the city
> glow layered on top. This looks like mid-tuning rather than a settled
> look; treat both numbers as in-flight and confirm intent with the
> owner before "fixing" them toward the plan. The mechanism is correct
> regardless of the constant — only the value differs.

`DAY_AMBIENT` (`globe.wgsl:44`, 0.04) interacts with `NIGHT_DARKNESS`:
ambient sets the floor of `day_lit`, then `night_factor` scales it on
the dark side. To darken the night side, lower `DAY_AMBIENT` first, then
`NIGHT_DARKNESS`.

### 2.3 New noise helpers (`globe.wgsl:111-149`)

Added next to the existing 2D ocean-glint noise (`hash2`/`value_noise`/
`wave_noise`, lines 87-109). A **3D value noise** sampled at the sphere
surface position:

- **`hash3(p)`** — an **integer-lattice bit-mixing hash**, not
  `fract(sin(...))`. This is deliberate and load-bearing:
  `n_geo * DITHER_SCALE` (≈ `n_geo * 400`) pushes the integer cell
  indices into the hundreds, where f32 `sin()` loses precision and
  develops visible banding. `p` arrives integer-valued (the floored cell
  corner); the hash casts through `i32`→`u32`, multiplies by three large
  primes, XOR-folds, and normalizes to `[0,1]`.
- **`value_noise_3d(p)`** — trilinear interpolation of `hash3` over the 8
  cube corners with a `f*f*(3−2f)` smootherstep fade. Single octave (a
  second octave like `wave_noise` is an easy quality bump if the grain
  reads too regular — keep it fixed-scale to preserve the coherent
  wipe).

**Why 3D noise on `n_geo` and not 2D UV / polar.** Hashing the 3D
surface position directly is **seamless and isotropic**: it has no
dateline seam and no pole pinch, with zero special-casing. A
`cos(lat)`-corrected polar parameterization was considered and rejected
(still degenerates at the poles). 3D noise sidesteps the problem.

### 2.4 The rewritten composite (`fs_main`, `globe.wgsl:212-310`)

Everything above the composite is **unchanged** from phase 1/2: the day
/night/normal/specular samples (lines 213-217), the perturbed normal `n`
in the analytic tangent frame (221-231), the Cook-Torrance GGX specular
with the ocean wave shimmer (233-265), `cos_sun = dot(n_geo, sun)` (270),
the atmosphere-filtered `sun_light` from the transmittance LUT (271-272),
and `day_lit` (274-277). The change is entirely in **how the final
surface color is assembled** (was, in phase 1/2, a single
`mix(night, day_lit, daylight)`).

The night texture is still sampled at line 214
(`let night = textureSample(night_texture, …).rgb;`) — it now feeds the
luminance mask, not the color.

Current composite (lines 282-310):

```wgsl
// Night side: day map darkened by sun geometry — no night texture as
// color. The GEOMETRIC normal feeds the terminator so bump detail
// doesn't speckle the day/night edge.
let daylight = smoothstep(-0.12, 0.18, cos_sun);
let night_factor = mix(NIGHT_DARKNESS, 1.0, daylight);
var surface = day_lit * night_factor;

// City mask: a single uniform luminance threshold on the night map.
let night_brightness = dot(night, vec3<f32>(0.2126, 0.7152, 0.0722));
let lit = smoothstep(EMISSIVE_THRESHOLD,
                     EMISSIVE_THRESHOLD + EMISSIVE_SOFTNESS,
                     night_brightness);

// Dither-dissolve across the terminator. fade: 0 deep on the night
// side (EMISSIVE_FADE_START) -> 1 at EMISSIVE_FADE_END; a positive END
// finishes the dissolve on the daylit side, so lights linger a little
// past the terminator before fully clearing.
let fade = smoothstep(EMISSIVE_FADE_START, EMISSIVE_FADE_END, cos_sun);

// Fixed-grain noise anchored to the 3D surface position: no crawl on
// zoom/rotate, and a stable per-pixel dissolve order under sun motion.
let dither = value_noise_3d(n_geo * DITHER_SCALE);
let keep = step(fade, dither);

surface += lit * keep * EMISSIVE_COLOR * EMISSIVE_STRENGTH;

return vec4<f32>(surface, 1.0);
```

**Walkthrough of why it works:**

- `night_factor = mix(NIGHT_DARKNESS, 1.0, daylight)` darkens (or, at the
  current 1.2, brightens) the day-lit color on the unlit hemisphere via
  the same **geometric** `daylight` smoothstep the old `mix` used —
  geometric, not bumped, so terrain relief never speckles the
  terminator.
- `lit` is a near-binary city mask from one fixed luminance threshold
  (`EMISSIVE_SOFTNESS` gives a 1-step soft edge that also absorbs BC7
  compression softness on the night map).
- `fade` ramps `0 → 1` from `EMISSIVE_FADE_START` up to
  `cos_sun = EMISSIVE_FADE_END` (now `0.15`, i.e. *onto* the daylit side).
- `dither = value_noise_3d(n_geo * DITHER_SCALE)` is **constant per
  surface point** (fixed scale, surface-anchored), so it never crawls or
  reshuffles under zoom/rotate/sun-motion.
- `keep = step(fade, dither)` is a **hard per-pixel dither**. Deep night
  (`fade ≈ 0`) → almost every pixel passes (full cities). As `cos_sun`
  rises toward `EMISSIVE_FADE_END`, `fade → 1`, so fewer noise cells clear
  the rising cutoff and the city stipples away. With the terminator at the
  **midpoint** of `[-0.15, 0.15]`, `fade ≈ 0.5` at `cos_sun = 0`, so
  **~half the city pixels survive past the terminator** and finish
  dissolving by `cos_sun = 0.15` on the daylit side — the intended bleed.
  Each survivor is full strength → the uniform brightness requested.
- **Fixed grain ⇒ coherent wipe.** Because each pixel's `dither` value is
  constant, it switches off precisely when `fade` crosses *its own* noise
  value. As the terminator sweeps (sun-slider drag or future time-of-day
  animation), pixels drop out in a **stable order** — a clean dissolve
  wipe, **no fizz/boil**. A frequency ramp (finer noise toward the
  terminator) was explicitly rejected for exactly this: it makes the
  per-point noise value uncorrelated between frames, so the band boils
  under sun motion. Cost of fixed grain: uniform detail (no
  finer-toward-terminator stippling); accepted for temporal stability.
- Additive `lit * keep * EMISSIVE_COLOR * EMISSIVE_STRENGTH` is the LDR
  "glow": a bright yellow that clips at white with `STRENGTH > 1` (same
  cheat as the `fs_stars` sun disc). No halo bleed — see caveats.

### 2.5 Design decisions and rejected alternatives (from PHASE3_PLAN)

The chain of reasoning, preserved so it isn't re-litigated:

- **Threshold-ramp toward the terminator → rejected.** City cores in the
  night map are clipped (saturated) plateaus; thresholding a plateau is
  all-or-nothing, so a rising threshold makes blobs vanish whole rather
  than shrink. A uniform threshold + dither shrinks them.
- **Morphological erosion → rejected.** Multi-tap neighborhood erosion is
  expensive and still needs a coverage ramp; the dither achieves the
  shrink in one tap.
- **Noise frequency ramp (finer toward terminator) → rejected.** Boils
  under sun motion (§2.4). Fixed grain chosen for a coherent wipe.
- **2D UV noise / polar coordinates → rejected.** Dateline seam and pole
  pinch. 3D noise on `n_geo` is seamless and isotropic.
- **`fract(sin())` hash → rejected.** Precision banding at the high
  lattice indices `DITHER_SCALE` produces. Integer bit-mixing hash
  instead.
- **Per-brightness glow scaling → rejected.** Glow is uniform strength;
  the owner wanted equal-brightness survivors, not night-map-modulated.

---

## 3. What did NOT change in phase 3

- **Bind group / layout** (PHASE2 §"Bind group 0 layout"): night texture
  stays at binding 3 and is still sampled. No `renderer.rs` change.
- **`build.rs`**: no new asset, transcode, or LUT change *in phase 3
  proper*. **(2026-06-17, post-phase-3):** the BC7 transcode's per-file
  `if dest.exists()` cache guard was removed — `transcode()` (renamed from
  `transcode_if_missing`) now runs unconditionally whenever the build
  script executes, leaning entirely on cargo's `rerun-if-changed` (emitted
  per asset in `download_if_missing`, plus `build.rs` itself) to decide
  *whether* it runs. Net effect: a no-change rebuild still skips the script
  via cargo's fingerprint, but **any** rerun — editing `build.rs`, or
  replacing/touching a texture — re-encodes **all five** 33 MP BC7 textures
  (~1.5 min wall), since there is no longer a per-output skip. The
  atmosphere LUT bake was already unconditional and is unchanged. An
  mtime-based guard and a content-hash cache were both considered and
  deliberately **not** adopted (owner preferred the pure `rerun-if-changed`
  model for now); revisit if the all-or-nothing re-encode becomes painful.
- **Rust / uniforms / pipelines**: untouched. Every new parameter is a
  WGSL `const`. (Exposing them as egui-driven uniforms is a noted
  follow-up — see §5.)
- **Stars and atmosphere passes**: untouched (`vs_stars`/`fs_stars`,
  `vs_atmosphere`/`fs_atmosphere`).
- **Everything in §1.**

### Correction to PHASE2's "Shader algorithms" section

PHASE2's closing section stated the WGSL was "byte-for-byte what phase 1
shipped" and described the surface as using "**night-side emissive city
lights blended across the terminator by `smoothstep(−0.12, 0.18,
cos_sun)`** … from the photographic night map." **That is now stale.**
As of phase 3:

- The night map is **not** displayed as color; the dark side is the
  **day map scaled by `night_factor`**.
- City lights are **procedural** (luminance mask + dither-dissolve +
  additive yellow), not the photographic texture.
- The `smoothstep(−0.12, 0.18, cos_sun)` `daylight` term still exists,
  but now drives `night_factor`, not a `mix(night, day_lit, …)`.

Everything else in PHASE2's shader/atmosphere summary (normal mapping,
GGX BRDF, ocean glint, transmittance-LUT sun color, the entire
atmosphere and stars math) remains accurate. PHASE1 stays the deep
reference for those.

---

## 4. Validation status

Smoke-tested per the PHASE2 pattern (`timeout 25 cargo run 2>&1 | head`):
a clean run with **no wgpu validation panic in the first frames**, which
means the WGSL compiles and pipelines/bindings are valid. One issue hit
and fixed during implementation: the noise variable was first named `n`,
colliding with the perturbed-normal `n` already in `fs_main` ("redefinition
of `n`"); renamed to `dither`.

**Not validated:** the actual look. Per PHASE2, WSLg here cannot judge
exact colors or interaction feel — the owner perf/feel-tests on **native
Windows release builds**. The current constants (especially
`NIGHT_DARKNESS = 1.2`, §2.2) appear to be mid-tuning and have **not**
been confirmed as a settled look.

The **06-17 `EMISSIVE_FADE_END` edit was not re-smoke-tested** in this
environment (no GPU). It is a one-constant addition plus a single
`smoothstep` argument swap (`0.0` → `EMISSIVE_FADE_END`) and is valid WGSL,
but note WGSL is compiled by naga **at runtime** in `create_shader_module`,
not during `cargo build` — so a clean `cargo build` proves nothing about
the shader. Treat both compile and look as unverified until run natively.

### Things to eyeball on native Windows

- Day side: unchanged day-map appearance.
- Night side: day map scaled by `night_factor` — **at the current 1.2 it
  will read brighter than day, not dark**; lowering toward the plan's
  ~0.02 gives the intended near-black night.
- Deep night: cities glow as solid bright-yellow blobs.
- Toward the terminator: cities dissolve to yellow sparkles; with
  `EMISSIVE_FADE_END = 0.15` roughly **half survive at `cos_sun = 0`** and
  finish dissolving by `cos_sun = 0.15`, so a thin lit-side rim still
  carries lights (the 06-17 bleed change). At `FADE_END = 0` they are gone
  exactly at the terminator instead — a quick A/B to confirm the knob.
- Zoom/rotate: dither pattern stays locked to the surface (no crawl).
- **Moving the sun sliders sweeps the dissolve as a coherent wipe** —
  pixels switch off in place, no fizz/boil (the test that the
  fixed-grain decision works).

---

## 5. Caveats and follow-ups

Carried from PHASE3_PLAN / PHASE2, still current:

- **No HDR/bloom.** LDR straight to the non-sRGB swapchain. "Glow" is
  bright additive yellow that *clips* at white — no halo bleed. A real
  bloom halo needs a post-process pass: **out of scope, declined.**
- **Non-sRGB surface calibration.** All look-tuning constants (the new
  emissive ones included) are calibrated to the **non-sRGB** swapchain;
  changing the surface transfer function invalidates them.
- **Dither aliasing when zoomed out.** Surface-anchored noise at a high
  `DITHER_SCALE` can twinkle as city blobs shrink toward sub-pixel at low
  zoom (no MSAA, no night-map mips). If it sparkles, lower `DITHER_SCALE`
  or swap `step(fade, dither)` for a narrow `smoothstep` (trades crisp
  dither for a 1-px-soft dither).
- **BC7 night map is lossy** — luminance thresholding a compressed source
  gives slightly soft/blocky mask edges; `EMISSIVE_SOFTNESS` absorbs it.
- **Terminator speckle** — night darkening uses the **geometric**
  `daylight`, not the bumped `n_dot_l`, to keep the edge clean. Don't
  switch it to the perturbed normal.

**Follow-ups (not done):**

- Settle `NIGHT_DARKNESS` / `EMISSIVE_THRESHOLD` on a native build (the
  values are currently mid-tuning).
- Expose `EMISSIVE_THRESHOLD` / `EMISSIVE_STRENGTH` / color / fade range
  as uniforms + egui controls for interactive tuning (uniform field + UI
  edit; see PHASE2 §UI and §Uniforms).
- Second noise octave in `value_noise_3d` if the single-octave grain
  reads too regular (keep it fixed-scale).
- Real bloom post-process for an actual glow halo (new pass, large) —
  explicitly declined.
