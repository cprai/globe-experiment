# PHASE2 — Project state and technical context

Snapshot at the end of phase 2 (2026-06-12). This is the canonical
context document: everything an agent or developer needs to make changes
without re-deriving the design. It supersedes `claude/phase1/PHASE1.md`
as the current-state reference — PHASE1 remains accurate for the
rendering math and conventions (unchanged) but its iced integration and
build/startup sections are now historical. Companion docs:
`claude/phase2/PHASE2_PLAN.md` (the migration plan + a detailed dated
status log of every change and bug in phase 2), `claude/phase1/PLAN.md`
(original v1 milestones), `claude/phase1/OPTIMIZE.md` (startup-perf
ideas; ideas 2 and 3b are now implemented).

**What phase 2 was.** Phase 1 shipped the globe embedded in an
**iced 0.14** app via the `iced::widget::shader` widget. Phase 2 deleted
iced entirely and rebuilt the host on a self-owned stack — **winit**
window/event loop, **raw wgpu** surface and render loop, mouse controls
on winit events, and the two sun sliders in **egui**. It then added three
optimizations: build-time BC7/KTX2 texture transcoding, a build-time
atmosphere LUT bake, and a hidden-until-ready window. The rendering
design (shaders, passes, atmosphere model, coordinate conventions) did
**not** change; the globe is pixel-identical to phase 1.

## What the project is

An interactive Google-Earth-style 3D globe viewer. Rust, edition 2024.
Windowing/input via **winit 0.30**, all 3D rendering via **wgpu 29**
(now a direct dependency), and the control overlay via **egui 0.34**
(`egui-wgpu` + `egui-winit`). Features: day/night Earth with city
lights, normal-mapped terrain, GGX ocean sun glint, Hillaire-2020-based
atmospheric scattering (precomputed LUTs), star/sun backdrop, orbital
camera with pan/tilt/zoom + flick inertia + smoothed zoom, and two UI
sliders controlling the sun position.

The package is still named `iced-test-app` for historical reasons; iced
is no longer a dependency.

## Dependencies (Cargo.toml)

Runtime:
- `wgpu = "29"` — GPU. **Now a direct dependency** (phase 1 used iced's
  `iced::wgpu` re-export of wgpu 27). Version forced by egui-wgpu 0.34,
  which requires `wgpu ^29.0.1`.
- `winit = "0.30"` — window + event loop. Required `^0.30.13` by
  egui-winit 0.34.
- `egui = "0.34.3"`, `egui-wgpu = "0.34.3"`, `egui-winit = "0.34.3"` —
  the sun-slider overlay and its wgpu paint + winit input bridges.
- `pollster = "0.4"` — blocks on the async wgpu adapter/device requests
  at startup (no async runtime otherwise).
- `glam = "0.33.1"` — camera/sun math (Mat3/Mat4/Quat/Vec3).
- `bytemuck = { version = "1.25", features = ["derive"] }` — Pod casts
  for vertex/uniform data.
- `ktx2 = "0.5"` — parses the build-script-produced KTX2 texture
  containers at runtime.

**No longer runtime dependencies** (moved or removed):
- `iced` — **removed**.
- `image` — moved to build-dependencies (runtime no longer decodes
  anything; textures arrive pre-transcoded).
- `half` — moved to build-dependencies (only the LUT bake, now at build
  time, produces f16; the runtime just uploads bytes).

Build-dependencies:
- `ureq = "3.3"` — asset download.
- `image = { version = "0.25", default-features = false, features =
  ["jpeg", "tiff"] }` — decodes the downloaded textures for transcoding.
  (png feature dropped; all five assets are jpeg/tiff.)
