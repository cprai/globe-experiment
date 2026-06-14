# PHASE3 — Emissive city-light glow with terminator dither-dissolve

Plan written 2026-06-13. Scope: a **shader-only** change to
`shaders/globe.wgsl` (fragment stage `fs_main` plus two small noise
helpers). No Rust, no `build.rs`, no bind-group/layout changes, no new
textures or downloads. Read `claude/phase2/PHASE2.md` first for the
surrounding architecture; the load-bearing facts for this change are
summarized inline below.

## Goal

Replace the photographic night map as the dark-side color with a
day-textured globe plus procedurally re-lit, glowing-yellow city lights
that dither away as they approach the day side:

1. **Light the whole globe with the day map.** The night texture is no
   longer used as a color anywhere — the entire sphere is shaded from
   `day_texture`, and the night hemisphere is darkened to near-black by
   sun geometry.
2. **Uniform-threshold emissive mask.** A pixel is "city" if its
   night-map luminance clears a single fixed threshold. (No
   terminator-varying threshold — that was tried and rejected: the night
   map's city cores are clipped plateaus, so raising the threshold makes
   blobs pop out whole instead of shrinking.)
3. **Dither-dissolve toward the terminator.** A fixed-grain noise dither
   erodes the city pixels stochastically as they near the lit side, so
   each city thins out into sparkles and is **fully gone at the geometric
   terminator** (`cos_sun = 0`). Surviving pixels stay at full uniform
   brightness (binary keep/discard — no dimming).
4. **Bright yellow glow.** Surviving city pixels add a bright yellow,
   self-lit term (additive LDR — see no-bloom caveat).

## Confirmed decisions (2026-06-13)

- **Day map everywhere; near-black night.** `NIGHT_DARKNESS ≈ 0.02`. No
  night-map color on the dark side.
- **Uniform glow strength, bright yellow.** Every surviving city pixel
  glows at the same strength — not scaled by night-map brightness.
- **Dither-dissolve, not erosion or threshold-ramp.** Uniform threshold
  builds the mask; a noise dither dissolves it.
- **Single coverage ramp on the dither**, driven by sun geometry: the
  kept fraction goes from full (deep night) to **zero exactly at the
  terminator** (`cos_sun = 0`). This is what makes the lights disappear;
  no city light bleeds onto the lit day side.
- **Fixed grain — no scale ramp** (decided 2026-06-13). An earlier design
  ramped the noise *frequency* finer toward the terminator, but that
  makes the noise value at a point uncorrelated between frames as the sun
  moves, so the dissolving band would **fizz/boil** under a sun-slider
  drag or the planned time-of-day animation. A single fixed
  `DITHER_SCALE` instead gives every pixel a **stable dissolve order**
  (its noise value = the `cos_sun` at which it switches off), so cities
  erode as a **coherent wipe** as the terminator sweeps — clean under
  animation. Cost: uniform grain (no finer-toward-terminator detail);
  accepted for temporal stability.
- **Noise anchored to the 3D surface position** (`n_geo`), so it never
  crawls when zooming or rotating — it's nailed to the globe. With fixed
  grain it's also stable as the sun moves: pixels switch off in place,
  one by one, rather than the pattern reshuffling.
- **3D noise, not 2D UV noise.** Hashing `n_geo` directly is seamless and
  isotropic — it avoids both the dateline seam and pole pinching with no
  special-casing. (A `cos(lat)`-corrected polar parameterization was
  considered and rejected: it still degenerates at the poles. 3D noise
  sidesteps the problem entirely.)
- **Additive LDR glow, no bloom.** Shader-only; bright clipped yellow
  (the sun-disc LDR cheat). A real haloing bloom pass remains an
  explicitly-declined follow-up.

## Current behavior (what we're replacing)

In `shaders/globe.wgsl`, `fs_main`:

- `night` is sampled at the top of the fragment (`globe.wgsl:154`) and
  used **as the night-side color**.
- `cos_sun = dot(n_geo, sun)` at `globe.wgsl:210`; the day-lit color
  `day_lit` is built at `globe.wgsl:214-217`.
- The final composite at `globe.wgsl:222-223`:

  ```wgsl
  let daylight = smoothstep(-0.12, 0.18, cos_sun);
  let surface = mix(night, day_lit, daylight);
  ```

  The **geometric** normal feeds `cos_sun`/`daylight` deliberately, so
  bump detail doesn't speckle the terminator.

Existing 2D noise helpers `hash2` / `value_noise` / `wave_noise` live at
`globe.wgsl:67-89` (used by the ocean glint). We add 3D siblings next to
them.

## Desired behavior (the new composite)

### New noise helpers (add near `globe.wgsl:67-89`)

A 3D value noise so the dither can be sampled on the sphere surface
position seamlessly:

