# REFACTOR_PLAN.md - Single-frame headless render mode

Add a CLI **render mode** that draws one frame to an image file and exits,
without ever touching winit, input, or egui. The frame is positioned by an
explicit datetime (for the celestial positions) and explicit camera parameters,
and is written to a caller-specified path in a format a multimodal LLM agent can
view for debugging rendering issues.

This is a **plan only** - no code is written yet.

---

## 1. Decisions (confirmed with the owner)

- **No satellite markers in render mode.** The frame is pure globe + atmosphere
  + stars. Render mode is therefore fully independent of `scenarios` and needs
  no TLE/`Satellite`/`SimulationState`/`Clock` plumbing - only the
  ephemeris-driven celestial sphere + the camera.
- **All camera parameters are required CLI inputs - no defaults, no default
  camera position.** The user fully controls longitude, latitude, distance,
  and tilt every invocation (Section 2).
- **Camera distance is in kilometers** on the CLI (the internal world-space
  unit; no conversion).
- **Datetime is ISO-8601 / RFC3339 UTC**, e.g. `2024-01-15T12:30:00Z`, parsed
  with the **`humantime` crate** (`humantime::parse_rfc3339`) into a
  `SystemTime`, then converted to a satkit `Instant` via `from_unixtime`.
- **Output format: PNG, RGBA8.** Lossless (no compression artifacts to confuse a
  debugging agent), universally decodable, and interpreted as sRGB by image
  viewers and multimodal models - which is exactly what this renderer's
  non-sRGB pipeline needs (see Section 6).
- **No EOP range check in render mode (intentional).** Unlike scenarios, render
  mode does *not* validate the datetime against the bundled EOP range. It is a
  debug tool and the caller owns the time; out-of-range times silently degrade
  (satkit falls back to zeros below 1962 / constant-extrapolates past the last
  entry). This deviation from the scenario rules is deliberate and **must be
  documented** in the code and docs (Section 7).

---

## 2. CLI surface (`src/main.rs`)

Add a second top-level subcommand alongside `Scenario`. The `Cli` enum gains a
`Render` variant; `main` dispatches it to a new `snapshot::run`.

```
globe-experiment render \
    --datetime 2024-01-15T12:30:00Z \
    --longitude -75.0 \
    --latitude 40.0 \
    --distance 12742 \
    --tilt 0 \
    --width 1920 \
    --height 1080 \
    --output frame.png
```

Proposed `Render` variant fields (all `#[arg(long)]`). The four camera
parameters are **required** - there is no default camera position:

| flag          | type        | required? | meaning                                              |
|---------------|-------------|-----------|------------------------------------------------------|
| `--datetime`  | `String`    | required  | RFC3339 UTC instant for celestial positions          |
| `--longitude` | `f32`       | required  | inertial look longitude, deg (Camera.longitude)      |
| `--latitude`  | `f32`       | required  | inertial look latitude, deg (Camera.latitude)        |
| `--distance`  | `f32`       | required  | eye distance to look-at point, **km**                |
| `--tilt`      | `f32`       | required  | tilt off nadir, deg (Camera.tilt)                    |
| `--width`     | `u32`       | `1920`    | output width, px                                     |
| `--height`    | `u32`       | `1080`    | output height, px                                    |
| `--output`    | `PathBuf`   | required  | path to write the PNG                                |

Notes:
- The four camera params (`--longitude`, `--latitude`, `--distance`, `--tilt`)
  are required `clap` args with no `default_value` - the user controls the camera
  fully on every invocation. `Camera::default()` is **not** used by render mode.
