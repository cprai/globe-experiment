# CLAUDE.md

Project rules, conventions, and constraints for the **Globe** viewer.
Companion file: `MEMORY.md` holds the technical reference (how each
subsystem works, the rendering/atmosphere math, exact constants, file
map, phase history). Read both. When `MEMORY.md` and the source disagree,
**the source wins** — the look-tuning constants in particular drift
between sessions.

This file supersedes the old `claude/phase{1,2,3}/*.md` docs, which have
been deleted and folded into here and `MEMORY.md`.

---

## What this is (one paragraph)

An interactive Google-Earth-style 3D globe viewer. Rust (edition 2024),
**winit 0.30** window/event loop, **wgpu 29** (direct dependency) for
rendering, **egui 0.34** for the sun-slider overlay. It renders a
physically lit Earth (day/night, procedural city lights, normal-mapped
relief, GGX ocean glint), a Hillaire-2020 precomputed-LUT atmosphere, and
a star/sun backdrop, with an orbital pan/tilt/zoom camera. The crate is
named `globe-experiment`; **iced is no longer a dependency** (removed in
phase 2) — do not reintroduce it.

## Build & run

```sh
cargo run --release
```

- **First build needs a network connection** and is slow (~1.5 min extra):
  `build.rs` downloads five textures into the gitignored `assets/` and
  BC7-encodes them once. Subsequent builds reuse the cached `OUT_DIR/*.ktx2`.
- Smoke test: `timeout <n> cargo run 2>&1 | head` (or redirect to a file —
  pipe buffering can swallow output). wgpu validation errors panic in the
  first frames, so a clean 15-25 s run means pipelines/bindings are valid.
- **WGSL is compiled by naga at runtime** in `create_shader_module`, *not*
  during `cargo build`. A clean `cargo build` proves nothing about the
  shader — you must actually run it.

---

## Golden rules (do not violate without asking the owner)

### Source format
- **All `.rs` files and `shaders/globe.wgsl` are pure ASCII**, comments and
  strings included (`—`→`-`, `°`→`deg`, `±`→`+/-`, `≈`→`~`, `×`→`x`/`*`).
  Keep new source ASCII-only. (Markdown docs like this one and `MEMORY.md`
  may use Unicode for math readability.)

### Constants that are duplicated and MUST stay in sync
- The **atmosphere medium + geometry constants exist twice**: in
  `build.rs`'s inline `mod atmosphere` (the LUT bake) and in
  `shaders/globe.wgsl` (the geometric twins + `MIE_G`). Change one, change
  the other, or the baked LUTs and the shader that samples them diverge.
- The **inscatter LUT parameterization is implemented independently twice**
  and must match exactly: the **split row mapping** (`v`: lower half =
  ground-hitting rays, upper half = limb rays), the **reference-point
  choice** (ground hit vs. closest approach), and the **Bruneton
  transmittance mapping** all appear in both `build.rs::bake_inscatter` /
  `bake_transmittance` / `sample_transmittance` and in
  `globe.wgsl::fs_atmosphere` / `sun_transmittance`. A change on one side
  silently corrupts the atmosphere.

### Surface / display
- **Surface format must be non-sRGB** (`Gfx::new` picks `find(|f|
  !f.is_srgb())`) and **present mode `AutoVsync`**. `get_default_config`'s
  defaults are *not* correct here (it picks an sRGB format and Mailbox on
  DX12) — these two overrides are deliberate. See `MEMORY.md` for why.
- **Every look-tuning constant in `globe.wgsl` is calibrated to the
  non-sRGB surface.** Moving to a colorimetrically correct sRGB target
  invalidates all of them and requires re-tuning the entire shader.
- **No HDR, no bloom.** Output is LDR straight to the swapchain. "Glow" and
  "brightness" everywhere (sun disc, city lights) are the LDR cheat: a
  clipped-white core inside an additive soft falloff. A real bloom pass is
  an explicitly **declined** follow-up — do not assume it exists.

### Rendering invariants
- **No depth buffer anywhere.** Draw order does all occlusion: stars →
  surface → atmosphere, in that order, into one render pass. This only
  works because the scene is convex spheres. Do not add geometry that
  breaks that assumption without also adding a depth attachment.
- **Idle = zero GPU work.** `main()` sets `ControlFlow::Wait`; frames are
  driven *only* by targeted `window.request_redraw()` (input changed the
  camera, inertia/zoom glide coasting, egui repaint, resize, surface
  recovery). **Never add an unconditional vsync render loop.**
- **The terminator / night-side darkening must use the GEOMETRIC normal**
  (`cos_sun = dot(n_geo, sun)`, the `daylight` smoothstep), never the
  bump-mapped normal `n`. Bump detail on the day/night edge speckles it.
- **Star map is rigidly attached to the sun** (`star_rotation` =
  `R_y(lon)·R_x(−lat)`). This is deliberately **non-physical** — the owner
  rejected the astronomically-correct model (sky rotating only about the
  polar axis) because both sliders then spun the sky around one axis. Do
  not "fix" it to be astronomically correct.
- **Backdrop anchoring**: both the star lookup and the sun disc are
  functions of the **camera-relative view direction** (`world − camera_pos`),
  not of position on the sky sphere. Anchoring either to the sky-sphere
  surface reintroduces parallax between sun and stars (a fixed bug). Do not
  regress it.

### Startup / window
- **The first frame must render via a direct `self.redraw()` call** from
  `resumed()`, never `request_redraw()`. The window starts hidden
  (`with_visible(false)`) and is revealed after the first `present()`;
  Windows does not deliver `RedrawRequested` to a hidden window, so a
  requested redraw would never fire and the window would stay invisible
  forever. There is also an `Occluded` first-frame guard for backends that
  report a hidden window as occluded. **Do not "simplify" either.**

