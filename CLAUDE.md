# CLAUDE.md

Project rules, conventions, and constraints for **Globe**, an
astronomically-accurate satellite simulation tool (past scenarios only).
Companion: `MEMORY.md` holds the technical reference (architecture, math,
phase history, file map, satkit API, exact constants). Read both. When
`MEMORY.md` and the source disagree, **the source wins** — look-tuning
constants in particular drift between sessions.

---

## What this is

Rust (edition 2024), winit 0.30, wgpu 29, egui 0.34. Physically-lit WGS84
globe in world-space km, Hillaire-2020 atmosphere, star/sun from JPL DE440
ephemeris + real EOP, satellite TLE tracking via satkit SGP4, inertial
(star-fixed) camera, simulation clock (1x-100x, plays from launch).
**Past scenarios only** (before build date) — what makes full EOP accuracy
attainable. The crate is named `globe-experiment`; `iced` is gone, do not
reintroduce it. See `MEMORY.md` §1 for the full stack and file map.

## Build & run

```sh
cargo run --release
cargo run --release -- render --datetime 2024-01-15T12:30:00Z \
    --longitude -75 --latitude 40 --distance 12742 --tilt 0 --output frame.png
```

First build: slow (~1.5 min extra), needs network. `build.rs` downloads 5
textures (JPEG/TIFF verbatim), the JPL ephemeris (~98 MB), and
`EOP-All.csv` into `OUT_DIR`; bakes 3 atmosphere LUTs as f16 KTX2.
Subsequent builds reuse cached files. Delete a file in `OUT_DIR` to
re-download it.

**WGSL is compiled by naga at runtime, not during `cargo build`.** A clean
build proves nothing about the shader. Validate with:

```sh
naga --compact --capabilities none shaders/globe.wgsl
```

after every shader edit (see Testing & verification).

---

## Golden rules (do not violate without asking the owner)

### Platform compatibility (load-bearing)
- **Supported matrix: Windows/Linux/macOS x both x86_64 and aarch64** (six
  targets). Regressions here are bugs, same as accuracy regressions.
- **`Features::empty()`** — no optional GPU features. Textures upload
  **uncompressed** (`Rgba8Unorm`/`Rgba8UnormSrgb`); no BC/ASTC requirement.
  Do not re-add `TEXTURE_COMPRESSION_BC` — it panics on Apple Silicon.
- **No host-arch or OS assumptions in the build.** `build.rs` and deps must
  compile natively on all six targets (including aarch64). No single-arch
  prebuilt build tools or platform-gated link flags without owner sign-off.
- Dev sandbox is x86_64 Linux + lavapipe — cannot prove macOS/aarch64
  behavior. Reason portability through against this rule; call out what
  needs hardware confirmation.

### Source format
- **All `.rs` files and `shaders/globe.wgsl` are pure ASCII**: `—` → `-`,
  `°` → `deg`, `±` → `+/-`, `≈` → `~`, `×` → `x`/`*`. Markdown docs may
  use Unicode.

### Constants that must stay in sync
- **Atmosphere medium + geometry constants** exist in `build.rs mod
  atmosphere` (LUT bake) AND `shaders/globe.wgsl` (geometric twins +
  `MIE_G`). Change one, change the other — a mismatch silently corrupts
  the atmosphere.
- **Inscatter LUT parameterization** (split row mapping, reference-point
  choice, Bruneton transmittance mapping) is independently implemented in
  both `build.rs` and `globe.wgsl`. A mismatch silently corrupts the
  atmosphere.

### Surface / display
- **Surface format: non-sRGB** (`Gfx::init` picks `find(|f| !f.is_srgb())`);
  **present mode: `AutoVsync`**. `get_default_config` defaults are wrong on
  both (sRGB format + Mailbox on DX12). Both overrides are deliberate.
- Every look-tuning constant in `globe.wgsl` is calibrated to the non-sRGB
  surface. Do not switch to sRGB.
