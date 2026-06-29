---
paths:
  - "src/renderer/**/*.rs"
  - "src/application/mod.rs"
---

# Renderer & application shell rules

## Startup / first frame (do not simplify either)

- **`ApplicationState::resumed()` calls `self.redraw()` directly**, never
  `request_redraw()`. Windows does not deliver `RedrawRequested` to a hidden
  window. The reveal code (inside `redraw`) would never run otherwise.
- **Window is created `with_visible(false)`** and revealed via `set_visible(true)`
  right after the first `frame.present()`, so it appears with the scene
  already drawn.
- **`Occluded` first-frame guard**: some backends (macOS) report a still-hidden
  window as `Occluded` from `get_current_texture`. If that happens before
  `shown`, show the window and retry rather than deadlocking invisible.

## egui texture-delta ordering (easy to regress)

**`egui textures_delta.set` must apply BEFORE the surface acquire in
`Gfx::update`.** The `free` deltas deliberately stay after present.

Why: egui emits each texture delta **exactly once** and then forgets it. A
full allocation delta (font atlas creation) is emitted only once; afterwards
egui sends only partial updates (one per newly rasterized glyph). If a frame
carrying a `set` delta exits early (via `Occluded`/`Lost`/`Outdated`/`Timeout`
before the apply step), the delta is lost for good. The next partial update
for that id panics: "Tried to update a texture that has not been allocated
yet." `update_texture` needs only the device/queue, not the swapchain frame,
so hoisting it above the acquire is free. A dropped `free` delta is benign
(delays cleanup only).

## Idle policy

**Idle = zero GPU work.** `ControlFlow::Wait` + targeted `request_redraw()`.
Never add an unconditional vsync loop. Redraws are requested on: camera
change, active flick inertia or zoom glide, simulation clock running (each
frame requests the next while playing), egui zero repaint delay, resize,
surface lost/timeout recovery.

## `SceneRenderer::new` parallelization

`create_shader_module` runs on one rayon task while 9 group-0 texture inputs
load in parallel via `into_par_iter` (the 7 planet textures decode in a separate
`par_iter` into group-1 bind groups). A nested `rayon::join` compiles the 5
group-0 render pipelines concurrently (Earth surface, atmosphere, stars, markers,
Moon); the **planet mesh pipeline** (a 6th) and the **planet billboard pipeline**
(a 7th) are built after the join (they borrow `planet_layout`). This is the
**sanctioned** parallel decode — do not confuse it with the phase-1 reverted
`thread::scope` approach.

## `Gfx::init` device setup

`Gfx::init` takes `OwnedDisplayHandle` (from `event_loop.owned_display_handle()`)
and passes it to `InstanceDescriptor::new_with_display_handle`. This is required
for the GLES/EGL backend to open its display connection — without it, GL adapter
enumeration fails and the app panics with "no GPU adapter found" on any system
where Vulkan is absent (WSL, some CI environments). winit defaults to Wayland on
Linux even in WSL, and Wayland + EGL requires the display handle at instance
creation time per the wgpu docs.

Key overrides from `get_default_config` defaults (both are load-bearing):
1. **Non-sRGB surface format**: `caps.formats.iter().find(|f| !f.is_srgb())`.
2. **`present_mode = AutoVsync`**: the default picks Mailbox on DX12 (unpaced,
   causes judder).
Device requested with `Features::empty()` + `experimental_features: disabled`.

## Render frame — all positions are camera-target-local

**The whole shader works in the render frame: every position uniform is relative
to the camera target's center.** `prepare` subtracts `RenderState.render_origin`
on the CPU from each absolute body position before upload, so the GPU never sees
an absolute world position (the far-planet f32-jitter fix). There is **no
`render_origin` uniform and no `sun_dir`** — the shader is purely local and
derives every Sun direction from `sun_pos`. The orbited body's position is a
bit-exact zero (`pos - render_origin`), so its mesh draws in local coordinates.
For Earth/Moon (`render_origin == 0`) the render frame is the absolute frame.

## Uniforms struct layout (must match Rust `Uniforms` and WGSL `Uniforms`)