- `intel_tex_2 = "0.5"` — ISPC BC7 encoder for the texture transcode.
- `ktx2 = "0.5"` — writes the KTX2 containers (same crate as runtime, so
  writer and reader can't drift).
- `half = { version = "2.7", features = ["bytemuck"] }`,
  `bytemuck = "1.25"` — the atmosphere LUT bake (`src/globe/atmosphere.rs`
  pulled into the build script via `#[path]`) and its f16 conversion.
  **Update (2026-06-13):** the bake source was inlined directly into
  `build.rs` (as an in-file `mod atmosphere`); `src/globe/atmosphere.rs`
  no longer exists and the `#[path]` include is gone. The build-deps are
  unchanged.

Profile: `[profile.dev.package.X] opt-level = 3` for `image`,
`zune-jpeg`, `zune-core`, `tiff`, `miniz_oxide`, `weezl` — these now
speed up the **build-script** transcode decode (5×33 MP). They no longer
affect runtime (no runtime decode).

`.cargo/config.toml` (new): adds `rustflags = ["-C",
"link-arg=-lstdc++"]` for `x86_64-unknown-linux-gnu` only —
intel_tex_2's prebuilt ISPC objects reference the GCC C++ exception
personality and Rust doesn't link libstdc++ by default on Linux. MSVC
on Windows resolves its C++ runtime automatically and is unaffected.

## File map

```
build.rs                 downloads 5 textures, BC7→KTX2 transcodes them,
                         AND bakes the atmosphere LUTs to KTX2, all into
                         OUT_DIR; pulls in atmosphere.rs via #[path]
                         [2026-06-13: atmosphere.rs was inlined into
                         build.rs as `mod atmosphere`; no #[path] anymore]
.cargo/config.toml       Linux-only -lstdc++ for intel_tex_2
src/main.rs              winit ApplicationHandler: App + Gfx, event loop,
                         wgpu surface/device, frame loop, egui integration
src/ui.rs                egui sun panel (two sliders + labels)
src/globe/mod.rs         module declarations only (camera/input/mesh/
                         renderer/sun) — no logic, no atmosphere
src/globe/renderer.rs    GlobeRenderer (was pipeline.rs): all wgpu objects
src/globe/input.rs       Controller (was mod.rs logic): drag/tilt/wheel,
                         pan flick inertia, smoothed zoom
src/globe/camera.rs      orbital camera
src/globe/sun.rs         sun position + star map orientation
src/globe/mesh.rs        UV-sphere generator
src/globe/atmosphere.rs  CPU bake of the atmosphere LUTs — compiled into
                         build.rs, NOT into the runtime crate
                         [2026-06-13: REMOVED — bake now lives inline in
                         build.rs as `mod atmosphere`; this file is gone]
shaders/globe.wgsl       ALL shader code (3 passes in one module)
assets/                  gitignored; downloaded textures (build input)
OUT_DIR/*.ktx2           gitignored build artifacts, include_bytes!'d:
                         5 BC7 textures + 3 f16 LUTs
claude/phase1/           phase-1 docs (PHASE1/PLAN/OPTIMIZE)
claude/phase2/           PHASE2 (this file) + PHASE2_PLAN
```

## Application shell (src/main.rs) — replaces the iced integration

The phase-1 "iced integration" section is fully obsolete. The host is
now a winit `ApplicationHandler`.

- `main()` builds an `EventLoop`, sets `ControlFlow::Wait`, and runs
  `App::default()`. **`ControlFlow::Wait` is load-bearing**: idle =
  zero GPU work, the phase-1 invariant. Frames are driven only by
  explicit `window.request_redraw()`.
- `App { camera: Camera, sun: Sun, controller: Controller, gfx:
  Option<Gfx> }`. Camera and sun state live here (as in phase 1's app),
  but there is **no message enum / no indirection** — input mutates the
  camera directly. The old `Interaction { Pan, Zoom, Tilt }` enum is
  deleted.
- `Gfx` holds everything tied to the window/GPU, created once in
  `resumed()`: `window: Arc<Window>`, `surface`, `device`, `queue`,
  `config`, `globe: GlobeRenderer`, `egui_ctx`, `egui_state:
  egui_winit::State`, `egui_renderer: egui_wgpu::Renderer`, and a
  `shown: bool` (hidden-until-ready flag).

### `Gfx::new` — device + surface setup (the part that's easy to get wrong)

1. `Instance::new(InstanceDescriptor::new_without_display_handle())`
   (wgpu 29: descriptor passed by value, no `Default`).
2. Surface from `window.clone()`, then `request_adapter`
   (`HighPerformance`) and `request_device`. **The device requests
   `Features::TEXTURE_COMPRESSION_BC`** — required for the BC7 textures.
   This is the freedom the iced removal bought: phase-1 iced created the
   device with `Features::empty()` and exposed no way to request
   features, which blocked all compressed/feature-gated formats.
   `experimental_features: ExperimentalFeatures::disabled()` (new wgpu
   29 field).
3. `get_default_config(...)`, then **two explicit overrides** — both
   are bugs waiting to happen via `get_default_config`'s defaults:
   - **Surface format: pick a non-sRGB format** (`find(|f|
     !f.is_srgb())`). The shader writes linear color; storing it raw on
     a non-sRGB surface and letting the display read it as sRGB is what
     iced's default `web-colors` feature did in phase 1, and **every
     look-tuning constant in globe.wgsl is calibrated to that darker
     rendition**. An sRGB surface (hardware encode) renders visibly
     brighter; moving to one would require re-tuning the whole shader.
   - **Present mode: `PresentMode::AutoVsync`**. `get_default_config`
     takes `present_modes.first()`, which is **Mailbox on DX12** —
     unpaced rendering that makes scroll-zoom and inertia judder (frames
     follow the bursty input cadence; the inertia loop free-runs with
     jittery dt). AutoVsync paces the animation loop to the refresh
     rate. iced used AutoVsync.
4. `GlobeRenderer::new(&device, &queue, config.format)` — eager, before
   the first frame (phase 1 did this lazily on first draw inside iced's
   `Pipeline::new`). After the BC7/LUT work this is fast: upload +
   pipeline creation, **no decode, no bake** at runtime.
5. egui: `egui::Context::default()`, `egui_winit::State::new(...)`,
   `egui_wgpu::Renderer::new(&device, config.format,
   RendererOptions::default())` (0.34 takes a `RendererOptions` struct,
   not positional args).

### Event routing (`window_event` → `handle_input`)

Replaces iced's `stack![]` overlay event capture. Every `WindowEvent`
except `CloseRequested`/`Resized`/`RedrawRequested` goes to
`handle_input`, which:
1. Feeds the event to `egui_state.on_window_event(&window, &event)`
   **first**. If `response.repaint`, request a redraw. If
   `response.consumed` (pointer over the panel, slider drag), return —
   the globe controller never sees it.
2. Otherwise `controller.handle_event(&event, &mut camera,
   config.height)`; if it returns true (camera changed or an animation
   started), request a redraw.

`Resized` reconfigures the surface and requests a redraw.
`CloseRequested` exits.

### Frame (`redraw`)

Driven by `RedrawRequested` (and called directly once at startup — see
below). Per frame:
1. `controller.tick(&mut camera, config.height)` advances flick inertia
   **and** the zoom glide; returns `animating` (whether another frame is
   needed).
2. egui: `take_egui_input` → `run_ui(|ui| ui::sun_panel(ui.ctx(), &mut
   sun))` (0.34 deprecated `Context::run` for `run_ui`, whose closure
   gets a transparent fullscreen `&mut Ui`; the Area panel hangs off
   `ui.ctx()`) → `handle_platform_output`.
3. Reassert the grab cursor unless `egui_ctx.is_pointer_over_egui()`
   (egui resets the cursor every frame; `is_pointer_over_area` was
   renamed `is_pointer_over_egui` in 0.34).
4. `get_current_texture()` returns the wgpu-29 `CurrentSurfaceTexture`
   enum (not `Result`): `Success`/`Suboptimal` carry the frame;
   `Lost`/`Outdated` → reconfigure + redraw + return; `Timeout` →
   redraw + return; `Occluded` → (hidden-window first-frame guard, see
   below); `Validation` → panic.
5. `globe.prepare(queue, camera, sun, aspect)` writes uniforms.
6. egui tessellate → `update_texture` for each `textures_delta.set`
   **before** the pass → `update_buffers` (returns prep command buffers)
   → one render pass (clear BLACK → `globe.render` → `egui_renderer.
   render`) → submit egui commands chained with the frame encoder →
   `pre_present_notify` → `present` → `free_texture` for each
   `textures_delta.free` **after** submit.
7. Reveal the window on the first present (hidden-until-ready).
8. Request another redraw iff `animating` or egui reported a zero
   `repaint_delay`.

The single render pass needs a `RenderPass<'static>` for egui-wgpu;
`render_pass.forget_lifetime()` is the documented pattern (the pass
still ends at scope close, before the encoder finishes).
`RenderPassDescriptor` gained `multiview_mask: None` and the color
attachment gained `depth_slice: None` in wgpu 29.

### Hidden-until-ready window (startup perception)

The window is created `with_visible(false)` and revealed via
`set_visible(true)` immediately after the first `frame.present()`, so it
appears with the globe already rendered rather than blank during
load. Two non-obvious requirements, both found by the owner on Windows:

- **`resumed()` calls `self.redraw()` directly** for the first frame,
  **not** `request_redraw()`. Windows only generates paint messages
  (`RedrawRequested`) for *visible* windows, so requesting a redraw on a
  hidden window never delivers and the reveal code (inside `redraw`)
  would never run — the window would stay invisible forever. Presenting
  to a hidden window is fine; only paint-event *delivery* needs
  visibility. **Do not "simplify" this back to `request_redraw()`.**
- **`Occluded` first-frame guard**: some backends report a still-hidden
  window as occluded from `get_current_texture`. If that happens before
  `shown`, show the window and retry rather than deadlocking invisible.

### Redraw policy summary

`ControlFlow::Wait` + targeted `request_redraw()` on: camera change
from input, active flick inertia or zoom glide (each frame requests the
next until it settles), egui zero repaint delay (slider drag), resize,
and surface lost/timeout recovery. Idle requests nothing → zero frames.
This preserves phase 1's idle-is-free property while giving animation a
real frame loop (future sun animation just becomes another "animating"
flag).

## Input (src/globe/input.rs) — `Controller`

Was phase-1 `globe/mod.rs`. Translates winit mouse events into camera
motion, mutating `&mut Camera` directly (returns `bool` = needs redraw).
State: `cursor`, `drag: Option<Drag>`, `inertia: Option<Inertia>`,
`zoom: Option<Zoom>`, plus wheel-cadence tracking (`last_wheel`,
`wheel_gap`).

- **Left drag pans** (globe follows cursor: `dlon = −dx·scale`, `dlat =
  +dy·scale`, scale = `camera.pan_degrees_per_pixel`).
- **Right drag tilts** (`−dy·0.25°/px`).
- **Cursor velocity EMA**: `alpha = 1−e^(−20·dt)` (frame-rate
  independent), same as phase 1.
- **Flick inertia** (unchanged from phase 1, just on winit events): on
  left release, if `speed > FLICK_SPEED` (50 px/s) and last move <
  `FLICK_TIMEOUT` (0.1 s) ago, coast. `tick_coast` integrates with real
  `Instant`-based dt (capped 0.1 s), decays with `HALF_LIFE` 0.3 s,
  stops below `STOP_SPEED` 15 px/s. Pressing any button cancels inertia.
- **Cursor icon**: `CursorIcon::Grab`/`Grabbing` (set on the window).

### Smoothed zoom (the heavily-iterated part — see PHASE2_PLAN status)

Phase 1 applied each wheel event directly (`distance *= 0.9^ticks`).
That stepped visibly on a laptop trackpad, whose scroll + OS-synthesized
momentum tail arrive as sparse quantized events. The current design,
arrived at over ~5 owner-tested iterations (including one full
remove-then-revert), is a **rate-adaptive glide with velocity
bridging**. Wheel events never move the camera directly — they move a
target distance; `tick_zoom` eases the camera toward it each frame.

- Work in **log distance** (`delta = ticks·ln(0.9)`); zoom is
  multiplicative so log space keeps perceived speed uniform at any
  altitude. Target clamped via `Camera::clamp_distance`.
- **Adaptive half-life**: `wheel_gap` is an EMA of the inter-event gap
  (capped at `WHEEL_GAP_CAP` 0.25 s); the glide half-life =
  `wheel_gap.clamp(ZOOM_HALF_LIFE_MIN 0.01, ZOOM_HALF_LIFE_MAX 0.1)`.
  Dense events (~10 ms, active scroll) → near-instant; sparse events
  (momentum tail, single notches) → interpolate across the gap that
  would otherwise step. An in-flight glide keeps its `tick` (resetting
  it per event stalls the glide between dense events).
- **Velocity bridging** (fixes a stall at finger-lift, where a
  ~10 ms half-life drained the target before the momentum tail's first
  event): `Zoom.velocity` is an EMA of the rate events move the target
  (log-dist/s). `tick_zoom` keeps advancing the target at that velocity
  (decaying with `ZOOM_COAST_HALF_LIFE` 0.15 s, stopping below
  `ZOOM_STOP_RATE` 0.1/s), logging the advance in `Zoom.bridged`. The
  next wheel event **repays** `bridged` — only the un-bridged remainder
  moves the target, surpluses carry forward — so while events flow the
  total zoom equals exactly what the device sent; velocity only fills
  delivery gaps. Reversing scroll direction zeroes the coast; a first
  event (no rate info) starts with zero velocity, so single mouse
  notches don't coast.
- Feel knobs: `ZOOM_HALF_LIFE_MIN/MAX` (glide response),
  `ZOOM_COAST_HALF_LIFE` (coast length), `ZOOM_STOP_RATE` (coast end).
  **Tune via these constants; don't restructure** (the design space was
  explored exhaustively — fixed-half-life glide and fixed burst-gap
  split were both tried and rejected; see PHASE2_PLAN).

## UI (src/ui.rs) — egui sun panel

Replaces phase 1's iced `stack![]`-overlaid `container(column![...])`.
`sun_panel(ctx, &mut Sun)` draws an `egui::Area` anchored top-left
([10,10], width 260): a white "Sun latitude" label + slider
−23.44..=23.44 step 0.1 (= season / solar declination), and a "Sun
longitude" label + slider −180..=180 step 0.5 (= time of day). Sliders
mutate `&mut Sun` directly (no message round-trip). `show_value(false)`
— the value is in the label. egui claims its own events first (see
routing above), so dragging a slider never pans the globe.

## Build system (build.rs) — download + transcode + bake

Three jobs, all writing into `OUT_DIR`, which the runtime `include_
bytes!`-es. `assets/` holds only the downloaded source images
(gitignored build input).

### 1. Download (unchanged behavior)

`ASSETS`: five solarsystemscope.com textures (8k_earth_daymap.jpg,
8k_earth_nightmap.jpg, 8k_earth_normal_map.tif, 8k_earth_specular_map.tif,
8k_stars_milky_way.jpg; all 8192×4096), each tagged `srgb: bool`.
`download_if_missing` derives the filename, emits
`cargo::rerun-if-changed=assets/<name>`, skips if present, else downloads
via ureq (100 MB cap). Deleting a file re-triggers its download.

### 2. BC7 → KTX2 transcode (OPTIMIZE idea 3b)

`transcode_if_missing` decodes each source (`image`), asserts
multiple-of-4 dimensions (BC7 block size), BC7-compresses with
`intel_tex_2::bc7::compress_blocks(opaque_basic_settings(), surface)`,
and writes `<stem>.ktx2` to `OUT_DIR`. `srgb` assets →
`BC7_SRGB_BLOCK` (color maps: day/night/stars), data assets →
`BC7_UNORM_BLOCK` (normal/specular — must stay linear). **Cached on
existence** (the encode is slow; sources never change). Delete the
`.ktx2` files in `OUT_DIR` to force a re-encode after changing encoder
settings.

### 3. Atmosphere LUT bake (OPTIMIZE idea 2)

`build.rs` pulls in `src/globe/atmosphere.rs` via `#[path = "src/globe/
atmosphere.rs"] mod atmosphere;` (the file is **not** part of the
runtime crate). `bake_luts` runs `atmosphere::bake()` and writes the
three tables as **uncompressed `R16G16B16A16_SFLOAT` KTX2** files
(transmittance.ktx2 256×64, inscatter_rayleigh.ktx2 /
inscatter_mie.ktx2 256×128). These are the **same f16 texels** the
phase-1 runtime bake produced — bit-identical, zero visual change.
Emits `cargo::rerun-if-changed=src/globe/atmosphere.rs`. Unlike the BC7
cache this reruns on **every** script execution (the bake is
sub-second), so the tables can never go stale after a constants tweak —
no hash-cache needed, cargo's rerun-if-changed is the cache key.

> **Update (2026-06-13):** the bake source is now **inlined directly in
> `build.rs`** as an in-file `mod atmosphere { … }`; `src/globe/
> atmosphere.rs` was deleted and the `#[path]` include removed. The
> paragraph above is otherwise accurate — `bake_luts` still calls
> `atmosphere::bake()` (now resolving to the inline module), the three
> KTX2 outputs and their f16 texels are unchanged (bit-identical). The
> `cargo::rerun-if-changed=src/globe/atmosphere.rs` line was **removed**
> (it pointed at a missing file); cargo already reruns the script on any
> change to `build.rs` itself, so the "never goes stale" guarantee holds
> with `build.rs` as the cache key.

### `write_ktx2` (shared by 2 and 3)

Hand-serializes a single-level 2D KTX2 using the `ktx2` crate's own
types (so writer and runtime `Reader` can't drift): 80-byte `Header`,
one 24-byte `LevelIndex`, a 4-byte DFD-total-length field + a basic DFD
block (`dfd::Basic::from_format` — the runtime `Reader` *requires* a DFD
block; length 0 is rejected), then the raw texel/block data 16-byte
aligned (`next_multiple_of(16)` covers BC7's 16 and RGBA16F's 8).

### Net startup effect

Runtime does **no image decode and no LUT bake** — both moved to build
time. Startup is GPU upload + pipeline creation + device init only.
Trade-offs: embedded bytes grew ~21 MB (jpeg/tiff) → 160 MB (5×32 MiB
BC7) + ~0.6 MB LUTs, so the binary is much larger and links slower
(runtime file loading, OPTIMIZE idea 5, is the known follow-up). VRAM
per color texture dropped 128 MB → 32 MB; uploads are 4× smaller.

## Runtime rendering (src/globe/renderer.rs) — `GlobeRenderer`

Was phase-1 `pipeline.rs` (`shader::Primitive` + `shader::Pipeline`).
Now a plain struct with `new` / `prepare` / `render` — no iced traits.

- `new(device, queue, format)`: compiles `shaders/globe.wgsl`, builds
  the UV-sphere vertex/index buffers (`uv_sphere(64,128)`), the uniform
  buffer, the bind group, and the three render pipelines. Uploads the
  five textures and three LUTs via `upload_ktx2`.
- `prepare(queue, camera, sun, aspect)`: writes the per-frame
  `Uniforms`. (Phase 1 derived aspect from iced widget bounds; now it's
  surface width/height, passed in.)
- `render(render_pass)`: sets bind group + buffers, then the three
  `draw_indexed` calls in order — stars, surface, atmosphere.

### `upload_ktx2`

The single texture-upload path for both BC7 and LUT textures. Parses the
KTX2 (`ktx2::Reader`), maps the format (`BC7_SRGB_BLOCK` →
`Bc7RgbaUnormSrgb`, `BC7_UNORM_BLOCK` → `Bc7RgbaUnorm`,
`R16G16B16A16_SFLOAT` → `Rgba16Float`), takes level 0, and
`create_texture_with_data` with the raw bytes — a straight memcpy to the
GPU, no decode. Replaces phase 1's separate `upload_texture` (image
decode) and `upload_lut` (f16 cast). All textures `mip_level_count 1`.

### Three render pipelines, one bind group, no depth buffer

Unchanged from phase 1. One WGSL module, one shared bind group (group
0), one shared sphere buffer, three pipelines drawn in order into a
single render pass (ordering does the occlusion; no depth buffer
anywhere):

1. **stars** (`vs_stars`/`fs_stars`): sphere ×35, front-face culling
   (seen from inside), no blending. Camera-relative view direction →
   equirectangular star lookup (rotated by `star_rot_inv`) + sun
   disc/glow.
2. **globe surface** (`vs_main`/`fs_main`): unit sphere, back-face
   culling, no blending.
3. **atmosphere** (`vs_atmosphere`/`fs_atmosphere`): sphere
   ×(6460/6360), front-face culling, **additive blending** (One/One).

wgpu-29 descriptor churn applied throughout: `PipelineLayoutDescriptor`
takes `&[Option<&BindGroupLayout>]` + `immediate_size: 0` (was
`push_constant_ranges`); pipelines use `multiview_mask: None` (was
`multiview`); sampler `mipmap_filter` is `MipmapFilterMode::Linear`.

### Uniforms (must match Rust `Uniforms` and WGSL)

```
view_proj: mat4x4<f32>
camera_pos: vec3<f32> + 1 f32 pad
sun_dir: vec3<f32>    + 1 f32 pad
star_rot_inv: mat3x3<f32>   // Rust: 3 columns each padded to [f32;4]
```
`star_rot_inv` = transpose of `sun.star_rotation()`. Written every frame
in `prepare`.

### Bind group 0 layout

| binding | resource | format/notes |
|---|---|---|
| 0 | uniforms | VERTEX_FRAGMENT |
| 1 | day texture | **Bc7RgbaUnormSrgb** (was Rgba8UnormSrgb) |
| 2 | `earth_sampler` | repeat U, clamp V, linear |
| 3 | night texture | Bc7RgbaUnormSrgb |
| 4 | normal map | **Bc7RgbaUnorm** (linear — data) |
| 5 | specular mask | Bc7RgbaUnorm (linear), .r = water |
| 6 | transmittance LUT | Rgba16Float 256×64 |
| 7 | `lut_sampler` | clamp both, linear |
| 8 | inscatter Rayleigh LUT | Rgba16Float 256×128 |
| 9 | inscatter Mie LUT | Rgba16Float 256×128 |
| 10 | stars texture | Bc7RgbaUnormSrgb |

The only change from phase 1 is the five image textures are now BC7
formats instead of Rgba8; the LUTs are still Rgba16Float and the
sampler/binding structure is identical. BC7 is high quality but lossy —
the normal map is the one to eyeball, since `NORMAL_STRENGTH` 4.5
amplifies block artifacts (encoder profile `opaque_slow_settings` is the
quality knob if needed).

## Coordinate and mapping conventions (UNCHANGED from phase 1)

- Globe is a **unit sphere at the origin**; distances in globe radii.
  +Y = north. Lon 0°, lat 0° faces **+Z**; surface point =
  `(cos lat·sin lon, sin lat, cos lat·cos lon)`.
- Equirectangular UVs: u = (lon+180)/360, v = 0 at north → 1 at south.
  Seam column duplicated; sampler repeats U, clamps V.
- Tangent frame: `east = normalize((n.z, 0, −n.x))`, `north = n × east`.
- Vertex position = surface normal = world position (the shaders rely on
  this identity).
- Mesh `uv_sphere(64, 128)`, ~8.4k verts, u32 indices, CCW from outside.

## Camera (src/globe/camera.rs) — mostly unchanged

Orbital: `longitude`, `latitude`, `distance` (radii), `tilt` (deg off
nadir). Defaults 0°N 0°E, distance 2.0, tilt 0. `frame()` builds
eye/target/up; `view_proj(aspect)` = `perspective_rh(45°, aspect, 0.01,
50.0) × look_at_rh`. Clamps: lat ±89°, distance 0.01..10.0, tilt
0..80°; longitude wraps via `rem_euclid`.
`pan_degrees_per_pixel(viewport_height)` = cursor-stable pan scale.

**Phase-2 change**: `zoom(factor)` was replaced by the associated
function `clamp_distance(distance) -> f32` (clamps to MIN/MAX_DISTANCE).
The smoothed-zoom controller owns the distance arithmetic now and only
needs the clamp; the camera no longer applies zoom itself.

## Sun model (src/globe/sun.rs) — UNCHANGED

`Sun { longitude, latitude }` = subsolar point (deg). Default (−40, 15).
`direction()` = unit vector to the sun. `star_rotation() -> Mat3` =
`R_y(lon)·R_x(−lat)` — star map rigidly attached to the sun (deliberately
non-physical; the user rejected the astronomically-correct model in
phase 1). Designed for future animation (time-of-day sweeps longitude,
season moves latitude ±23.44°).

---

# Shader algorithms and atmosphere math (UNCHANGED from phase 1)

Everything in this section is identical to phase 1 — the WGSL
(`shaders/globe.wgsl`) and the bake (`src/globe/atmosphere.rs`) are
byte-for-byte what phase 1 shipped. The **only** structural change is
*where the bake runs*: phase 1 baked the LUTs on the calling thread
inside `Pipeline::new` at first frame; phase 2 bakes them in `build.rs`
and ships f16 KTX2 files, so the runtime just uploads them (see Build
system above). The texels are bit-identical.
(**2026-06-13:** the bake source `src/globe/atmosphere.rs` was inlined
into `build.rs` as `mod atmosphere`; the math is byte-for-byte the same.
Wherever this section says `atmosphere.rs`, the code now lives in that
inline module in `build.rs`.) For the full detailed
reference — texture inventory, equirectangular mapping both directions,
analytic tangent frame, Cook-Torrance GGX BRDF, wave noise, day/night
terminator, the Hillaire-2020 single-scattering model, transmittance
and inscatter LUT parameterizations, the bake pipeline function-by-
function, and the runtime composition — **see the "Shader algorithms
and math (detailed reference)" section of `claude/phase1/PHASE1.md`,
which remains fully accurate.** A summary of the load-bearing points:

- **Surface (fs_main)**: normal mapping in the analytic equirectangular
  tangent frame (`NORMAL_STRENGTH` 4.5, exaggerated); Cook-Torrance GGX
  (D = GGX, G = Smith k=α/2, F = Schlick) with roughness/F0 blended
  land↔ocean by the water mask; static two-octave value-noise glint
  modulation; sunlight color from the transmittance LUT (this makes the
  terminator orange); night-side emissive city lights blended across the
  terminator by `smoothstep(−0.12, 0.18, cos_sun)` on the geometric
  normal.
- **Atmosphere**: single scattering after Hillaire 2020. Medium
  constants (Rayleigh β=(5.802,13.558,33.1)e-3/km H=8; Mie 3.996e-3
  scatter / 4.40e-3 extinction H=1.2 g=0.8; ozone tent at 25±15 km; Rp
  6360, Ra 6460 km) live in `atmosphere.rs` **and** their geometric
  twins in `globe.wgsl`. **These must stay in sync** — change one,
  change both. Transmittance LUT uses the Bruneton (r,μ)
  parameterization; the two inscatter LUTs exploit phase-function
  factoring + spherical symmetry (a ray ≡ impact parameter b + sun
  cosine μ_ref) with a split row mapping (ground rays b<Rp in the lower
  half, limb rays in the upper). `fs_atmosphere` applies the Rayleigh +
  Cornette-Shanks Mie phases and the `1−exp(−L·SUN_INTENSITY)` roll-off
  (SUN_INTENSITY 12).
- **Star/sun backdrop (fs_stars)**: a function of camera-relative view
  *direction* only (true infinity). `dir = star_rot_inv·rel` →
  equirectangular star lookup; `view = rel` → sun disc
  (`SUN_ANGULAR_RADIUS` 0.012) + cubic glow (`SUN_GLOW_RADIUS` 0.12,
  STRENGTH 0.5). Anchoring both lookups to the same camera-relative
  direction is what keeps sun and stars locked under orbit/zoom.
- Look-tuning constants at the top of `globe.wgsl` are frequently
  user-tuned between sessions — **read the file for current values
  rather than trusting any doc.** They are calibrated to the **non-sRGB
  surface** (see Gfx::new); changing the surface transfer function
  invalidates all of them.

## Known issues / context for future work

- **Surface format and present mode are deliberately overridden** in
  `Gfx::new` (non-sRGB, AutoVsync). Lesson: `get_default_config`'s
  defaults are not parity with phase-1 iced — set them explicitly.
- **Binary is large** (~450 MB debug) from 160 MB of embedded BC7.
  Links slowly. Runtime file loading (OPTIMIZE idea 5) is the follow-up
  if it hurts; the textures are self-describing KTX2 already.
- **BC7 is lossy** — eyeball the normal map (NORMAL_STRENGTH amplifies
  artifacts); `opaque_slow_settings` is the quality dial in build.rs.
- **The atmosphere medium constants are duplicated** between
  `atmosphere.rs` (now build-only) and `globe.wgsl` and must stay in
  sync. Tweaking them now costs a build-script rerun (cargo reruns on
  `atmosphere.rs` change) rather than an app restart.
  **Update (2026-06-13):** `atmosphere.rs` is now the inline
  `mod atmosphere` in `build.rs`, so one set of these constants lives
  there and the other in `globe.wgsl` — still duplicated, still must
  stay in sync. The rerun is now keyed on `build.rs` changing (cargo
  always reruns the script on that), not on a separate file.
- **Zoom feel is exhaustively iterated** — tune the named constants,
  don't restructure the glide/coast. See PHASE2_PLAN status log.
- **Texture loading stays sequential** — a parallel `thread::scope`
  version was implemented and deliberately reverted by the owner in
  phase 1; do not reintroduce without asking. (Less relevant now that
  decode moved to build time, but the upload loop is still sequential by
  intent.)
  **Update (2026-06-13):** superseded on explicit request —
  `GlobeRenderer::new` is now parallelized with **rayon** (added as a
  dependency). The phase-1 revert was about decode + LUT bake, both of
  which moved to build time; the remaining runtime work was re-parallelized:
  `rayon::join` runs shader-module compilation alongside the 8 KTX2 GPU
  uploads (`into_par_iter`), and a nested join compiles the 3 render
  pipelines concurrently. Cheap device-object creation (buffers,
  samplers, bind-group layout) is left sequential on purpose. This is
  intentional now — do not treat it as the reverted change.
- **The hidden-window first frame must render via a direct `redraw()`
  call**, not `request_redraw()` (Windows paint-event delivery). Don't
  simplify it.
- **WSLg (the dev environment here) intermittently fails app launch**
  with libEGL/MESA errors — transient, retry; not present on native
  Windows. The owner perf/feel-tests on native Windows release builds;
  WSLg can't validate interaction feel or exact colors.
- Smoke-test pattern: `timeout <n> cargo run 2>&1 | head` (or redirect
  to a file — pipe buffering can swallow output). wgpu validation errors
  panic in the first frames, so a clean 15–25 s run = pipelines/bindings
  valid. First clean build is slow (downloads + one-time BC7 encode of
  5×33 MP); subsequent builds reuse the cached `.ktx2`.
- No HDR/bloom (LDR straight to swapchain; the sun is the disc+glow
  cheat). No heading control, fly-to, tile streaming, or mipmaps. The
  `Sun` model and the animation-capable redraw loop are in place for a
  future time-of-day animation (just another "animating" flag).
- `cargo add` can emit a bogus "found cargo.toml please rename" error on
  this case-insensitive Windows mount — edit Cargo.toml directly and
  trust `cargo metadata`.