- The same `distance` clamp the interactive camera applies
  (`Camera::clamp_distance`, ~0.01..10 Earth radii) should still be applied to
  the supplied value, with a runtime warning if the input was clamped, so a
  wildly out-of-range `--distance` degrades gracefully instead of producing a
  black/empty frame. (Clamping a user-supplied value is not a "default
  position" - it just bounds what is rendered.)
- `--width`/`--height` keep defaults (they size the output image, not the
  camera) and are bounded by the GPU's max 2D texture dimension
  (`wgpu::Limits::default().max_texture_dimension_2d` = 8192). Validate and
  error clearly above that.

Keep `main` tiny, as today: it parses and dispatches only. The bare-subcommand
listing behavior for `Scenario` is unchanged.

---

## 3. Current architecture - what is and isn't coupled to the window

Mapping the existing code so the refactor touches the minimum:

- **`renderer::GlobeRenderer` (private in `renderer/mod.rs`) is already
  window-/surface-agnostic.** It owns device-built scene resources (pipelines,
  mesh, textures, LUTs, uniforms, marker buffer) and exposes `new(device,
  queue, format)`, `prepare(device, queue, &RenderState, viewport)`, and
  `render(&mut RenderPass)`. Nothing in it references winit, the surface, or
  egui. **This is the shared core and needs no behavioral change.**
- **`renderer::Gfx` is the windowed target.** It owns the `Surface` (built from
  the `Arc<Window>`), the `SurfaceConfiguration`, the `GlobeRenderer`, and the
  `egui_wgpu::Renderer`. Its `update` presents to the surface and draws egui in
  the *same* render pass as the globe (the single-pass, no-depth invariant). The
  window coupling is: surface creation, `present()`, `pre_present_notify`,
  `FrameOutcome`, and resize.
- **`application::ApplicationState`** owns winit's window, the egui
  `Context`/`State`, the `Camera`, the input `Controller`, the
  `SimulationState`, and the `Gfx`. The per-frame `redraw` resolves the camera,
  advances the sim, runs egui, and calls `Gfx::update`. All winit/egui/input is
  here.
- **`application::camera::Camera` is pure glam math** (no winit). It produces
  `eye(celestial_to_world)` and `view_proj(aspect, celestial_to_world)` from
  lon/lat/distance/tilt. Only the input `Controller` (sibling module) is
  winit-coupled. Camera is currently private to `application`.
- **`simulation`** is fully decoupled (no winit/wgpu/egui). `CelestialSphere::at(time)`
  gives `sun_dir` + `star_rot_inv` + subsolar lat/lon; `RenderState`/
  `SatelliteMarker` are plain glam data. `simulation::init()` seeds satkit.
- **`Clock`/`SimulationState`** are about *advancing* time from a TLE epoch via
  wall-clock deltas. Render mode wants a *fixed* instant, so it can bypass both
  (Section 5) - it never needs the clock.

**Conclusion:** the only genuinely window-bound piece is `Gfx`'s surface +
present + egui. Everything render mode needs (the scene core, the camera math,
the celestial sphere) is already reusable. The refactor is therefore *additive*
plus three small extractions, not a rewrite.

---

## 4. Target architecture - the abstraction

Goal: windowed simulation and single-frame render share one scene core and one
device-creation path, and differ only in their **presentation target**
(swapchain surface vs. offscreen texture + readback) and whether egui/UI is
present.

### 4.1 Shared device/adapter creation (`renderer/mod.rs`)

Extract the instance/adapter/device/queue creation out of `Gfx::init` into a
small free function so both targets create the device identically (same
`TEXTURE_COMPRESSION_BC` requirement, same limits):

```
fn request_gpu(compatible_surface: Option<&wgpu::Surface>)
    -> (wgpu::Instance, wgpu::Adapter, wgpu::Device, wgpu::Queue)
```

- Windowed: `request_gpu(Some(&surface))` (adapter must be surface-compatible).
- Headless: `request_gpu(None)` (`compatible_surface: None`; everything else
  identical). BC7 is still required because the embedded earth/star textures are
  BC7 - this works under WSLg lavapipe, as the windowed path already proves.

`Gfx::init` is refactored to call this helper; its observable behavior is
unchanged.

### 4.2 The scene core stays `GlobeRenderer`

`GlobeRenderer` already *is* the target-agnostic core. The plan is to keep it as
the single shared scene type, owned by both targets, and just document it as the
shared core (and ensure its visibility allows a sibling `headless` submodule to
construct/drive it - `pub(crate)` within `renderer`). No logic changes.

Both targets follow the identical sequence the windowed path uses today:
`globe.prepare(...)` -> begin one render pass that clears to black ->
`globe.render(&mut pass)`. The windowed target additionally runs the egui
renderer inside that same pass; the headless target does not (no UI).

### 4.3 New headless target (`renderer/headless.rs`)

A new `HeadlessRenderer` that mirrors `Gfx` but renders to an owned offscreen
texture and reads it back to CPU instead of presenting:

```
pub struct HeadlessRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    globe: GlobeRenderer,
    color: wgpu::Texture,       // RENDER_ATTACHMENT | COPY_SRC, non-sRGB Rgba8Unorm
    readback: wgpu::Buffer,     // MAP_READ | COPY_DST, padded rows
    width: u32,
    height: u32,
}

impl HeadlessRenderer {
    pub fn new(width: u32, height: u32) -> Self;            // request_gpu(None) + GlobeRenderer::new(format)
    pub fn render(&mut self, render: &RenderState) -> image::RgbaImage; // one frame -> CPU pixels
}
```

`render`:
1. `globe.prepare(&device, &queue, render, (width, height))`.
2. Encode a single render pass into the offscreen `color` view: `LoadOp::Clear(BLACK)`,
   no depth, then `globe.render(&mut pass)` (stars -> surface -> atmosphere; no
   markers since `RenderState.markers` is empty, so the marker draw is skipped).
3. `copy_texture_to_buffer` into `readback` with `bytes_per_row` padded up to
   `wgpu::COPY_BYTES_PER_ROW_ALIGNMENT` (256). `rows_per_image` = height.
4. `queue.submit(...)`, then `device.poll(Wait)` and map the buffer
   (block via `pollster`, as the rest of the codebase does for async wgpu).
5. Un-pad the rows (drop the per-row padding) into a tight RGBA8 buffer and wrap
   it in an `image::RgbaImage`.

**Validated wgpu 29.0.3 API** (a throwaway probe compiled against the project's
own wgpu - see Section 14 - so these names are confirmed, not guessed):
- Copy: `encoder.copy_texture_to_buffer(TexelCopyTextureInfo{ texture, mip_level,
  origin: Origin3d::ZERO, aspect: TextureAspect::All }, TexelCopyBufferInfo{
  buffer, layout: TexelCopyBufferLayout{ offset, bytes_per_row: Some(padded),
  rows_per_image: Some(height) } }, Extent3d)`. (These are the v29 renames of the
  old `ImageCopy*`/`ImageDataLayout` types.)
- Readback: `buffer.slice(..).map_async(MapMode::Read, callback)`, then
  `device.poll(PollType::Wait{ submission_index: None, timeout: None })`, then
  `slice.get_mapped_range()`. `COPY_BYTES_PER_ROW_ALIGNMENT == 256`. Padding:
  `padded = (width*4).div_ceil(256) * 256`.
- The `RenderPassColorAttachment` carries `depth_slice: None` and the pass
  descriptor a `multiview_mask: None` (already used in `Gfx::update`).

No window, no surface, no `FrameOutcome`, no egui, no present, no resize. Done in
one shot.

**Offscreen format = non-sRGB `Rgba8Unorm`** (not the surface's typical
`Bgra8Unorm`): RGBA byte order means readback needs no B/R swap, and non-sRGB is
mandatory for color correctness (Section 6). `GlobeRenderer::new` is already
format-parameterized, so this just passes `Rgba8Unorm`.

### 4.4 Camera exposure (`application`)

Render mode needs the pure `Camera` math but **not** the input `Controller`. Per
the documented design ("the camera lives in `application`"), do **not** move
`Camera` out. Instead:

- Change `Camera` to `pub(crate)` and re-export it from `application`
  (`pub(crate) use camera::Camera;`), keeping `Controller` private.

This honors the "swapping the input scheme stays local to `application`"
rationale (the input controller stays put) while letting a second consumer read
the rig. The headless mode imports `crate::application::Camera`, constructs it
directly from the CLI fields, and calls `eye`/`view_proj` exactly as the
windowed `redraw` does.

> Alternative considered (not recommended): extract `Camera` into a new
> top-level `camera` module shared by `application` and `snapshot`. Cleaner in
> the abstract, but it contradicts the explicit CLAUDE.md decision that the
> camera lives in `application`, and would require rewording that rule. The
> `pub(crate)` exposure achieves the same reuse with far less churn. Flag to the
> owner only if they prefer the move.

### 4.5 New mode module (`src/snapshot.rs`, module `snapshot`)

A new top-level module - parallel to `scenarios` - that owns the render-mode
orchestration. Named `snapshot` to avoid confusion with the `renderer` module.
It is the headless analogue of a scenario's `run`:

```
pub struct RenderParams {       // built by main from the parsed CLI
    pub datetime: String,       // RFC3339
    pub longitude: f32,
    pub latitude: f32,
    pub distance_km: f32,
    pub tilt: f32,
    pub width: u32,
    pub height: u32,
    pub output: PathBuf,
}

pub fn run(params: RenderParams) { ... }   // no winit, no event loop
```

`run` does, in order (Section 5 details the frame build):
1. Parse the datetime (Section 7.1). No EOP range check (Section 7.2).
2. `simulation::init()` (seed satkit's ephemeris + EOP - same as a scenario).
3. Build the `RenderState` for the instant + camera (also yields the celestial
   sphere's subsolar lat/lon for the summary below).
4. `HeadlessRenderer::new(width, height).render(&render_state)`.
5. Save the `RgbaImage` to `--output` as PNG (`image::RgbaImage::save`), creating
   parent dirs as needed.
6. Print a concise text summary to stdout (see below).

No `ApplicationState`, no `application::run`, no `Clock`, no `SimulationState`.

### Stdout summary (agent feedback)

On success, print a short, stable, machine-/agent-readable summary so the agent
has textual context alongside the image it will open:

- the resolved UTC datetime (echo the parsed instant back, e.g. via
  `Instant::as_datetime` in the same format the UI's `datetime_label` uses);
- the subsolar geodetic lat/lon (deg), from the celestial sphere - tells the
  agent where the day side / terminator is in the frame;
- the camera params (longitude, latitude, distance km - report the *clamped*
  value if it was clamped - and tilt);
- the output path and pixel dimensions.

This is purely informational - it is **not** an EOP warning (render mode stays
fully silent about out-of-range times per Section 7.2). Keep it a few tidy
lines; the agent reads it together with the PNG.

---

## 5. Building the frame without the clock/simulation

Render mode needs a `RenderState`, which `SimulationState::frame_state` normally
produces. But that path is built around the running clock + tracked satellites,
and render mode has neither (fixed time, no markers). So build the `RenderState`
directly in `snapshot::run` from the pieces that are already public/pure:

```
let time = instant_from_rfc3339(&datetime)?;             // humantime parse -> Instant::from_unixtime (Section 7.1)
let cs   = CelestialSphere::at(&time);                   // simulation::celestial_sphere (pub)
let celestial_to_world = cs.star_rot_inv.transpose();    // same as SimulationState::celestial_to_world
let cam  = Camera { longitude, latitude, distance: clamp_distance(distance_km), tilt };
let aspect = width as f32 / height.max(1) as f32;
let eye  = cam.eye(celestial_to_world);
let render = RenderState {
    view_proj: cam.view_proj(aspect, celestial_to_world),
    camera_pos: eye,
    sun_dir: cs.sun_dir,
    star_rot_inv: cs.star_rot_inv,
    markers: Vec::new(),   // pure globe: no markers
};
```

This reuses `CelestialSphere`, `Camera`, and `RenderState` verbatim - identical
math to the windowed path - and avoids the `SimulationState::new` empty-satellite
panic and all clock plumbing. (`celestial_to_world` is recomputed inline rather
than via `SimulationState`, which is a one-liner transpose.)

> If markers are ever wanted in render mode later, the clean extension is a
> `SimulationState::at(instant, satellites)` that builds state pinned to a fixed
> time (a paused clock at `instant`) and calls `frame_state`. Out of scope now.

---

## 6. Color / image correctness (the subtle part)

The windowed surface is deliberately **non-sRGB** (`Gfx::init` picks
`!f.is_srgb()`), and *every look-tuning constant in `globe.wgsl` is calibrated to
that non-sRGB target* (a golden rule). On a non-sRGB surface the shader's stored
8-bit output is written raw, and the display applies the sRGB EOTF on read - so
the stored byte values **are** the sRGB-encoded pixels the viewer sees.

Therefore, to make the saved PNG look like the on-screen image:

- **The offscreen texture must also be non-sRGB** (`Rgba8Unorm`, not
  `Rgba8UnormSrgb`). Using an sRGB target here would hardware-encode the output
  and render visibly different from the window - re-introducing exactly the bug
  the surface-format rule guards against.
- **Write the read-back bytes verbatim into the PNG.** A PNG is interpreted as
  sRGB by viewers and multimodal models; since the stored non-sRGB bytes already
  equal the sRGB-encoded on-screen pixels, a direct copy reproduces the on-screen
  look with no conversion. Do **not** apply any gamma/sRGB transform on readback.

This is the headless mirror of the existing "surface must be non-sRGB" golden
rule and should be documented as such (CLAUDE.md + MEMORY.md).

Output is LDR straight from the pipeline (no HDR/bloom - unchanged). Alpha is
whatever the pipeline wrote; save as RGBA8 (the globe fully covers nothing - the
black-cleared background is opaque black, fine for an opaque PNG; if alpha looks
wrong in practice, force alpha to 255 on readback).

---

## 7. Datetime parsing - and the deliberate absence of an EOP range check

### 7.1 Parsing (humantime)

In `snapshot::run`:

1. `humantime::parse_rfc3339(&params.datetime)` -> `SystemTime` (errors on a
   malformed timestamp with a clear message; the accepted form is RFC3339, e.g.
   `2024-01-15T12:30:00Z`).
2. `SystemTime::duration_since(UNIX_EPOCH)` -> seconds as `f64` ->
   `satkit::Instant::from_unixtime(secs)`. (A pre-1970 `SystemTime` yields an
   `Err` from `duration_since`; surface it as a clean parse error rather than a
   panic.)

`humantime` only parses UTC RFC3339, which matches the "datetime is UTC"
decision; no timezone handling is needed.

**Validated (Section 14):** `Instant::from_unixtime` is leap-second-correct - its
source explicitly *adds* leap seconds back (Unix time omits them) and re-checks
the boundary, so a `humantime` Unix instant lands on the right UTC instant.
`from_unixtime` (and `from_datetime`/`as_datetime`) all exist in satkit 0.18.1.
(Aside: satkit also has `Instant::from_rfc3339`, but the owner chose `humantime`,
so we keep the `humantime` -> `from_unixtime` path.)

### 7.2 No EOP range check (intentional - and documented)

Scenarios are required to keep their time window inside the bundled EOP range
(CLAUDE.md "Scenarios & valid time range"). **Render mode deliberately does not
enforce this.** Per the owner's decision, the datetime is *not* validated against
the EOP table: render mode is a debugging entry point where the caller owns the
time and may intentionally probe degraded/edge cases.

Consequences the caller accepts (and that the docs must spell out):

- **Before 1962-01-01**: satkit finds no EOP and silently falls back to zeros,
  degrading the Sun/star backdrop to roughly `*_approx`-without-EOP accuracy.
- **After the last bundled EOP row**: satkit constant-extrapolates the final
  row's values - also a silent degradation.

Neither is an error in render mode; the frame still renders. This is a conscious
deviation from the scenario accuracy rules, so it **must be documented** as such:

- A doc-comment on `snapshot::run` (and/or the `Render` CLI help) stating that
  the datetime is not EOP-range-checked and why.
- A note in `CLAUDE.md` (under render mode) and `MEMORY.md` recording that render
  mode intentionally skips the EOP range check that scenarios enforce, with the
  silent-degradation consequence above.

(The celestial-sphere path still uses the `*_approx` transforms as everywhere
else; the only difference in render mode is that the time is unchecked.)

---

## 8. Dependencies

- **Promote `image` to a runtime dependency** for PNG encoding. It is currently
  a *build*-dependency only (`jpeg`, `tiff` features for `build.rs`). Add a
  `[dependencies]` entry with `default-features = false, features = ["png"]`.
  The build-dep entry stays as-is (separate table, separate feature set).
  *Validated:* cached `image 0.25.10` exposes `png = ["dep:png"]` and
  `RgbaImage = ImageBuffer<Rgba<u8>, Vec<u8>>`, so `RgbaImage::from_raw(w, h,
  Vec<u8>)` + `.save(path)` works under the `png` feature.
- **Add `humantime`** (`humantime = "2"`) for RFC3339 datetime parsing.
  *(crates.io reachable; `humantime::parse_rfc3339(&str) -> Result<SystemTime,
  _>` is its stable API.)*
- `pollster` is already a dep (used for blocking on the buffer map).

---

## 9. File-by-file change list

| File                          | Change                                                                                          |
|-------------------------------|-------------------------------------------------------------------------------------------------|
| `src/main.rs`                 | Add `Render` subcommand variant + fields; dispatch to `snapshot::run`. Add `mod snapshot;`.      |
| `src/snapshot.rs` (new)       | `RenderParams` + `run`: parse datetime (humantime), build `RenderState`, drive `HeadlessRenderer`, save PNG. Doc-comment notes the deliberate lack of an EOP range check. |
| `src/renderer/mod.rs`         | Extract `request_gpu(Option<&Surface>)`; have `Gfx::init` use it; `pub(crate)` on `GlobeRenderer`; `mod headless; pub use headless::HeadlessRenderer;`. |
| `src/renderer/headless.rs` (new) | `HeadlessRenderer`: offscreen texture + readback, single-pass globe render, returns `image::RgbaImage`. |
| `src/application/mod.rs`      | `pub(crate) use camera::Camera;` (re-export); `Camera` -> `pub(crate)`.                          |
| `src/application/camera.rs`   | Visibility bump to `pub(crate)` (struct + the `eye`/`view_proj`/`clamp_distance`/`pan`... as needed by snapshot). |
| `Cargo.toml`                  | Add `image` to `[dependencies]` (`default-features=false, features=["png"]`); add `humantime = "2"`. |
| `CLAUDE.md`                   | Document render mode, the headless non-sRGB rule, the camera `pub(crate)` exposure, the `image`/`humantime` deps, required camera params, and the deliberate omission of the EOP range check. |
| `MEMORY.md`                   | Add a render-mode subsystem note (offscreen target, readback alignment, color-correctness mirror of the surface rule, and the unchecked-time deviation). |
| `README.md`                   | Add the `render` usage example.                                                                 |
| `.claude/skills/analyze-render/SKILL.md` (new) | Agent skill: render a frame via `render` mode and inspect the PNG for visual feedback; documents the lack of EOP time-bound checking (Section 11). |

(Keep docs current *in the same change* per the documentation golden rule.)

---

## 10. Testing & verification

> **Environment caveat (validated - Section 14):** this dev/sandbox has **no
> GPU stack** (no `/dev/dri`, no Vulkan/GL ICDs), so *any* rendering - windowed
> or headless - cannot run here; a surfaceless `request_adapter` fails with
> `active_backends: 0x0` purely for lack of a driver. The runtime checks below
> must be done where the windowed app already runs (native Windows / a real-GPU
> WSLg host). Everything that does **not** need a live GPU (compile, clippy,
> CLI parsing/validation, the `image`/`humantime`/satkit API usage) is
> verifiable anywhere.

- **Build + clippy**: `cargo run --release` and `cargo clippy` warning-free
  (per CLAUDE.md). No shader change, so no naga/wgslfmt run is needed - confirm
  `globe.wgsl` is untouched.
- **Smoke**: `globe-experiment render --datetime 2024-01-01T00:00:00Z
  --longitude 0 --latitude 0 --distance 12742 --tilt 0 --output /tmp/a.png` and
  confirm a non-empty, decodable PNG of the requested size is written and the
  process exits 0 with no winit window appearing. (All camera flags are
  required, so a smoke command must pass them.)
- **Visual sanity vs. windowed**: pick a datetime + camera, render headless, then
  (separately) eyeball the windowed app at a matching configuration; the globe
  framing/lighting/terminator should match (modulo the missing UI/markers). This
  is the practical check that the non-sRGB color path is correct.
- **Determinism**: same args -> byte-identical PNG (no clock, no animation, fixed
  ephemeris) - a useful regression signal for an agent debugging the renderer.
- **Stdout summary**: confirm the run prints the resolved datetime, subsolar
  lat/lon, camera params, and output path, and that it stays silent about EOP
  range even for an out-of-range time.
- **Input validation**: a malformed datetime errors via humantime; a missing
  required camera flag errors via clap; an oversize `--width`/`--height` errors.
  An out-of-EOP-range datetime still renders (no range check) - optionally spot
  check that a pre-1962 time produces a frame without panicking.
- **No stray dir**: run from a clean directory and confirm no `satkit-data` dir
  appears (the `init_satkit` EOP seed still applies - render mode calls
  `simulation::init()`).

---

## 11. New agent skill: render analysis (visual feedback loop)

Render mode exists largely so an **agent can see the output**: the WSLg dev
environment can't judge look/color interactively, but snapshot mode writes a PNG
the agent can open with its multimodal vision. Ship a project skill that
codifies this loop so an agent reaches for it after rendering changes.

- **Location:** `.claude/skills/analyze-render/SKILL.md` (same layout as the
  existing skills: YAML frontmatter + body).
- **Purpose:** render one frame headless via the `render` CLI mode, then inspect
  the resulting PNG (with the `Read` tool) to get concrete visual feedback on a
  rendering change - lighting, terminator, atmosphere limb, city lights, ocean
  glint, star backdrop, framing. This is the headless analogue of `build-and-run`
  but it produces an artifact the agent can actually look at here, no window
  needed.
- **When to use:** after editing `shaders/globe.wgsl`, the atmosphere
  constants/LUTs, the renderer, or anything that changes the picture - to check
  the actual output, and to do before/after comparisons (render to two paths and
  compare).

### Skill content (sketch)

```
---
name: analyze-render
description: Render a single frame headless via the `render` CLI mode and inspect the PNG to get visual feedback on rendering changes (lighting, terminator, atmosphere, framing). Use after shader/atmosphere/renderer edits to see the actual output.
---

# Analyze a render (visual feedback loop)

Render one frame to a PNG with `render` mode, then open the PNG to judge the
look. Pick a datetime + camera that frames the feature you changed.

## Command
cargo run --release -- render \
    --datetime 2024-01-01T12:00:00Z \
    --longitude <deg> --latitude <deg> --distance <km> --tilt <deg> \
    --width 1280 --height 720 \
    --output /tmp/render.png

Then open /tmp/render.png with the Read tool and describe/compare what you see.
The command also prints a summary (resolved datetime, subsolar lat/lon, camera,
output path) - read it for context on where the day side / terminator should be.

## Framing tips
- Terminator / day-night edge: aim the camera at a longitude near the subsolar
  +/-90 deg; a low tilt shows the edge across the disc.
- Atmosphere limb / sky glow: high tilt (toward the horizon) so the limb fills
  the frame.
- Night side (city lights): a longitude on the dark hemisphere.
- Before/after: render the change to two files and inspect both.

## IMPORTANT: no EOP time-bound checking in render mode
Render mode does NOT validate the datetime against the bundled EOP range
(unlike scenarios). Out-of-range times silently degrade and will mislead a
visual analysis:
- before ~1962-01-01: satkit falls back to zero EOP (Sun/stars drift);
- after the last bundled EOP entry (~build date): satkit constant-extrapolates.
Choose an in-range PAST datetime (1962-01-01 .. build date) for an accurate
frame. This is the agent's responsibility - the tool will not warn you.

## What it does / does not validate
- DOES: the rendered look/color/lighting (the PNG reproduces the on-screen,
  non-sRGB LDR output; see the renderer color note).
- Does NOT: interaction feel (pan/zoom/inertia) - that still needs a native
  windowed run. Markers/UI are absent in render mode by design.
```

The skill's `description` and the "no EOP time-bound checking" section are
load-bearing: the description is how an agent finds the skill, and the EOP note
is the one correctness footgun unique to render mode (Section 7.2). Keep both in
sync with the code when render mode changes.

---

## 12. Non-goals / out of scope

- No depth buffer, HDR, or bloom (unchanged invariants).
- No markers/UI/egui in render mode (by decision).
- No animation/video output (single frame only).
- No new scenario; render mode is a peer mode, not a scenario.
- No change to the windowed app's observable behavior - the `Gfx` refactor is a
  pure extraction.
- Not moving `Camera` out of `application` (using `pub(crate)` exposure instead).
- **No EOP range check / time validation in render mode** (intentional; the
  scenario range rules do not apply here). Documented per Section 7.2.
- No default camera position; all four camera params are required.

---

## 13. Resolved decisions

All resolved by the owner:
- **PNG bit depth**: RGBA8.
- **Datetime parsing**: `humantime` crate.
- **Camera parameters**: all four (`--longitude`/`--latitude`/`--distance`/
  `--tilt`) are required; no defaults, no default camera position.
- **EOP range check**: **none** in render mode, and **no runtime EOP logic at
  all** (not even a non-fatal warning). The datetime is not validated against the
  EOP range; out-of-range times silently degrade. This deliberate deviation from
  the scenario rules is documented in code + `CLAUDE.md` + `MEMORY.md` + the
  `analyze-render` skill (Section 7.2 / 11).
- **Stdout summary**: on success, `render` mode prints a concise text summary
  (resolved datetime, subsolar lat/lon, camera params, output path) for agent
  context - purely informational, never an EOP warning (Section 4.5).
- **Agent skill**: ship `.claude/skills/analyze-render/SKILL.md` (Section 11).

No open questions remain.

---

## 14. Pre-implementation validation results (2026-06-20)

Checks run against the real toolchain/crates before writing feature code. A
throwaway wgpu probe (an `examples/` binary reusing the project's own
`wgpu 29.0.3`, since removed) plus source/registry inspection.

**Confirmed - safe to build against:**
- **wgpu 29.0.3 headless API compiles**: the probe (surfaceless adapter ->
  device with `TEXTURE_COMPRESSION_BC` -> offscreen `Rgba8Unorm` render ->
  padded `copy_texture_to_buffer` -> `map_async`/`poll(PollType::Wait)` ->
  `get_mapped_range`) built cleanly. Exact v29 type names captured in Section 4.3
  (`TexelCopyTextureInfo`/`TexelCopyBufferInfo`/`TexelCopyBufferLayout`,
  `PollType::Wait{ submission_index, timeout }`, `MapMode::Read`,
  `COPY_BYTES_PER_ROW_ALIGNMENT == 256`).
- **satkit time API**: `from_unixtime` exists and is **leap-second-correct**
  (adds leap seconds back since Unix time omits them); `from_datetime` /
  `as_datetime` exist. So `humantime` -> `from_unixtime` is sound (Section 7.1).
- **image 0.25.10**: `png` feature (`png = ["dep:png"]`) and `RgbaImage` /
  `from_raw` / `save` are present (Section 8).
- **humantime**: crates.io reachable for the add; `parse_rfc3339` is its stable
  entry point (Section 8).

**Could NOT be validated here (no fault of the design):**
- **Any live-GPU behavior.** This sandbox has **no GPU stack**: no `/dev/dri`,
  no Vulkan/GL ICDs, no lavapipe. The surfaceless `request_adapter` fails with
  `NotFound { active_backends: 0x0, supported_backends: VULKAN|GL,
  incompatible_surface_backends: 0x0 }` - i.e. **no driver to bring up a
  backend**, *not* a surface-compatibility problem. The same wall blocks the
  windowed app here; CLAUDE.md already states rendering is only validatable on a
  native Windows / real-GPU host.
  - **Implication for the plan:** `compatible_surface: None` is **not**
    implicated by this failure (it would surface in `incompatible_surface_backends`,
    which is `0x0`). On any host where the windowed app runs, a surfaceless
    adapter request and the offscreen readback are expected to work - that is
    where the actual headless render must be smoke-tested (Section 10 caveat).
  - **Residual risk to verify on a real GPU host:** that the chosen offscreen
    format (`Rgba8Unorm`) is renderable+`COPY_SRC` (it is a core format, so
    expected fine) and that the produced PNG matches the windowed look (the
    non-sRGB color reasoning in Section 6).