```wgsl
// Integer-lattice bit-mixing hash. Precision-safe at large coordinates,
// unlike fract(sin(...)) — important here because n_geo * DITHER_SCALE
// pushes the lattice indices into the hundreds, where f32 sin() loses
// precision and the noise develops visible banding. p arrives as an
// integer-valued vec3 (the floored cell corner).
fn hash3(p: vec3<f32>) -> f32 {
    var n = u32(i32(p.x)) * 1597334677u
        ^ u32(i32(p.y)) * 3812015801u
        ^ u32(i32(p.z)) * 2369874511u;
    n = (n ^ (n >> 15u)) * 2246822519u;
    n = (n ^ (n >> 13u)) * 3266489917u;
    n = n ^ (n >> 16u);
    return f32(n) / 4294967295.0;
}

// Trilinearly-interpolated 3D value noise. Sampled at the unit-sphere
// surface position, so it has no seam and no pole pinch.
fn value_noise_3d(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);

    let c000 = hash3(i + vec3<f32>(0.0, 0.0, 0.0));
    let c100 = hash3(i + vec3<f32>(1.0, 0.0, 0.0));
    let c010 = hash3(i + vec3<f32>(0.0, 1.0, 0.0));
    let c110 = hash3(i + vec3<f32>(1.0, 1.0, 0.0));
    let c001 = hash3(i + vec3<f32>(0.0, 0.0, 1.0));
    let c101 = hash3(i + vec3<f32>(1.0, 0.0, 1.0));
    let c011 = hash3(i + vec3<f32>(0.0, 1.0, 1.0));
    let c111 = hash3(i + vec3<f32>(1.0, 1.0, 1.0));

    let x00 = mix(c000, c100, u.x);
    let x10 = mix(c010, c110, u.x);
    let x01 = mix(c001, c101, u.x);
    let x11 = mix(c011, c111, u.x);
    let y0 = mix(x00, x10, u.y);
    let y1 = mix(x01, x11, u.y);
    return mix(y0, y1, u.z);
}
```

(Single octave is the starting point; a second octave like `wave_noise`
uses is an easy quality bump if the grain looks too regular.)

### New fragment logic (replaces `globe.wgsl:222-223`)

Keep `cos_sun` and the existing `daylight` exactly as they are, then:

```wgsl
// --- Night side: day map darkened by sun geometry (no night texture as color) ---
let daylight = smoothstep(-0.12, 0.18, cos_sun);   // existing
let night_factor = mix(NIGHT_DARKNESS, 1.0, daylight);
var surface = day_lit * night_factor;

// --- City mask: uniform luminance threshold on the night map ---
let night_brightness = dot(night, vec3<f32>(0.2126, 0.7152, 0.0722));
let lit = smoothstep(
    EMISSIVE_THRESHOLD,
    EMISSIVE_THRESHOLD + EMISSIVE_SOFTNESS,
    night_brightness,
);

// --- Dither-dissolve toward the terminator ---
// fade: 0 deep on the night side (EMISSIVE_FADE_START), 1 at the
// terminator (cos_sun = 0). Drives the coverage cutoff off sun geometry
// so the "fully gone at the terminator" alignment is exact.
let fade = smoothstep(EMISSIVE_FADE_START, 0.0, cos_sun);

// Fixed-grain noise anchored to the 3D surface position -> no crawl on
// zoom/rotate, and a stable per-pixel dissolve order under sun motion.
let n = value_noise_3d(n_geo * DITHER_SCALE);

// Coverage cutoff ramps to 1 at the terminator, so all city pixels are
// discarded by cos_sun = 0. step() = hard per-pixel dither (uniform
// brightness for survivors); each pixel switches off at fade == its own
// noise value, giving a coherent wipe rather than a reshuffle.
let keep = step(fade, n);

surface += lit * keep * EMISSIVE_COLOR * EMISSIVE_STRENGTH;

return vec4<f32>(surface, 1.0);
```

Why this produces the wanted look:

- `lit` is a near-binary city mask from a single fixed threshold.
- `keep = step(fade, n)`: deep on the night side `fade ≈ 0`, so nearly
  every pixel passes (full cities). As `cos_sun → 0`, `fade → 1`, so
  fewer noise cells clear the cutoff and the cities stipple away to
  nothing right at the terminator. Each survivor is full-strength → the
  uniform brightness you asked for.
- Fixed `DITHER_SCALE` means each pixel's noise value `n` is constant, so
  it switches off precisely when `fade` crosses `n`. As the terminator
  sweeps, pixels drop out in a stable order — a coherent dissolve wipe,
  no boiling under sun motion.
- Sampling `value_noise_3d(n_geo * DITHER_SCALE)` ties the pattern to the
  globe surface: zooming/rotating/sun-motion never makes the grain crawl
  or reshuffle.

### New constants (add to the look-tuning block, `globe.wgsl:44-65`)

Starting values — all feel knobs, tune on a native Windows release build
(see caveats):