- **Headless render target must also be non-sRGB** (`Rgba8Unorm`). Do not
  "fix" `HeadlessRenderer` to `Rgba8UnormSrgb`.
- **No HDR, no bloom.** LDR only. A real bloom pass is explicitly declined.

### Rendering invariants
- **No depth buffer.** Draw order handles all occlusion: stars → surface →
  atmosphere → markers, one render pass. Convex-sphere assumption; don't
  add geometry that breaks it without adding a depth attachment.
- **Markers are instanced screen-space overlays** drawn last. CPU occlusion
  per marker (`marker_occluded` in `src/simulation/mod.rs`). No depth.
- **Idle = zero GPU work.** `ControlFlow::Wait` + targeted
  `request_redraw`. Never add an unconditional vsync loop. The clock starts
  playing, so the app is non-idle from launch until paused.
- **Terminator / night-side darkening must use the GEOMETRIC normal**
  (`dot(n_geo, sun)`), never the bump-mapped normal `n`. Bump detail on
  the day/night edge speckles it.
- **Star map and Sun are ephemeris-driven** (`src/simulation/celestial_sphere.rs`).
  `sun_dir` from JPL DE440; `star_rot_inv = P · R_itrf→gcrf · Pᵀ`. Do not
  replace with a sun-attached rotation.