```
view_proj:    mat4x4<f32>            // built in the render frame (camera.rs)
camera_pos:   vec3<f32> + 1 f32 pad  (_pad0)   // eye, render frame km
star_rot_inv: mat3x3<f32>            // Rust: 3 columns each padded to [f32;4]
marker:       vec4<f32>              // x,y = viewport px; z = radius px; w = unused
moon_rot:     mat3x3<f32>            // body-fixed -> world; cols padded to [f32;4]
moon_pos:     vec3<f32> + 1 f32 pad (_pad2)   // Moon center, render frame km
moon_params:  vec4<f32>              // x = Moon radius km; y = Earth radius km;
                                     //   z = Sun angular radius rad; w = unused
sun_pos:      vec3<f32> + 1 f32 pad (_pad4)   // Sun, render frame km; lights every
                                     //   body + aims the backdrop disc
```

Key: WGSL `mat3x3` columns have `vec4` stride, so the Rust struct pads each
column to 4 floats. Per-marker position + visibility live in the **marker
instance buffer** (`MarkerInstance { position: vec3, visible: f32 }`), not
in the uniform block. The uniform and marker instances are written every frame
in `prepare` (`queue.write_buffer`). The Moon mesh is a separate vertex/index
buffer (`mesh::moon_ellipsoid`), drawn with its own model transform via
`moon_rot`/`moon_pos` — sourced in `prepare` from the Moon entry of
`RenderState.celestial_bodies` (`EarthSystem(Moon)`), its radius from the
identity (`moon::MEAN_RADIUS_KM`); the Earth shell mesh is rebound for the
atmosphere pass that follows it.

**Planets (group 1).** Each planet has its own mesh (`mesh::planet_ellipsoid`),
a per-planet `PlanetUniform` (`rot` mat3x3 cols->vec4 + `pos` vec3 (render frame)
+ `equatorial_radius_km` + `polar_radius_km` + pad), and a group-1 bind group
(uniform + texture + the shared sampler). `prepare` walks the planet entries of
`RenderState.celestial_bodies` (a body whose identity is found in `planet::ALL`),
maps each to its GPU slot by its position in `planet::ALL`, then classifies it by
apparent angular size and
routes it to one of two
shared pipelines (both layout `[group0, group1]`): the **mesh** pipeline
(`vs_planet`/`fs_planet`, same solid-body reversed-Z depth as the Moon) for
large/near planets, or the **billboard** pipeline (`vs_planet_billboard`/
`fs_planet_billboard`, no vertex buffer, depth-off) for far ones. The mesh and
billboard draw indices are rebuilt each `prepare` (`mesh_planet_indices` /
`billboard_planet_indices`, the latter far-to-near sorted); both empty except the
solar-system scenario. The 7 planet textures live ONLY in group 1, so group 0
stays at 9 sampled textures — clear of the portable 16-per-stage limit.

**Earth-system gate.** `prepare` sets `draw_earth_system = (render_origin == 0)`;
`render` draws the Earth surface, atmosphere, Moon, and markers only when true
(orbiting Earth/Moon). Orbiting a planet they are skipped. **Draw order: stars
-> distant-planet billboards -> Earth surface -> Moon -> near-planet meshes ->
atmosphere -> markers** (the Earth-system ones gated; billboards draw right after
the backdrop so the later opaque bodies paint over them).

## Bind group 0 layout

| binding | resource | format / notes |
|---|---|---|
| 0 | uniforms | visibility VERTEX_FRAGMENT |
| 1 | day texture | `Rgba8UnormSrgb`, 8192x4096 (decoded JPEG) |
| 2 | `earth_sampler` | repeat U (dateline seam), clamp V (poles), linear |
| 3 | night texture | `Rgba8UnormSrgb` (decoded JPEG) |
| 4 | normal map | **`Rgba8Unorm`** (linear — data, not color; decoded TIFF) |
| 5 | specular mask | `Rgba8Unorm` (linear), `.r` = water (decoded TIFF) |
| 6 | transmittance LUT | `Rgba16Float` 256x64 |
| 7 | `lut_sampler` | clamp both, linear |
| 8 | inscatter Rayleigh LUT | `Rgba16Float` 256x128 |
| 9 | inscatter Mie LUT | `Rgba16Float` 256x128 |
| 10 | stars texture | `Rgba8UnormSrgb` (decoded JPEG) |
| 11 | moon texture | `Rgba8UnormSrgb`, 8192x4096 (decoded JPEG; lunar albedo) |

