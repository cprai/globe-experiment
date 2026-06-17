# MEMORY.md

Technical reference for the **Globe** viewer: architecture, subsystem
behavior, the rendering and atmosphere math, exact constants, and the
project's history. Companion file: `CLAUDE.md` holds the rules,
conventions, and constraints (what you must / must not do). This file is
the "how and why."

**Constants drift between sessions** — the values quoted here are a
snapshot (2026-06-17). Always read the source for live values; the §
"Live constant snapshot" at the end lists where each lives.

---

## 1. What it is, the stack, the file map

A Google-Earth-style 3D globe viewer. Rust edition 2024. The crate is
named `iced-test-app` for historical reasons (iced was removed in
phase 2). Feature set: day/night Earth with procedural city lights,
normal-mapped terrain relief, GGX ocean sun-glint, Hillaire-2020
precomputed-LUT atmospheric scattering, star + sun backdrop, orbital
camera with pan/tilt/zoom + flick inertia + smoothed zoom, two egui
sliders driving the sun position.

### Dependencies (`Cargo.toml`)

Runtime:
- `wgpu = "29"` — GPU (direct dep; phase 1 used iced's wgpu-27 re-export).
  Version forced by `egui-wgpu` 0.34, which needs `wgpu ^29.0.1`.
- `winit = "0.30"` — window + event loop (egui-winit needs `^0.30.13`).
- `egui = "0.34.3"`, `egui-wgpu = "0.34.3"`, `egui-winit = "0.34.3"` —
  sun-slider overlay + its wgpu paint and winit input bridges.
- `pollster = "0.4"` — blocks on the async wgpu adapter/device requests.
- `glam = "0.33.1"` — camera/sun math (Mat3/Mat4/Quat/Vec3).
- `bytemuck = "1.25" (derive)` — Pod casts for vertex/uniform data.
- `ktx2 = "0.5"` — parses the build-produced KTX2 containers at runtime.
- `rayon = "1.10"` — parallelizes `GlobeRenderer::new`.

Build-dependencies:
- `ureq = "3.3"` — asset download.
- `image = "0.25"` (default-features off, `jpeg` + `tiff`) — decodes the
  downloaded source textures for transcoding.
- `intel_tex_2 = "0.5"` — ISPC BC7 encoder.
- `ktx2 = "0.5"` — writes the KTX2 containers (same crate as runtime, so
  writer and reader cannot drift).
- `half = "2.7" (bytemuck)`, `bytemuck = "1.25"` — the atmosphere LUT bake
  produces f16 texels.

`image` and `half` are **build-only** now (runtime decodes nothing).

Profile overrides: `[profile.dev.package.X] opt-level = 3` for `image`,
`zune-jpeg`, `zune-core`, `tiff`, `miniz_oxide`, `weezl` — they speed up
the build-script transcode decode (5 × 33 MP images). They no longer
affect runtime.

### File map

```
build.rs                 download 5 textures -> BC7/KTX2 transcode +
                         bake 3 atmosphere LUTs to f16 KTX2, all into
                         OUT_DIR. Contains inline `mod atmosphere`.
.cargo/config.toml       Linux-only -lstdc++ for intel_tex_2's ISPC objs
src/main.rs              winit ApplicationHandler: App + Gfx, event loop,
                         wgpu surface/device, frame loop, egui integration
src/ui.rs                egui sun panel (two sliders + labels)
src/globe/mod.rs         module declarations only (camera/input/mesh/
                         renderer/sun) - no logic
src/globe/renderer.rs    GlobeRenderer: all wgpu objects, prepare, render
src/globe/input.rs       Controller: drag/tilt/wheel, flick inertia,
                         smoothed zoom
src/globe/camera.rs      orbital camera
src/globe/sun.rs         subsolar point + star-map orientation
src/globe/mesh.rs        UV-sphere generator
shaders/globe.wgsl       ALL shader code (3 passes in one module)
assets/                  gitignored; downloaded source textures (build input)
OUT_DIR/*.ktx2           gitignored build artifacts, include_bytes!'d:
                         5 BC7 textures + 3 f16 LUTs
CLAUDE.md, MEMORY.md     the docs (this consolidation)
```

Note: `src/globe/atmosphere.rs` **no longer exists** — the bake source was
inlined into `build.rs` as `mod atmosphere` (2026-06-13). Older docs and
the package name still reference both atmosphere.rs and iced; both are
historical.

---

## 2. Phase history (timeline)

- **Phase 1** (to 2026-06-11): the globe, embedded in an **iced 0.14** app
  via the `iced::widget::shader` widget. All the rendering/atmosphere math
  was designed here. iced created the device with `Features::empty()`,
  which blocked compressed textures; textures were decoded at runtime in
  `Pipeline::new` (lazy, first frame); the atmosphere LUTs were baked on
  the CPU at startup.
- **Phase 2** (2026-06-11 → 06-13): **removed iced entirely**, rebuilt the
  host on winit + raw wgpu + egui. Then three optimizations: build-time
  **BC7/KTX2 texture transcode**, build-time **atmosphere LUT bake**, and a
  **hidden-until-ready window**. Heavily iterated the **smoothed-zoom**
  controller. Later (06-13) inlined the LUT bake into `build.rs`, deleted
  `atmosphere.rs`, and **parallelized `GlobeRenderer::new` with rayon**.
  The rendering output stayed pixel-identical to phase 1.
- **Phase 3** (06-13 → 06-15, +06-17 update): **shader-only** change to
  `fs_main`. Dropped the photographic night map as *color*; the whole
  globe is day-mapped and darkened by sun geometry, with **procedural city
  lights** (luminance mask + fixed-grain 3D-noise dither-dissolve +
  additive yellow glow). 06-17 added `EMISSIVE_FADE_END` (lights bleed
  slightly onto the daylit side), converted all source to **ASCII-only**,
  and removed the per-file BC7 transcode cache guard.

---

## 3. Application shell (`src/main.rs`)

A winit `ApplicationHandler`. `main()` builds an `EventLoop`, sets
`ControlFlow::Wait`, runs `App::default()`.

- `App { camera: Camera, sun: Sun, controller: Controller, gfx:
  Option<Gfx> }`. Camera/sun state lives here. **No message enum / no
  indirection** — input mutates the camera directly (the phase-1 iced
  `Interaction { Pan, Zoom, Tilt }` enum is gone).
- `Gfx` holds everything tied to the window/GPU, created once in
  `resumed()`: `window: Arc<Window>`, `surface`, `device`, `queue`,
  `config`, `globe: GlobeRenderer`, `egui_ctx`, `egui_state`,
  `egui_renderer`, and `shown: bool` (hidden-until-ready flag).

### `Gfx::new` — device + surface setup (easy to get wrong)

1. `Instance::new(InstanceDescriptor::new_without_display_handle())`.
2. Surface from `window.clone()`; `request_adapter` (`HighPerformance`);
   `request_device` requesting `Features::TEXTURE_COMPRESSION_BC` and
   `experimental_features: ExperimentalFeatures::disabled()`.
3. `get_default_config(...)`, then **two deliberate overrides** (both are
   `get_default_config` traps — see `CLAUDE.md` "Surface / display"):
   - **Non-sRGB surface format** via `caps.formats.iter().find(|f|
     !f.is_srgb())`. The shader writes linear color; a non-sRGB surface
     stores it raw and the display reads it as sRGB. This is what iced's
     default `web-colors` feature did in phase 1, and every look-tuning
     constant is calibrated to that darker rendition. An sRGB surface
     (hardware encode) renders visibly brighter.
   - **`present_mode = AutoVsync`**. The default takes
     `present_modes.first()`, which is **Mailbox on DX12** — unpaced
     rendering that makes scroll-zoom and inertia judder (frames follow the
     bursty input cadence; the animation loop free-runs with jittery dt).
4. `GlobeRenderer::new(&device, &queue, config.format)` — eager, before
   the first frame. After the BC7/LUT build work this is fast: GPU upload +
   pipeline creation only, no decode, no bake.
5. egui: `Context::default()`, `egui_winit::State::new(...)`,
   `egui_wgpu::Renderer::new(&device, config.format,
   RendererOptions::default())`.

### Event routing (`window_event` → `handle_input`)

Replaces iced's `stack![]` overlay capture. `CloseRequested` exits;
`Resized` reconfigures the surface + requests redraw; `RedrawRequested`
calls `redraw()`. Everything else → `handle_input`:
1. Feed the event to `egui_state.on_window_event(&window, &event)`
   **first**. If `response.repaint`, request a redraw. If
   `response.consumed` (pointer over panel / slider drag), **return** — the
   globe controller never sees it.
2. Else `controller.handle_event(&event, &mut camera, config.height)`; if
   it returns `true` (camera changed or animation started), request redraw.

### Frame (`redraw`)

1. `controller.tick(&mut camera, config.height)` advances flick inertia
   **and** the zoom glide; returns `animating`.
2. egui: `take_egui_input` → `run_ui(|ui| ui::sun_panel(ui.ctx(), &mut
   sun))` → `handle_platform_output`. (0.34 deprecated `Context::run` for
   `run_ui`, whose closure gets a transparent fullscreen `&mut Ui`; the
   Area panel hangs off `ui.ctx()`.)
3. Reassert the grab cursor unless `egui_ctx.is_pointer_over_egui()` (egui
   resets the cursor every frame; method renamed from `is_pointer_over_area`
   in 0.34).
4. `get_current_texture()` returns the wgpu-29 `CurrentSurfaceTexture` enum
   (not `Result`): `Success`/`Suboptimal` carry the frame; `Lost`/
   `Outdated` → reconfigure + redraw + return; `Timeout` → redraw + return;
   `Occluded` → hidden-window first-frame guard (reveal + retry); else skip
   the frame; `Validation` → panic.
5. `globe.prepare(queue, camera, sun, aspect)` writes uniforms (aspect =
   surface width/height).
6. egui tessellate → `update_texture` for each `textures_delta.set`
   **before** the pass → `update_buffers` (returns prep command buffers) →
   one render pass (clear BLACK → `globe.render` → `egui_renderer.render`)
   → submit egui commands chained with the frame encoder →
   `pre_present_notify` → `present` → `free_texture` for each
   `textures_delta.free` **after** submit.
7. Reveal the window on first present (`shown` flips true).
8. Request another redraw iff `animating` or egui reported a zero
   `repaint_delay`.

The single render pass needs a `RenderPass<'static>` for egui-wgpu;
`render_pass.forget_lifetime()` is the documented pattern (the pass still
ends at scope close, before the encoder finishes).

### Hidden-until-ready window

Created `with_visible(false)`; revealed via `set_visible(true)` right after
the first `frame.present()`, so the window appears with the globe already
drawn instead of blank during load. Two non-obvious requirements (both
found by the owner on Windows):
- **`resumed()` calls `self.redraw()` directly** for the first frame, not
  `request_redraw()`. Windows only generates paint messages for *visible*
  windows; a requested redraw on a hidden window never delivers, so the
  reveal code (inside `redraw`) would never run. Presenting to a hidden
  window is fine — only paint-event *delivery* needs visibility.
- **`Occluded` first-frame guard**: some backends report a still-hidden
  window as occluded from `get_current_texture`; if that happens before
  `shown`, show the window and retry rather than deadlocking invisible.

### Redraw policy

`ControlFlow::Wait` + targeted `request_redraw()` on: camera change from
input, active flick inertia or zoom glide (each frame requests the next
until it settles), egui zero repaint delay (slider drag), resize, and
surface lost/timeout recovery. Idle requests nothing → zero frames. Future
sun animation just becomes another "animating" flag.

---

## 4. Camera (`src/globe/camera.rs`)

Orbital model anchored to a look-at point on the surface.

- Fields: `longitude`, `latitude` (look-at point, degrees), `distance`
  (eye→target, globe radii), `tilt` (degrees off nadir; 0 = straight down).
  Defaults: `0°N 0°E`, distance `2.0`, tilt `0`.
- Associated consts: `FOV_Y = 45`, `MIN_DISTANCE = 0.01`,
  `MAX_DISTANCE = 10.0`, `MAX_TILT = 80`. Latitude clamps to `±89°`;
  longitude wraps via `rem_euclid`.
- `frame()` builds `(eye, target, up)`: `radial = (cos lat·sin lon, sin
  lat, cos lat·cos lon)` is both the look-at point on the unit sphere and
  the local up. Local tangent frame `east = normalize(Y × radial)`,
  `north = radial × east`. Tilt is a quaternion about `east`:
  `eye = target + tilt·radial·distance`, `up = tilt·north` — increasing
  tilt swings the eye off straight-down and reveals the horizon to the
  north.
- `view_proj(aspect) = perspective_rh(45°, aspect, 0.01, 50.0) ×
  look_at_rh(eye, target, up)`. The `_rh` variants give wgpu's 0..1 depth
  (no depth buffer is used regardless).
- `pan_degrees_per_pixel(viewport_height)` — cursor-stable panning:
  `world_per_pixel = 2·distance·tan(fov/2)/height`, then `to_degrees()`
  (on the unit sphere, 1 world unit of ground arc = 1 radian). Used by both
  live drags and inertia, so panning tracks the cursor at any altitude.
- `clamp_distance(d) -> f32` clamps to `MIN..MAX_DISTANCE`. (Phase 2
  replaced the old `zoom(factor)` with this — the smoothed-zoom controller
  owns the distance arithmetic and only needs the clamp.)

---

## 5. Input (`src/globe/input.rs`) — `Controller`

Translates winit mouse events into camera motion, mutating `&mut Camera`
directly; `handle_event` returns `bool` (needs redraw). State: `cursor`,
`drag: Option<Drag>`, `inertia: Option<Inertia>`, `zoom: Option<Zoom>`,
plus wheel-cadence tracking (`last_wheel`, `wheel_gap`).

### Pan / tilt
- **Left drag pans** (globe follows cursor): `dlon = −dx·scale`,
  `dlat = +dy·scale`, `scale = camera.pan_degrees_per_pixel`.
- **Right drag tilts**: `−dy·0.25°/px`.
- **Cursor velocity EMA**: `alpha = 1 − e^(−20·dt)` (frame-rate
  independent), tracked on every `CursorMoved` for flick detection.
- Cursor icon `CursorIcon::Grab` / `Grabbing` (set on the window).

### Flick inertia
On left release, if `speed > FLICK_SPEED (50 px/s)` and the last move was
`< FLICK_TIMEOUT (0.1 s)` ago, start coasting. `tick_coast` integrates with
real `Instant`-based dt (capped 0.1 s), pans via the same
`pan_degrees_per_pixel` scale, decays with `HALF_LIFE (0.3 s)`, stops below
`STOP_SPEED (15 px/s)`. Pressing any button cancels inertia.

### Smoothed zoom (the heavily-iterated part — tune constants, do not restructure)

Phase 1 applied each wheel event directly (`distance *= 0.9^ticks`), which
stepped visibly on a trackpad (scroll + OS-synthesized momentum tail arrive
as sparse, quantized events). The current design is a **rate-adaptive glide
with velocity bridging**. Wheel events never move the camera directly — they
move a *target* distance; `tick_zoom` eases the camera toward it each frame.

- **Log space**: `delta = ticks · ln(0.9)`. Zoom is multiplicative, so
  working in log distance keeps perceived speed uniform at any altitude.
  Target clamped via `Camera::clamp_distance`.
- **Adaptive half-life**: `wheel_gap` is an EMA (0.5 blend) of the
  inter-event gap, each sample capped at `WHEEL_GAP_CAP (0.25 s)`. The glide
  half-life = `wheel_gap.clamp(ZOOM_HALF_LIFE_MIN 0.01, ZOOM_HALF_LIFE_MAX
  0.1)`. Dense events (~10 ms, active scroll) → near-instant; sparse events
  (momentum tail, single notches) → interpolate across exactly the gap that
  would otherwise step. **An in-flight glide keeps its `tick`** — resetting
  it per event stalls the glide between dense events.
- **Velocity bridging** (fixes a stall at finger-lift, where a ~10 ms
  half-life drained the target before the momentum tail's first event):
  `Zoom.velocity` is an EMA of the rate events move the target (log-dist/s).
  `tick_zoom` keeps advancing the target at that velocity, decaying with
  `ZOOM_COAST_HALF_LIFE (0.15 s)`, stopping below `ZOOM_STOP_RATE (0.1/s)`,
  and logging each advance in `Zoom.bridged`. The next wheel event
  **repays** `bridged` — only the un-bridged remainder moves the target;
  surpluses carry forward — so while events flow the total zoom equals
  exactly what the device sent; velocity only fills delivery gaps.
- Edge cases: reversing scroll direction zeroes the coast (`velocity` and
  `bridged` reset when `delta·velocity < 0`); a first event (no rate info)
  starts with zero velocity, so single mouse notches don't coast.
- `tick_zoom` eases the camera by exponential approach in log space:
  `blend = 1 − 0.5^(dt/half_life)`, `camera.distance *= (target/distance)^blend`.
  Settles (drops the `Zoom`) when within `1e-3` of the target *and*
  velocity is zero.
- **Rejected designs (do not reintroduce)**: fixed-half-life always-glide
  (laggy during active scroll); a fixed burst-gap split (instant if gap <
  0.05 s, glide otherwise — failed because a momentum tail starts dense and
  decays *through* the threshold, so its mid-tail got classified "active"
  and stepped); direct per-event zoom with no smoothing (briefly shipped,
  reverted).

`tick(camera, height)` runs both `tick_coast` and `tick_zoom`, returning
whether either still needs frames.

---

## 6. Sun (`src/globe/sun.rs`)

`Sun { longitude, latitude }` = the **subsolar point** (the spot where the
sun is directly overhead), in degrees. Default `(−40, 15)` — morning over
the Atlantic, lighting the default camera pose from the upper left.

- `direction()` = unit vector to the sun, same formula as any lat/lon point:
  `(cos lat·sin lon, sin lat, cos lat·cos lon)`.
- `star_rotation() -> Mat3` = `R_y(lon) · R_x(−lat)`: the star map is
  **rigidly attached to the sun** (sun pinned at the map's center). This is
  deliberately **non-physical** — the owner rejected the astronomically
  correct model (see `CLAUDE.md`). Longitude spins the sky about the polar
  axis; latitude tilts it about the horizontal equinox axis.
- Designed for future time-of-day animation: longitude sweeps westward
  360°/day (solar noon at UTC hour `h` ≈ `(12−h)·15°`); latitude moves
  ±23.44° (solar declination) over the year.

The renderer uploads `star_rot_inv` = `star_rotation().transpose()`
(orthonormal, so transpose = inverse) — the shader maps view directions
*back* onto the star texture.

---

## 7. UI (`src/ui.rs`) — egui sun panel

`sun_panel(ctx, &mut Sun)` draws an `egui::Area` anchored top-left
(`[10,10]`, width 260): a white "Sun latitude" label + slider
`−23.44..=23.44` step `0.1` (= season / solar declination), and a "Sun
longitude" label + slider `−180..=180` step `0.5` (= time of day). Sliders
mutate `&mut Sun` directly; `show_value(false)` (the value is in the label,
formatted as `deg` since the ASCII conversion). egui claims its own events
first (see event routing), so dragging a slider never pans the globe.

---

## 8. Mesh (`src/globe/mesh.rs`)

`uv_sphere(stacks, slices)` — renderer calls `uv_sphere(64, 128)` (~8.4k
verts, u32 indices). `Vertex { position: [f32;3], uv: [f32;2] }`.
- For stack/slice fractions `(u, v)`: `lat = 90° − 180°·v`,
  `lon = 360°·u − 180°`, `position = (cos lat·sin lon, sin lat, cos lat·cos
  lon)`, `uv = (u, v)`.
- The seam column at `u=0`/`u=1` is **duplicated** so the texture wraps.
- Indices: two triangles per quad, **CCW when viewed from outside** the
  sphere (so back-face culling keeps the near side for the surface pass).

---

## 9. Renderer (`src/globe/renderer.rs`) — `GlobeRenderer`

A plain struct with `new` / `prepare` / `render` (no iced traits). Owns the
three render pipelines, the shared vertex/index buffers, the uniform
buffer, and the bind group. `STACKS = 64`, `SLICES = 128`.

### `new(device, queue, format)` and its rayon parallelization

Cheap device-object creation (vertex/index/uniform buffers, two samplers,
bind-group layout, bind group) is sequential — it serializes on the
device's internal lock anyway. The expensive work is parallelized:
- One `rayon::join` runs `create_shader_module` (naga parse + validate) on
  one task while the **8 KTX2 textures upload in parallel** via
  `into_par_iter` + `upload_ktx2` on the rest of the pool. `par_iter`
  preserves input order, so the collected views line up with
  `texture_inputs` and the bind-group bindings.
- A nested `rayon::join` compiles the **3 render pipelines** concurrently
  (they share the module + layout, both `Sync`; each does independent
  backend PSO compilation).

This is **intentional** and was added on explicit request (do not confuse
it with the phase-1 reverted parallel decode; see `CLAUDE.md`).

### `upload_ktx2(device, queue, label, bytes) -> TextureView`

The single texture-upload path for both BC7 and LUT textures. Parses the
KTX2 (`ktx2::Reader`), maps the format (`BC7_SRGB_BLOCK` →
`Bc7RgbaUnormSrgb`, `BC7_UNORM_BLOCK` → `Bc7RgbaUnorm`,
`R16G16B16A16_SFLOAT` → `Rgba16Float`), takes level 0, and
`create_texture_with_data` with the raw bytes — a straight memcpy to the
GPU, no decode. All textures `mip_level_count 1`.

### Uniforms (must match Rust `Uniforms` and WGSL `Uniforms`)

```
view_proj:    mat4x4<f32>
camera_pos:   vec3<f32> + 1 f32 pad   (_pad0)
sun_dir:      vec3<f32> + 1 f32 pad   (_pad1)
star_rot_inv: mat3x3<f32>   // Rust: 3 columns each padded to [f32;4]
```

WGSL `mat3x3` columns have vec4 stride, so the Rust struct pads each
column. `star_rot_inv` = transpose of `sun.star_rotation()`. Written every
frame in `prepare` (`queue.write_buffer`, ordered before the frame's
submit).

### Bind group 0 layout

| binding | resource | format / notes |
|---|---|---|
| 0 | uniforms | visibility VERTEX_FRAGMENT |
| 1 | day texture | `Bc7RgbaUnormSrgb`, 8192×4096 |
| 2 | `earth_sampler` | repeat U (dateline seam), clamp V (poles), linear, linear mipmap |
| 3 | night texture | `Bc7RgbaUnormSrgb` |
| 4 | normal map | **`Bc7RgbaUnorm`** (linear — data, not color) |
| 5 | specular mask | `Bc7RgbaUnorm` (linear), `.r` = water |
| 6 | transmittance LUT | `Rgba16Float` 256×64 |
| 7 | `lut_sampler` | clamp both, linear |
| 8 | inscatter Rayleigh LUT | `Rgba16Float` 256×128 |
| 9 | inscatter Mie LUT | `Rgba16Float` 256×128 |
| 10 | stars texture | `Bc7RgbaUnormSrgb` |

`earth_sampler` is shared by all image textures including stars;
`lut_sampler` by the three LUTs. The LUTs are read with
`textureSampleLevel(..., 0.0)` (used in non-uniform control flow; no mips
anyway). **BC7 is lossy** — the normal map is the one to eyeball, since
`NORMAL_STRENGTH` 4.5 amplifies block artifacts (`opaque_slow_settings` in
build.rs is the quality dial).

### `render(render_pass)`

Sets bind group + vertex/index buffers, then three `draw_indexed` calls in
order: **stars → surface → atmosphere**. Draw order does the occlusion (no
depth buffer). The atmosphere is additive over the whole disc (aerial
perspective) and beyond the limb.

---

## 10. Shader (`shaders/globe.wgsl`) — three passes, one module

### Texture inventory (how each is used)

| Texture | Binding | Format | Used by | Role |
|---|---|---|---|---|
| day | 1 | BC7 sRGB | `fs_main` | Surface albedo, **whole globe**. sRGB ⇒ hardware linearizes on sample, so lighting math is linear. |
| night | 3 | BC7 sRGB | `fs_main` | **Luminance mask only** (find cities). Never displayed as color since phase 3. |
| normal | 4 | BC7 UNORM (linear) | `fs_main` | Tangent-space normal, unpacked `*2−1`. x=east, y=north (OpenGL green-up), z=up. Linear is load-bearing — sRGB decode would warp the vectors. |
| specular | 5 | BC7 UNORM (linear) | `fs_main` | Water mask, `.r` (1 = ocean). Blends BRDF params and gates the wave noise. |
| transmittance LUT | 6 | Rgba16Float 256×64 | `fs_main` | T(r, μ): fraction of sunlight surviving to the top of atmosphere. |
| inscatter Rayleigh | 8 | Rgba16Float 256×128 | `fs_atmosphere` | Σ_R: view-ray Rayleigh integral, phase factored out. |
| inscatter Mie | 9 | Rgba16Float 256×128 | `fs_atmosphere` | Σ_M: same for Mie. |
| stars | 10 | BC7 sRGB | `fs_stars` | Equirectangular sky backdrop, sampled by direction. |

### Equirectangular mapping (both directions)

- Forward (mesh): `lat = 90° − 180°v`, `lon = 360°u − 180°`, position as
  above.
- Inverse (`fs_stars`, by direction `d`): `u = atan2(d.x, d.z)/2π + 0.5`,
  `v = acos(d.y)/π`. Runs **per fragment** — interpolating `u` across a
  triangle crossing the ±180° seam would smear the whole texture.

### Surface pass (`vs_main` / `fs_main`)

`vs_main`: `position = view_proj · vec4(pos, 1)`; passes `uv` and `normal =
pos` (unit sphere ⇒ position = normal).

`fs_main`, step by step:

1. Sample day (albedo), night, normal (`*2−1`), specular (`.r` mask).
2. **Analytic tangent frame** at geometric normal `n_geo = normalize(in.normal)`:
   `east = normalize((n_geo.z, 0, −n_geo.x))` (the exact normalized
   ∂position/∂longitude), `north = cross(n_geo, east)`. No per-vertex
   tangents, no seam issues, exact except at the poles (textures are near-
   constant there so it's invisible).
3. **Perturbed normal**: `n = normalize(east·s.x·S + north·s.y·S +
   n_geo·s.z)`, `S = NORMAL_STRENGTH` (scales tangent components before
   renormalize, ≈ multiplying the slope by S; deliberately exaggerated).
4. **Cook-Torrance GGX specular** (`α = roughness²`):
   - D (GGX): `α² / (π · ((n·h)²(α²−1) + 1)²)`, `h = normalize(v + sun)`.
   - G (Smith, Schlick-GGX): `G₁(n·v)·G₁(n·l)`, `G₁(x) = x/(x(1−k)+k)`,
     `k = α/2`.
   - F (Schlick): `F0 + (1−F0)·(1 − v·h)⁵`.
   - `specular = D·G·F / max(4·(n·v)·(n·l), 1e-4) · (n·l)`. `n·v` clamped
     `≥ 1e-4`. Material from the water mask `m`:
     `roughness = mix(LAND_ROUGHNESS, OCEAN_ROUGHNESS, m)`,
     `F0 = mix(LAND_F0, OCEAN_F0, m)` — land rough/dim, ocean smooth/bright
     ⇒ the sun glint is water-only. Wider glint = raise OCEAN_ROUGHNESS;
     brighter = raise OCEAN_F0.
5. **Wave shimmer** (water only, mean-preserving): two-octave 2D value
   noise `wave_noise(uv)` (`hash2`/`value_noise`); `specular *= mix(1, 1 +
   WAVE_STRENGTH·(2·wave−1), m)`. Static by design.
6. **Atmosphere-filtered sunlight**: `cos_sun = dot(n_geo, sun)`;
   `sun_light = sun_transmittance(PLANET_RADIUS_KM + 0.1, cos_sun)` — the
   transmittance LUT color. This single lookup tints the whole lit surface
   and is what makes the terminator go orange. Applied to **both** diffuse
   and specular.
7. **Diffuse + composite**: `day_lit = albedo·(DAY_AMBIENT + (1−DAY_AMBIENT)
   ·(n·l)·sun_light) + specular·sun_light`.
8. **Night-side darkening** (geometric normal, see `CLAUDE.md`):
   `daylight = smoothstep(−0.12, 0.18, cos_sun)`,
   `night_factor = mix(NIGHT_DARKNESS, 1.0, daylight)`,
   `surface = day_lit · night_factor`.
9. **Procedural city lights (phase 3)** — see next section.

### City lights: dither-dissolve (phase 3, in `fs_main`)

The dark side is the day map scaled by `night_factor`; cities are
re-synthesized procedurally from the night map's *luminance*, never its
color.

```wgsl
let night_brightness = dot(night, vec3(0.2126, 0.7152, 0.0722));
let lit  = smoothstep(EMISSIVE_THRESHOLD,
                      EMISSIVE_THRESHOLD + EMISSIVE_SOFTNESS,
                      night_brightness);          // near-binary city mask
let fade = smoothstep(EMISSIVE_FADE_START, EMISSIVE_FADE_END, cos_sun);
let dither = value_noise_3d(n_geo * DITHER_SCALE);
let keep = step(fade, dither);                    // hard per-pixel dither
surface += lit * keep * EMISSIVE_COLOR * EMISSIVE_STRENGTH;
```

Why it works:
- `lit` is a near-binary mask from one **fixed luminance threshold**
  (`EMISSIVE_SOFTNESS` gives a soft edge that also absorbs BC7 softness).
- `fade` ramps `0 → 1` over `cos_sun ∈ [EMISSIVE_FADE_START,
  EMISSIVE_FADE_END]`. With the current `[−0.15, 0.15]`, the terminator
  (`cos_sun = 0`) is the **midpoint**, so ~half the city pixels survive
  *past* the terminator and finish dissolving by `cos_sun = 0.15` on the
  daylit side — the intended bleed. `EMISSIVE_FADE_END = 0` recovers the
  old "gone exactly at the terminator" behavior. The pair must satisfy
  `FADE_START < FADE_END` (smoothstep edges must be ordered).
- `dither = value_noise_3d(n_geo · DITHER_SCALE)` is **constant per surface
  point** (fixed scale, surface-anchored), so it never crawls or reshuffles
  under zoom/rotate/sun-motion.
- `keep = step(fade, dither)` is a hard per-pixel dither (uniform-brightness
  survivors, no dimming). Each pixel switches off exactly when `fade`
  crosses *its own* `dither` value, so as the terminator sweeps, pixels drop
  out in a **stable order** — a coherent wipe, **no fizz/boil**. (This is
  why the grain is fixed-scale; a frequency ramp makes the per-point value
  uncorrelated frame-to-frame and the band boils.)

**3D noise helpers** (next to the 2D `hash2`/`value_noise`/`wave_noise`):
- `hash3(p)` — an **integer-lattice bit-mixing hash** (cast i32→u32, three
  large-prime multiplies, XOR-folds, normalize to `[0,1]`). Deliberately
  *not* `fract(sin(...))`: `n_geo·DITHER_SCALE` (≈ ×400) pushes lattice
  indices into the hundreds where f32 `sin()` loses precision and bands.
  `p` arrives integer-valued (the floored cell corner).
- `value_noise_3d(p)` — trilinear interpolation of `hash3` over the 8 cube
  corners with a `f²(3−2f)` fade. Single octave (a second octave is an easy
  quality bump; keep it fixed-scale to preserve the coherent wipe).

> **Design note (`NIGHT_DARKNESS = 1.2` is intentional)**: since
> `night_factor = mix(NIGHT_DARKNESS, 1.0, daylight)`, a value > 1 makes the
> unlit hemisphere ~20 % **brighter** than full daylight. This is a
> deliberate departure from the original PHASE3 plan's near-black-night
> intent (`~0.02`) — the owner set it this way on purpose; the globe reads
> bright all the way around with the city glow layered on top. **This is the
> shipped look — do not "revert" it toward the plan's value.**
> (`EMISSIVE_THRESHOLD = 0.05` likewise diverges from the plan's starting
> `0.25`, making the city mask more permissive.) Mechanically, `DAY_AMBIENT`
> sets the floor of `day_lit` and `night_factor` scales it, so `< 1` would
> darken the night side (`0` = black night).

A naming gotcha hit during implementation: the noise var cannot be `n`
(that's the perturbed normal) — it is `dither`.

### Atmosphere pass (`vs_atmosphere` / `fs_atmosphere`)

The sphere mesh inflated to `ATMOSPHERE_SHELL = ATMOSPHERE_TOP_KM /
PLANET_RADIUS_KM` (= 6460/6360), rendered **front-face-culled** (far side of
the shell, so it spans the whole silhouette beyond the limb) with
**additive blending** (One/One). Works in **km**, planet center at origin.

Per fragment:
1. `origin = camera_pos · PLANET_RADIUS_KM`, `dir = normalize(world_pos·Rp −
   origin)`. `ray_sphere(origin, dir, Ra)`; discard if it misses the shell.
2. **Impact parameter** `b = length(origin − dot(origin,dir)·dir)` (closest
   approach to the planet center).
3. **Split row mapping** (must match the bake): `ray_sphere(origin, dir,
   Rp)`; if it hits the ground, reference = ground hit and
   `v = 0.5·clamp(b/Rp)`; else reference = closest approach and
   `v = 0.5 + 0.5·clamp((b−Rp)/(Ra−Rp))`.
4. `μ_ref = dot(normalize(reference), sun)`; `uv = (μ_ref·0.5+0.5, v)`.
   Sample both inscatter LUTs.
5. **Phase functions** (constant along a ray): `μ = dot(dir, sun)`;
   `Φ_R = 3/(16π)(1+μ²)`; Cornette-Shanks
   `Φ_M = 3/(8π)·(1−g²)(1+μ²) / ((2+g²)(1+g²−2gμ)^1.5)`, `g = MIE_G`.
6. `L = Σ_R·Φ_R + Σ_M·Φ_M`; tone roll-off `color = 1 − exp(−L·SUN_INTENSITY)`
   (soft-clips the bright limb).

`ray_sphere(origin, dir, R)` returns the two roots `t = −b' ± √(b'²−c)`,
`b' = origin·dir`, `c = |origin|² − R²`, or `(−1,−1)` on a miss.

### Star + sun backdrop (`vs_stars` / `fs_stars`)

Sphere inflated to `STARS_RADIUS = 35` (must enclose the camera at max
distance ~11 but stay inside the 50-radii far plane), rendered front-face
(seen from inside), no blending, **before everything**.

`vs_stars` computes the **camera-relative** view direction
`relative = world − camera_pos` (linear in the vertex ⇒ exact under
interpolation), output twice: `dir = star_rot_inv · relative` (for the star
lookup) and `view = relative` (world frame, for the sun). Both normalized
per fragment.

`fs_stars`:
- Star color: equirectangular lookup from `normalize(dir)` (inverse mapping,
  per fragment so the seam doesn't smear).
- Sun: `angle = acos(dot(normalize(view), sun))`; AA disc
  `1 − smoothstep(0.85·SUN_ANGULAR_RADIUS, SUN_ANGULAR_RADIUS, angle)` plus
  cubic glow `SUN_GLOW_STRENGTH · max(1 − angle/SUN_GLOW_RADIUS, 0)³`.
- `color = stars·STARS_BRIGHTNESS + SUN_COLOR·(disc + glow)`.

Anchoring **both** lookups to the same camera-relative direction is what
keeps sun and stars locked under orbit/zoom (the backdrop is at infinity —
no parallax, no zoom dependence); the globe drawn afterward occludes it.
The 35-radius geometry is purely a screen-coverage device — nothing of it
shows.

---

## 11. Atmosphere model (Hillaire 2020) — the math and the bake

Single-scattering atmosphere with Earth's standard medium (Rayleigh + Mie +
ozone). The per-pixel raymarch is replaced by two precomputed LUTs, baked
on the CPU in `build.rs`'s `mod atmosphere` and uploaded as f16 KTX2.

### Medium (defined in `build.rs mod atmosphere`; per-km coefficients)

At altitude `h` (km):
- **Rayleigh**: `σs_R(h) = (5.802, 13.558, 33.1)e-3 · exp(−h/8)` (scattering
  = extinction; no absorption). The 1 : 2.3 : 5.7 blue bias makes the sky
  blue and the transmitted light orange.
- **Mie**: `σs_M(h) = 3.996e-3 · exp(−h/1.2)`, extinction
  `4.40e-3 · exp(−h/1.2)` (the difference is absorption). Phase asymmetry
  `g = 0.8` (Cornette-Shanks). Both `extinction()` and `scattering()` exist
  because Mie scatters less than it extinguishes.
- **Ozone**: absorption only, `(0.650, 1.881, 0.085)e-3 · max(0, 1 −
  |h−25|/15)` — a tent peaking at 25 km, ±15 km. The biggest contributor to
  sunset reds/purples.
- Total extinction `σt(h)` = sum of the three.
- Geometry: `PLANET_RADIUS_KM = 6360`, `ATMOSPHERE_TOP_KM = 6460`.

### Transmittance LUT (256×64; `bake_transmittance`, sampled by `sun_transmittance`)

Beer-Lambert `T = exp(−∫σt dt)` integrated to the top of the atmosphere (40
midpoint steps). **Bruneton (r, μ) parameterization** — resolution
concentrates near the horizon where T changes fastest:
- `ρ = √(r²−Rp²)`, `H_top = √(Ra²−Rp²)`. y axis: `x_r = ρ/H_top`.
- x axis: `d = −r·μ + √(r²(μ²−1) + Ra²)` (distance to atmosphere top),
  `x_mu = (d − d_min)/(d_max − d_min)`, `d_min = Ra − r`, `d_max = ρ + H_top`.
- The bake **inverts** this mapping per texel to recover `(r, μ)`, then
  integrates optical depth: at `t = (s+0.5)·dt`, `r(t) = √(r²+t²+2rμt)`
  (law of cosines), `h = r(t) − Rp` (clamped ≥ 0), accumulate `σt(h)·dt`.
  Store `exp(−depth)`. Kept in **full f32** for the inscatter bake (re-sampled
  thousands of times), converted to f16 only for the GPU copy.
- The WGSL `sun_transmittance(r, μ)` and the CPU `sample_transmittance`
  additionally return 0 when `μ` is below the geometric horizon cosine
  `−√(1 − (Rp/r)²)` — **the planet's own shadow**. The CPU twin does a
  hand-rolled bilinear fetch at texel centers, mirroring GPU filtering so
  bake-time and draw-time reads match.
- `fs_main` uses `T(Rp + 0.1, cos_sun)` directly as the **color of sunlight
  at the ground**.

### Inscatter LUTs (2 × 256×128; `bake_inscatter`)

Single-scattering luminance along a view ray
`L = ∫ T_view(t)·σs(h)·Φ(μ_view·sun)·T_sun(p(t)) dt`. Two exploits make it
precomputable for this scene:
1. **Phase functions factor out** — `dir·sun` is constant along a straight
   ray, so `L = Φ_R·Σ_R + Φ_M·Σ_M` with Σ the integral *without* phase.
2. **Spherical symmetry** — viewed from outside a sphere, a ray is fully
   described by its impact parameter `b` and one sun angle `μ_ref` (the sun
   cosine at a **reference point**: the ground hit for `b < Rp`, else the
   closest-approach point).

LUT axes: x = `μ_ref·0.5 + 0.5`; y = **split mapping** — `0.5·(b/Rp)` for
ground-hitting rays (lower half), `0.5 + 0.5·(b−Rp)/(Ra−Rp)` for limb rays
(upper half). The split gives the thin bright limb band half the table.
**This mapping is implemented independently in the bake and in
`fs_atmosphere` and must match.**

Bake geometry (canonical 2D frame): ray along +x, closest approach `(0, b)`;
entry `t = −√(Ra²−b²)`; exit `−√(Rp²−b²)` (ground) or `+√(Ra²−b²)` (limb).
Reference point normalized with a 1e-3 floor (b→0 head-on rays would
normalize a zero vector). Per step (32 midpoint), the **one approximation**:
the sun's tilt *along* the ray is treated as perpendicular —
`μ_sun(t) = μ_ref · (p̂(t) · r̂_ref) = μ_ref·(t·ref_x + b·ref_y)/r`. Then,
Hillaire eq. 11 (analytic inscatter across a constant-medium step, removes
step-count sensitivity):
```
step_trans = exp(−σt·dt)
integ      = T_view · t_sun · (1 − step_trans) / max(σt, 1e-6)
Σ_R += σs_R · integ ;  Σ_M += σs_M · integ ;  T_view *= step_trans
```
`t_sun = sample_transmittance(r, μ_sun)` (carries the planet shadow into the
LUT). rgb = Σ, alpha = 1.0, pushed as f16 row-major.

### Bake driver

`bake() -> Luts`: `bake_transmittance()` (f32) → `to_f16_texels` for the GPU
copy, and → `bake_inscatter(&transmittance)` → the two f16 inscatter
tables. The whole bake is sub-second.

### Gotchas when modifying
- Any change to the medium constants, the split mapping, the reference-point
  choice, or the Bruneton mapping must be made in **both** `build.rs mod
  atmosphere` and `globe.wgsl`.
- f16 max is 65504, min normal ~6e-5; keep any large scale factor (e.g.
  `SUN_INTENSITY`) **in the shader, not the bake**, or precision suffers.
- `μ_ref = ±1` is never baked exactly (texel centers); don't add math that
  depends on exact endpoint values.

---

## 12. Build pipeline (`build.rs`)

Three jobs, all writing into `OUT_DIR` (which the runtime `include_bytes!`-es);
`assets/` holds only the downloaded source images (gitignored build input).

### 1. Download
`ASSETS`: five solarsystemscope.com textures, each tagged `srgb: bool` —
`8k_earth_daymap.jpg`, `8k_earth_nightmap.jpg` (srgb), `8k_earth_normal_map.tif`,
`8k_earth_specular_map.tif` (linear/data), `8k_stars_milky_way.jpg` (srgb);
all 8192×4096. `download_if_missing` derives the filename, emits
`cargo::rerun-if-changed=assets/<name>`, skips if present, else downloads via
ureq (100 MB cap). Deleting a file re-triggers its download.

### 2. BC7 → KTX2 transcode
`transcode(source, srgb, out_dir)` decodes (`image`), asserts multiple-of-4
dimensions (BC7 block size), BC7-compresses with
`intel_tex_2::bc7::compress_blocks(opaque_basic_settings(), surface)`, and
writes `<stem>.ktx2`. `srgb` → `BC7_SRGB_BLOCK` (day/night/stars), data →
`BC7_UNORM_BLOCK` (normal/specular — must stay linear).
- **Runs unconditionally on every build-script execution** (the per-file
  `if dest.exists()` skip guard was removed 2026-06-17). cargo's
  `rerun-if-changed` (per asset + `build.rs` itself) is the sole rerun gate.
  Consequence: a no-change rebuild skips the script entirely, but **any**
  rerun (editing `build.rs`, touching a texture) re-encodes **all five** BC7
  textures (~1.5 min). An mtime guard and a content-hash cache were both
  considered and deliberately not adopted (owner preferred pure
  rerun-if-changed); revisit if the all-or-nothing re-encode becomes painful.

### 3. Atmosphere LUT bake
`bake_luts` runs the inline `atmosphere::bake()` and writes the three tables
as uncompressed `R16G16B16A16_SFLOAT` KTX2 (transmittance 256×64, two
inscatter 256×128). Runs **unconditionally** (sub-second), so the tables can
never go stale after a constants tweak. The WGSL twin constants still need
manual sync.

### `write_ktx2(format, width, height, blocks)` (shared by 2 and 3)
Hand-serializes a single-level 2D KTX2 using the `ktx2` crate's own types
(so writer and runtime `Reader` can't drift): 80-byte `Header`, one 24-byte
`LevelIndex`, a 4-byte DFD-total-length field + a basic DFD block
(`dfd::Basic::from_format` — the `Reader` *requires* a DFD; length 0 is
rejected), then the raw data 16-byte aligned (`next_multiple_of(16)` covers
BC7's 16 and RGBA16F's 8).

### Net startup effect
Runtime does **no image decode and no LUT bake** — both moved to build time.
Startup is GPU upload + pipeline creation + device init only. Trade-off:
embedded bytes grew ~21 MB (jpeg/tiff) → ~160 MB (5 × 32 MiB BC7) + ~0.6 MB
LUTs, so the binary is large and links slowly (runtime file loading is the
known follow-up). VRAM per color texture dropped 128 MB → 32 MB; uploads 4×
smaller.

---

## 13. Live constant snapshot (2026-06-17 — verify against source)

**`shaders/globe.wgsl`** (top look-tuning block + atmosphere/star consts):
```
DAY_AMBIENT 0.04   NORMAL_STRENGTH 4.5
LAND_ROUGHNESS 0.9   OCEAN_ROUGHNESS 0.45   LAND_F0 0.015   OCEAN_F0 0.15
WAVE_SCALE 2200.0   WAVE_STRENGTH 0.04
EMISSIVE_THRESHOLD 0.05   EMISSIVE_SOFTNESS 0.1
EMISSIVE_COLOR (1.0, 0.85, 0.3)   EMISSIVE_STRENGTH 1.5
EMISSIVE_FADE_START -0.15   EMISSIVE_FADE_END 0.15
DITHER_SCALE 400.0   NIGHT_DARKNESS 1.2      // >1 brightens night side, intentional (§10)
PLANET_RADIUS_KM 6360.0   ATMOSPHERE_TOP_KM 6460.0   MIE_G 0.8   SUN_INTENSITY 12.0
STARS_RADIUS 35.0   STARS_BRIGHTNESS 0.8
SUN_ANGULAR_RADIUS 0.012   SUN_GLOW_RADIUS 0.12   SUN_GLOW_STRENGTH 0.5
SUN_COLOR (1.0, 0.96, 0.9)
day/night terminator smoothstep: smoothstep(-0.12, 0.18, cos_sun)
```
**`build.rs` mod atmosphere**:
```
RAYLEIGH_SCATTERING [5.802, 13.558, 33.1]e-3   RAYLEIGH_SCALE_HEIGHT 8.0
MIE_SCATTERING 3.996e-3   MIE_EXTINCTION 4.40e-3   MIE_SCALE_HEIGHT 1.2
OZONE_ABSORPTION [0.650, 1.881, 0.085]e-3       (tent peak 25 km, ±15)
TRANSMITTANCE 256×64 / 40 steps   INSCATTER 256×128 / 32 steps
```
**`src/globe/input.rs`**:
```
FLICK_SPEED 50   STOP_SPEED 15   HALF_LIFE 0.3   FLICK_TIMEOUT 0.1
ZOOM_HALF_LIFE_MIN 0.01   ZOOM_HALF_LIFE_MAX 0.1   WHEEL_GAP_CAP 0.25
ZOOM_COAST_HALF_LIFE 0.15   ZOOM_STOP_RATE 0.1
```
**`src/globe/camera.rs`**: FOV_Y 45, MIN_DISTANCE 0.01, MAX_DISTANCE 10.0,
MAX_TILT 80; defaults lon 0, lat 0, distance 2.0, tilt 0; lat clamp ±89.
**`src/globe/sun.rs`**: default (−40, 15). **`renderer.rs`**: STACKS 64,
SLICES 128.

---

## 14. Known issues & open follow-ups

Implemented startup optimizations (history): parallelize renderer setup
(rayon ✓), build-time LUT bake (✓), BC7/KTX2 transcode (✓). **Not done**:
- **Downsize textures to 4K** in `build.rs` (would quarter decode/upload;
  current zoom rarely samples past 4K density).
- **Placeholder-then-sharpen**: embed tiny textures for an instant first
  frame, load full-res async, swap the bind group. Fixes *perceived*
  startup; the largest effort.
- **Runtime file loading** instead of `include_bytes!` — shrinks the binary
  and link time, unblocks hot-swapping textures. The KTX2 files are
  self-describing already.

Other standing items:
- **No mipmaps** → far-zoom shimmer; the city-light dither can twinkle/alias
  sub-pixel at low zoom (no MSAA). Mitigations: lower `DITHER_SCALE`, or swap
  `step(fade, dither)` for a narrow `smoothstep`.
- **Expose emissive params (threshold/strength/color/fade) as uniforms +
  egui controls** for interactive tuning.
- **Second noise octave** in `value_noise_3d` if the grain reads too regular
  (keep it fixed-scale).
- **Real bloom post-process** for an actual glow halo (new pass, large) —
  explicitly **declined** for now.
- **No heading control, no fly-to animation, no tile streaming** (the "v2
  leap" from textured globe toward real Google Earth). The `Sun` model and
  the animation-capable redraw loop are already in place for a future
  time-of-day animation (just another "animating" flag).

### wgpu 27 → 29 churn (reference, in case of a future bump)
From the phase-2 migration: `Instance::new` takes `InstanceDescriptor` by
value, no `Default` (use `new_without_display_handle()`); `DeviceDescriptor`
gained `experimental_features`; `get_current_texture()` returns the
`CurrentSurfaceTexture` enum, not `Result`; `PipelineLayoutDescriptor` takes
`&[Option<&BindGroupLayout>]` + `immediate_size: 0` (replaced
`push_constant_ranges`); `multiview` → `multiview_mask` on pipeline and
render-pass descriptors; color attachments gained `depth_slice: None`;
`RenderPassDescriptor` gained `multiview_mask: None`; sampler `mipmap_filter`
is `MipmapFilterMode`. egui 0.34: `Context::run` → `run_ui`,
`is_pointer_over_area` → `is_pointer_over_egui`, `Renderer::new` takes
`RendererOptions`.