- **`init_satkit()` must seed both the ephemeris and the EOP table** before
  any satkit use (called once at the start of each scenario's `run()`).
  Without the EOP seed, satkit creates a stray `satkit-data` dir next to
  the binary and every frame transform silently falls back to zeros for
  EOP. Also call `disable_eop_time_warning()`. Going full IERS-2010 for the
  celestial sphere additionally needs the IERS nutation tables (Tab5A/5B/5D)
  bundled and seeded — see `MEMORY.md` §14. **Do not drop the EOP seed.**
- **Camera is in the inertial (star-fixed) frame.** Built in the celestial
  frame, rotated into world by
  `celestial_to_world = star_rot_inv.transpose()` (via
  `SimulationState::celestial_to_world()`, applied in
  `ApplicationState::redraw`). `Camera.longitude/latitude` are inertial
  directions, not geography. Don't move the camera into the ECEF/world frame.
- **Backdrop anchoring**: star lookup and sun disc are functions of the
  camera-relative view direction, not position on the celestial sphere.
  Changing this reintroduces parallax between sun and stars (a fixed bug).

### Startup / window
- **First frame: `self.redraw()` directly from `ApplicationState::resumed()`**,
  never `request_redraw()`. Windows does not deliver `RedrawRequested` to a
  hidden window. The reveal is driven by `FrameOutcome` from `Gfx::update`.
  **Do not "simplify" either.**
- **`egui textures_delta.set` must apply BEFORE the surface acquire in
  `Gfx::update`.** egui emits each texture delta exactly once; a missed
  allocation delta causes a panic ("Tried to update a texture that has not
  been allocated yet") on the next partial atlas update. The `free` deltas
  deliberately stay after present. See `MEMORY.md` §3.

### Input feel
- **Do not restructure the smoothed-zoom glide/coast in `input.rs`.** Tune
  only the named constants (`ZOOM_HALF_LIFE_MIN/MAX`, `ZOOM_COAST_HALF_LIFE`,
  `ZOOM_STOP_RATE`). Rejected designs that must not return: fixed-half-life
  always-glide; fixed burst-gap split. See `MEMORY.md` §5.
- **macOS input feel is unvalidated** (no native hardware here). Validate on
  a real Mac before changing the trackpad `/ 60.0` divisor or tilt
  (right-drag) binding.

### Tuning discipline
- **Look-tuning constants drift between sessions.** Always read `globe.wgsl`
  for live values; `MEMORY.md` §13 is a dated snapshot.
- **Tune and feel-test on a native Windows release build.** The WSLg dev
  environment cannot validate exact colors or interaction feel.

### Documentation
- **Keep all docs current in the same change**: code comments, `CLAUDE.md`,
  `MEMORY.md`, and `README.md`. Stale docs are bugs.
- One exception: `MEMORY.md` §13 (live constant snapshot) may lag owner
  tuning — for live values the source is authoritative.

---

## Deliberately reverted / rejected — do not re-add

- **Animated/time-varying ocean wave noise** — static by design (temporal
  stability + idle-is-free).
- **Day-side albedo saturation boost** and **`OCEAN_TINT`** water-darkening
  tint — both reverted; albedo is used as sampled.
- **Sun-attached celestial sphere** (the slider-era model) — replaced by
  ephemeris-driven on 2026-06-18. Do not reintroduce the old non-physical
  model.
- **Noise frequency ramp toward the terminator** for the city-light dissolve
  — boils/fizzes under sun motion. `DITHER_SCALE` is fixed.
- **Terminator-varying emissive threshold** — rejected (blob popping). Use
  the uniform threshold + dither.
- **Per-brightness glow scaling** — rejected; uniform glow strength.
- **`thread::scope` parallel texture decode** — reverted in phase 1. The
  sanctioned runtime decode is rayon inside `GlobeRenderer::new`; the
  rejected thing is specifically the `thread::scope` design.

---

## TODO / backlog (confirm with owner before starting)

None is scheduled work or a bug. See `MEMORY.md` §14 for engineering
details on each.

- **Full IERS-2010 Earth orientation for the celestial sphere**: switch
  `*_approx` transforms to full `qgcrf2itrf`/`qitrf2gcrf`. Requires bundling
  satkit's IERS nutation tables (Tab5A/5B/5D) and seeding them in
  `init_satkit()`. Sub-pixel improvement; a consistency nicety, not a
  visible fix. See `MEMORY.md` §14.
- **Reconsider GPU texture compression (BC7 + ASTC)**: not a simple revert
  — Apple Silicon has no BC, so the full re-add needs both formats baked,
  runtime format selection from adapter caps, and a portable build-host
  encoder solution. Cheaper partial win: downsize textures to 4K (quarter
  VRAM, no feature needed). See `MEMORY.md` §14.

---

## Conventions

### Coordinate & mapping (see `MEMORY.md` for formulas)
- **WGS84 ellipsoid at origin; world space in km; +Y north; lon0/lat0 →
  +Z; +X east.** Constants and helpers in `src/earth.rs` (single source of
  truth; mesh and camera both call it).
- Mesh carries explicit **geodetic normals** (not `normalize(position)` on
  an ellipsoid). Atmosphere and star shells use the unit normal x their
  radii — spherical, not ellipsoid position.
- Equirectangular UVs: `u = (lon+180)/360` (wraps; sampler repeats on U),
  `v = 0` at north → 1 at south (sampler clamps on V). Seam column
  duplicated.
- Atmosphere stays spherical (`PLANET_RADIUS_KM 6360`, `ATMOSPHERE_TOP_KM
  6460`). Do not try to make the LUT model ellipsoidal.

### Code style
- Match surrounding code: dense explanatory comments on *why* (the non-
  obvious GPU/winit/precision reasons), small focused structs, descriptive
  names.
- Six top-level modules + `earth`: `application` (window, camera, input,
  egui logic), `simulation` (clock, satellites, celestial sphere — no
  winit/wgpu/egui, no `Camera` type), `renderer` (`Gfx` +
  `HeadlessRenderer`), `ui`, `earth`, `scenarios`, `snapshot`. See
  `MEMORY.md` §1 for the full file map.
- **`cargo +nightly fmt` after every `.rs` edit.** Nightly is required for
  `wrap_comments`; plain stable `cargo fmt` silently skips it. Never
  hand-format. After reflow, **scan diffs for formula-breaking line breaks
  and reword** to keep formulas on one line.
- **`wgslfmt shaders/globe.wgsl` after every shader edit.** Don't
  hand-format WGSL.

### Where things live
- **Shader look knobs**: `shaders/globe.wgsl` top `const` block.
- **Atmosphere medium constants**: `build.rs mod atmosphere` (bake) AND
  `shaders/globe.wgsl` (shader twins) — both.
- **Input feel constants**: `src/application/input.rs` top.
- **Earth physical constants + helpers**: `src/earth.rs`.
- **Camera limits**: `Camera` associated consts in
  `src/application/camera.rs` (in km).
- **All build assets** land in `OUT_DIR`, `include_bytes!`-ed. No `assets/`
  dir.
- Full file map: `MEMORY.md` §1.

### Scenarios & valid time range
- **Scenarios in `src/scenarios/`** — one module per past scenario with a
  `run()`. Add one by adding a module and a `ScenarioName` variant in
  `main.rs`. Each scenario owns its inline TLE `const`s; the `ISS_TLE`
  literal is **deliberately duplicated** across scenarios — do not factor it
  into a shared const.
- **Every scenario's epoch window must fall inside `[1962-01-01, last
  EOP-All.csv entry]`**:
  - Below 1962: EOP doesn't exist, satkit silently falls back to zeros.
  - Above last entry (the build date): satkit does constant extrapolation,
    silently degrading. Past-only keeps scenarios below this by construction.
  - Out-of-range = does not meet the accuracy bar. Flag it rather than
    shipping a silently-degraded result.
- **Render mode (`snapshot`) deliberately does NOT enforce the EOP range.**
  Do not add a check there.

---

## Hard constraints

- **`Features::empty()`** — no optional GPU features; textures uncompressed.
  Do not re-add any feature requirement.
- **No mipmaps** (`mip_level_count 1`). Known shimmer at far zoom; accepted.
- **8K textures at the portable limit**: `Limits::default()` guarantees
  `max_texture_dimension_2d = 8192` with zero headroom. Don't grow a texture
  past 8192 without raising the limit (narrows the matrix) or downsizing.
- **~670 MB VRAM** for 5 uncompressed 8K textures — accepted cost of the
  no-feature portability.
- **No `.cargo/config.toml`** — deleted when `intel_tex_2` was removed; its
  `-lstdc++` was its only purpose. Do not re-add.
- **Build requires a C compiler** (`ring` via `ureq` in `build.rs`, build-
  time only). Portable across all six targets. No pure-Rust workaround.
- **WSLg flakiness**: transient libEGL/MESA errors on app launch — retry,
  not a code bug.
- **Windows `cargo add`**: can emit a bogus "found cargo.toml please rename"
  error — edit `Cargo.toml` directly and trust `cargo metadata`.

---

## Testing & verification

- **No test suite, no CI.** Verification is the smoke test above plus manual
  interaction on native Windows.
- **`cargo clippy`** — run heavily, aim warning-free. Does not validate WGSL.
- **After every shader edit**: `naga --compact --capabilities none
  shaders/globe.wgsl`. This is the same naga the app links — authoritative.
  No output file = validate only. `Validation successful` + exit 0 = good.
  Keep the CLI version aligned with the wgpu/naga version in `Cargo.lock`.
- **`wgsl-analyzer`** is a secondary spec-strict linter (LSP only — CLI
  subcommands are stubs). See `MEMORY.md` §14 for verified gotchas.
- **Manual pass after risky changes**: pan, flick (inertia), zoom to
  min/max, tilt to clamp, play/pause + speed slider (watch Sun, stars, and
  satellite advance together), window resize, minimize/restore. Confirm idle
  (paused) renders **zero** frames.
- After atmosphere-constant or mapping changes, verify **both** the bake and
  shader sides and re-run — bit-identical output is the goal for neutral
  changes.