### Input feel
- **Do not restructure the smoothed-zoom glide/coast in `input.rs`.** It
  was iterated ~5 times with the owner (including a full remove-then-revert).
  Tune only the named constants (`ZOOM_HALF_LIFE_MIN/MAX`,
  `ZOOM_COAST_HALF_LIFE`, `ZOOM_STOP_RATE`). Rejected designs that must not
  be reintroduced: fixed-half-life always-glide, and a fixed burst-gap
  split (instant if gap < threshold, glide otherwise). See `MEMORY.md`.

### Tuning discipline
- **Look-tuning constants drift between sessions.** Always read
  `globe.wgsl` for current values; never trust a doc's numbers. The
  `MEMORY.md` snapshot is dated and will go stale.
- **Tune and feel-test on a native Windows release build.** The WSLg dev
  environment here cannot validate exact colors or interaction feel.

---

## Deliberately reverted / rejected — do not re-add without asking

These were implemented and then removed (or considered and declined) by
the owner. Re-introducing them silently is a regression.

- **Animated/time-varying ocean wave noise** — made static on purpose
  (temporal stability + idle-is-free). The wave noise is fixed.
- **Day-side albedo saturation boost** and an **`OCEAN_TINT`
  water-darkening tint** — both reverted; albedo is used as sampled.
- **The astronomically-correct star model** — rejected (see above).
- **Noise *frequency* ramp toward the terminator** for the city-light
  dissolve — rejected because it boils/fizzes under sun motion. The dither
  uses a **fixed** `DITHER_SCALE` for a coherent wipe.
- **Terminator-varying emissive threshold** — rejected (night-map city
  cores are clipped plateaus, so a rising threshold pops whole blobs
  instead of shrinking them). Use the uniform threshold + dither.
- **Per-brightness glow scaling** — rejected; surviving city pixels glow at
  uniform strength.
- A **`thread::scope` parallel texture *decode* + LUT bake** at startup —
  reverted in phase 1. NOTE: this is *not* the same as the current
  rayon parallelization of `GlobeRenderer::new` (module compile + KTX2
  uploads + pipeline compiles), which is **intentional** and was added on
  explicit request after decode/bake moved to build time. Do not confuse
  the two; do not "re-revert" the rayon code.

---

## Conventions

### Coordinate & mapping (used consistently everywhere — see `MEMORY.md` for formulas)
- Globe is a **unit sphere at the origin**; distances in **globe radii**.
  **+Y is north.** Longitude 0°, latitude 0° faces **+Z**.
- **Load-bearing identity**: because the mesh is a unit sphere at the
  origin, for any surface fragment **vertex position = surface normal =
  world position**. The shaders rely on this throughout.
- Equirectangular UVs: `u = (lon+180)/360` (wraps; sampler repeats on U),
  `v = 0` at north pole → `1` at south (sampler clamps on V). The mesh
  duplicates the seam column so U wraps cleanly.
- Atmosphere math works in **kilometers** (positions scaled by
  `PLANET_RADIUS_KM`); surface/star passes work in **globe radii**.

### Code style
- Match the surrounding code: dense, explanatory comments that capture
  *why* (especially the non-obvious GPU/winit/precision reasons), small
  focused structs, descriptive names. The existing files are the style
  guide.
- Each subsystem is one module under `src/globe/`; `globe/mod.rs` is
  declarations only (no logic).

### Where things live
- **Shader look knobs**: top of `shaders/globe.wgsl` (`const` block).
- **Atmosphere medium constants**: `build.rs` `mod atmosphere` (bake side)
  *and* `shaders/globe.wgsl` (geometric twins).
- **Input feel constants**: top of `src/globe/input.rs`.
- **Camera limits**: `Camera` associated consts in `src/globe/camera.rs`.
- All baked/transcoded assets land in `OUT_DIR` and are `include_bytes!`-ed.

---

## Hard constraints (environment & platform)

- **Device requires `Features::TEXTURE_COMPRESSION_BC`** (for the BC7
  textures). Universal on desktop GPUs; works under WSLg lavapipe too.
- **No mipmaps** (`mip_level_count 1` on every texture). Known shimmer at
  far zoom; the city-light dither can twinkle/alias when blobs shrink
  sub-pixel at low zoom (no MSAA either). Mitigations are documented but
  not implemented.
- **Large binary**: ~160 MB of BC7 + ~0.6 MB LUTs are embedded via
  `include_bytes!`, so the binary is large and links slowly. Runtime file
  loading is a known, unimplemented follow-up.
- **`.cargo/config.toml`** adds `-lstdc++` on `x86_64-unknown-linux-gnu`
  **only** — `intel_tex_2`'s prebuilt ISPC objects need the GCC C++
  personality. MSVC on Windows is unaffected; do not make it
  cross-platform.
- **WSLg flakiness**: app launch intermittently fails with libEGL/MESA
  errors — transient, retry; not a code bug. Not present on native Windows.
- **Windows mount tooling**: `cargo add` can emit a bogus "found cargo.toml
  please rename" error on the case-insensitive mount — edit `Cargo.toml`
  directly and trust `cargo metadata`.

---

## Testing & verification

- There is **no test suite and no CI**. Verification is the smoke test
  above plus manual interaction on native Windows.
- Manual pass to run after risky changes: pan, flick (inertia), zoom to
  min/max, tilt to clamp, both sun sliders, window resize,
  minimize/restore. Confirm idle renders **zero** frames.
- After atmosphere-constant or mapping changes, verify **both** the bake
  (`build.rs`) and shader (`globe.wgsl`) sides and re-run — bit-identical
  output is the goal when the change is meant to be neutral.