`earth_sampler` is shared by all image textures including stars, the moon, and
the planets; `lut_sampler` by the three LUTs. LUTs are read with
`textureSampleLevel(..., 0.0)` (used in non-uniform control flow; no mips
anyway). Normal map linear format is load-bearing — sRGB decode would warp the
tangent vectors.

## Bind group 1 layout (planets)

Used by both planet pipelines (mesh + billboard); one bind group per planet, the
right one set per draw.

| binding | resource | notes |
|---|---|---|
| 0 | per-planet `PlanetUniform` | VERTEX_FRAGMENT; `rot` + `pos` (render frame) + `equatorial_radius_km` + `polar_radius_km` |
| 1 | planet texture | `Rgba8UnormSrgb` (8K for inner/gas, 2K for ice giants) |
| 2 | sampler | the shared `earth_sampler` (repeat U / clamp V) |

## Depth buffer

`Depth32Float`, reversed-Z (see `shader.md` and `camera.md`). `Gfx` owns a
`depth_view` recreated on resize; `HeadlessRenderer` owns one sized to its
target. Shared helpers `create_depth_view` + `depth_attachment` (cleared to
`0.0`) build both. All six scene pipelines declare `depth_stencil`; egui's
overlay pipeline is built with `depth_stencil_format: Some(DEPTH_FORMAT)`.

## Renderer constants

`STACKS 64`, `SLICES 128` (mesh resolution; shared by the Earth, Moon, and
planet meshes), `MARKER_RADIUS_PX 6`, `DEPTH_FORMAT Depth32Float`,
`SUN_ANGULAR_RADIUS_RAD 0.004652` (eclipse penumbra width).

## Headless render mode (`HeadlessRenderer` + `snapshot`)

- Offscreen format is **`Rgba8Unorm` (non-sRGB), on purpose** — twin of the
  surface format rule. The stored bytes already equal the sRGB-encoded
  on-screen pixels; written verbatim to PNG.
- Shares `SceneRenderer` and `request_adapter_device` with the windowed path.
  `HeadlessRenderer` passes `compatible_surface: None` to the adapter.
- **No EOP range check** in render mode. Out-of-range datetimes render and
  silently degrade. This is deliberate — documented in `snapshot.rs` and
  `scenarios.md`.
- **No markers** in render mode (`RenderState.markers` is empty). The whole
  body list (Earth system + all 7 planets) is filled in
  `RenderState.celestial_bodies` (`celestial.bodies.clone()`), so `camera.target`
  can be any of `"earth"`, `"moon"`, or a planet (`"mars"`, ..., `"neptune"`);
  the camera's `render_origin` is set from the resolved target.
- **One `--scene` JSON drives the whole frame** (`snapshot::SceneSpec`,
  `deny_unknown_fields`): a `simulation` section (datetime), a `camera` section,
  and an optional `ui` section (`Vec<ui::UiPanel>`). The output target
  (`--output`/`--width`/`--height`) stays as CLI flags, NOT in the JSON. A
  misspelled key at any level errors with exit 2 (the agent-debugging payoff of
  strict parsing).
- **Bodies-only by default; optional egui overlay when the scene has a `ui`.**
  `HeadlessRenderer` owns an `egui_wgpu::Renderer` and `render()` takes an
  `Option<UiFrame>`; when `Some`, panels composite over the scene exactly as in
  `Gfx::update` (apply `textures_delta.set`, `update_buffers`, `forget_lifetime`
  the pass, draw scene then egui, submit egui commands first, free deltas after).
  `snapshot::build_ui_frame` takes the already-parsed `Vec<ui::UiPanel>`,
  wraps it in `ui::PanelSet` — a `UiElement` tag enum over the bare instrument
  structs (which derive `Deserialize`), each cloned into an inert boxed
  `Instrument` — and drives it through the live `ui::control_panel`, so a mock
  renders identically to the real UI. **Two egui
  passes are required** (a throwaway warmup, then the real pass): egui lays out
  text + builds its font atlas lazily, so a single pass tessellates to nothing
  and the font-atlas texture delta lands on the warmup output — `build_ui_frame`
  merges the warmup's texture deltas into the second pass's. ppp = 1.0, so mock
  positions are in output pixels.
- Readback: `copy_texture_to_buffer` with 256-byte row alignment -> strip
  padding -> `image::RgbaImage::from_raw`.