```wgsl
// --- Emissive city lights (procedural, from the night map's brightness) ---
// A pixel is a "city" when its night-map luminance clears the threshold.
const EMISSIVE_THRESHOLD: f32 = 0.25;
const EMISSIVE_SOFTNESS: f32 = 0.1;
// Bright yellow glow; STRENGTH > 1 drives the core toward clip (LDR).
const EMISSIVE_COLOR: vec3<f32> = vec3<f32>(1.0, 0.85, 0.3);
const EMISSIVE_STRENGTH: f32 = 1.5;
// Dither-dissolve: begins at this sun cosine (deeper night = more
// negative) and completes at the terminator (cos_sun = 0).
const EMISSIVE_FADE_START: f32 = -0.15;
// Noise grain (cells across the unit sphere). Fixed — no terminator ramp,
// for a temporally coherent dissolve under sun motion.
const DITHER_SCALE: f32 = 400.0;
// How dark the day map goes on the unlit hemisphere (0 = black night).
const NIGHT_DARKNESS: f32 = 0.02;
```

`DAY_AMBIENT` (`globe.wgsl:44`, currently 0.04) interacts with
`NIGHT_DARKNESS`: ambient sets the floor of `day_lit`, then
`night_factor` scales it on the dark side. If the night side reads too
bright/flat, lower `DAY_AMBIENT` first, then `NIGHT_DARKNESS`.

## Step-by-step edit list

All edits in `shaders/globe.wgsl`:

1. Add `hash3` and `value_noise_3d` near the existing noise helpers
   (`globe.wgsl:67-89`).
2. Add the new constants to the look-tuning block (`globe.wgsl:44-65`).
3. Keep the `night` sample (`globe.wgsl:154`) — it now feeds the
   luminance mask, not color. Leave the binding/sample as-is.
4. Replace the final composite (`globe.wgsl:222-223`, the
   `let surface = mix(night, day_lit, daylight);` line and its `return`)
   with the new fragment block above (keep the existing `daylight`
   line). End in `return vec4<f32>(surface, 1.0);`.

Stars and atmosphere passes are untouched.

## What does NOT change

- **Bind group / layout** (PHASE2 §"Bind group 0 layout"): night texture
  stays at binding 3; still sampled. No layout edit, no `renderer.rs`
  change.
- **`build.rs`**: no new asset, transcode, or LUT change. The night map
  is already downloaded and BC7-transcoded.
- **Rust / uniforms / pipelines**: unchanged. All new parameters are
  shader constants. (Exposing them as egui-driven uniforms is a noted
  follow-up, not this plan.)

## Caveats (carried from PHASE2)

- **No HDR/bloom** (PHASE2 §"Known issues"): LDR straight to a non-sRGB
  swapchain. "Glow" = bright additive yellow that *clips* at white, no
  halo bleed. `EMISSIVE_STRENGTH > 1` gives a clipped-white-cored,
  yellow-fringed point (same LDR cheat as `fs_stars`' sun). A real bloom
  halo would need a post-process pass — **out of scope, declined**.
- **Non-sRGB surface calibration** (PHASE2 §Gfx::new): all look-tuning
  constants are calibrated to the non-sRGB swapchain. Tune the new
  constants the same way and verify on a **native Windows release
  build** — WSLg here can't validate exact colors or interaction feel.
- **Terminator speckle**: night darkening uses the geometric `daylight`,
  not the bumped `n_dot_l`, to keep the day/night edge clean (the
  original blend did the same). Don't switch it to the perturbed normal.
- **Dither aliasing when zoomed out**: surface-anchored noise at a high
  `DITHER_SCALE` can twinkle/alias when city blobs shrink toward
  sub-pixel at low zoom (no MSAA, no mips on the night map). If it
  sparkles, lower `DITHER_SCALE` or soften `keep` with a `smoothstep`
  instead of `step` (trades crisp dither for a 1-px-soft dither).
- **BC7 night map is lossy**: luminance thresholding on a compressed
  source can give slightly soft/blocky mask edges; `EMISSIVE_SOFTNESS`
  absorbs this.

## Testing

Per the PHASE2 smoke-test pattern:

```
timeout 25 cargo run 2>&1 | head
```

A clean 15–25 s run (no wgpu validation panic in the first frames) means
the WGSL still compiles and pipelines/bindings are valid. Then eyeball,
ideally on a native Windows release build:

- Day side: unchanged day-map appearance.
- Night side: day map darkened to near-black (`NIGHT_DARKNESS`), **no
  photographic city-lights texture**.
- Deep night: cities glow as solid bright-yellow blobs.
- Toward the terminator: cities dissolve into yellow sparkles and are
  **fully gone by `cos_sun = 0`** — none bleed onto the lit day side.
- Zoom in/out and rotate: the dither pattern stays locked to the surface
  (no crawl). Moving the **sun** sliders should sweep the dissolve as a
  coherent wipe — pixels switch off in place, **no fizz/boil** (the test
  that the fixed-grain decision is working).

## Follow-ups (not in this plan)

- Expose `EMISSIVE_THRESHOLD` / `EMISSIVE_STRENGTH` / color / fade range
  as uniforms + egui controls for interactive tuning (uniform field + UI
  edit; see PHASE2 §UI and §Uniforms).
- Second noise octave in `value_noise_3d` if the single-octave grain
  looks too regular (keep it fixed-scale to preserve the coherent wipe).
- A real bloom post-process for an actual glow halo (new pass; large) —
  explicitly declined for now.
