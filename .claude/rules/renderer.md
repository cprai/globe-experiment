---
paths:
  - "src/engine/renderer/**/*.rs"
  - "src/engine/application/mod.rs"
  - "src/engine/application/gfx.rs"
  - "src/offscreen.rs"
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

`create_shader_module` runs on one rayon task while 8 group-0 texture inputs
load in parallel via `into_par_iter` (the 8 impostor-body textures - 7 planets
+ Luna - decode in a separate
`par_iter` into group-1 bind groups). A nested `rayon::join` compiles the 5
group-0 render pipelines concurrently (Terra surface, atmosphere, stars,
markers, orbit path); the single **body impostor pipeline** (a 6th) is built
after the join
(it borrows `planet_layout`). This is the **sanctioned** parallel decode — do
not confuse it with the phase-1 reverted `thread::scope` approach.

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
to the camera target's center.** `prepare` first derives the scene from the
frame's time (`CelestialSphere::at(RenderState.time)` — Sol, Luna, the 7
planets, star matrices), then subtracts the origin
(`RenderState.camera_target.render_origin(&celestial)`) on the CPU from each absolute body
position before upload, so the GPU never sees an absolute world position (the
far-planet f32-jitter fix). There is **no `render_origin` uniform and no
`sol_dir`** — the shader is purely local and derives every Sol direction from
`sol_pos`. The orbited body's position is a bit-exact zero (`pos - origin`), so
it draws in local coordinates. For Terra/Luna (`render_origin == 0`) the render
frame is the absolute frame. `prepare` also rebuilds `view_proj` (and its
inverse) from the camera rig via `view_proj_reversed_z`.

## Uniforms struct layout (must match Rust `Uniforms` and WGSL `Uniforms`)

```
view_proj:     mat4x4<f32>           // rebuilt in prepare (view_proj_reversed_z)
inv_view_proj: mat4x4<f32>           // for the body impostor's perspective ray
camera_pos:    vec3<f32> + 1 f32 pad (_pad0)   // eye, render frame km
star_rot_inv:  mat3x3<f32>           // Rust: 3 columns each padded to [f32;4]
marker:        vec4<f32>             // x,y = viewport px; z = radius px; w = unused
luna_pos:      vec3<f32> + 1 f32 pad (_pad2)   // Luna center, render frame km
luna_params:   vec4<f32>             // x = Luna radius km; y = Terra radius km;
                                     //   z = Sol angular radius rad; w = unused
sol_pos:       vec3<f32> + 1 f32 pad (_pad4)   // Sol, render frame km; lights every
                                     //   body + aims the backdrop disc
```

Key: WGSL `mat3x3` columns have `vec4` stride, so the Rust struct pads each
column to 4 floats. Per-marker position + visibility live in the **marker
instance buffer** (`MarkerInstance { position: vec3, visible: f32 }`), not
in the uniform block. The uniform and marker instances are written every frame
in `prepare` (`queue.write_buffer`). Luna has NO mesh and no `luna_rot`
uniform: it draws through the shared body impostor (below). The group-0
`luna_pos`/`luna_params` exist for the two passes that must know about Luna
without drawing it - the solar-eclipse shadow on Terra (`fs_main`) and the
atmosphere's Luna occlusion check (`fs_atmosphere`) - sourced in `prepare`
from the Luna entry of the derived `CelestialSphere::at(time).bodies`
(`TerraSystem(Luna)`), the shadow-caster radius from the identity
(`CelestialBody::LUNA.mean_radius_km()`).

**Impostor bodies (group 1) — the 7 planets + Luna, single shader impostor,
no mesh.** Each body has a
per-body `PlanetUniform` (`rot` mat3x3 cols->vec4 + `pos` vec3 (render frame) +
`sol_angular_radius` + `ndc_center` vec2 + `ndc_half_extent` vec2 +
`radii` vec3 (triaxial semi-axes km; rx = rz for a planet, all three distinct
for Luna) + `depth` + `occluders` (`[vec4; MAX_OCCLUDERS]`: xyz = same-system
occluder center render-frame km, w = caster sphere radius km, 0 = unused
slot) + `perspective`) and a group-1 bind group (uniform +
texture + the shared sampler). `prepare` walks the entries of the derived
`bodies` with a GPU slot in `IMPOSTOR_BODIES` (= `planet::ALL` then Luna;
Terra is the only meshed body and has no slot; **Luna's slot is skipped unless
`draw_terra_system`**), **projects the center to screen space** (NDC center +
quad half-extent + reversed-Z depth), picks the trace mode by apparent
angular size of the largest semi-axis (`perspective` flag), fills the occluder
list from `CelestialBody::same_system` (Terra shadowing Luna = the lunar
eclipse; a future moon system self-shadows by adding its enum arm there), and
computes the per-body Sol angular radius (`asin(SOL_RADIUS_KM / Sol
distance)`, the penumbra softness). The quad is placed per mode: a distant
(orthographic) body's quad is anchored at the projected center sized to the
angular radius; a near/orbited (perspective) body's quad is **full-screen**
(`[-1,1]^2`), because its projected center can land far off-screen at high tilt
while the near surface still fills the frame - a center-anchored quad would
follow the center off-screen and the body would vanish. All draw through one
shared pipeline (`vs_planet`/`fs_planet`, layout `[group0, group1]`, no vertex
buffer, solid-body reversed-Z depth write + `Greater`): the fragment shader
ray-traces the triaxial ellipsoid — perspective (eye-ray via `inv_view_proj`)
for
near bodies, orthographic (parallel-ray) for far ones — and writes per-fragment
depth. The
draw list `planet_draw_indices` (bodies whose center projects in front of the
camera) is rebuilt each `prepare`; order is irrelevant (depth-tested). The 8
body textures live ONLY in group 1, so group 0 stays at 8 sampled textures —
clear of the portable 16-per-stage limit.

**Terra-system gate.** `prepare` sets `draw_terra_system = (origin == ZERO)`;
`render` draws the Terra surface, atmosphere, orbit paths, and markers
only when true
(orbiting Terra/Luna), and the impostor loop includes Luna's slot only when
true. Orbiting a planet they are skipped. **Draw order: stars
-> body impostors (planets + Luna) -> Terra surface -> atmosphere -> orbit
paths ->
markers** (the
Terra-system ones gated; the depth buffer keeps Luna / a planet behind Terra
hidden;
the paths draw before the markers so each dot sits on its own line).

**Predicted orbit paths (`path_pipeline`, `vs_path`/`fs_path`).** For every
`RenderState.markers` entry, `prepare` propagates the marker's
`satellite::Propagation` one orbital period ahead
(`satellite::orbit_path_inertial`, dispatching per object: `Sgp4` = one batch
`sgp4` call ~65 us, `Numerical` = one satkit `orbitprop` integration + one
dense-output `interp_batch` ~0.4 ms — cheap enough to recompute every frame,
no caching; a scene may mix kinds) over `PATH_SEGMENTS` segments and writes
per-segment instances (`PathInstance`:
prev / p0+alpha / p1+alpha / next, four vec4s) into a grow-on-demand instance
buffer (`paths`, marker pattern). The vertex shader expands each segment to a
constant-pixel-width screen-space quad (the marker `clip.w` trick) with
**mitered joints** — each instance carries its neighbor samples so both quads
at a joint offset the shared endpoint identically: watertight, zero overlap
(any alpha-blended overlap, even just the AA fringes, reads as a brighter bead
at every joint). Vertices keep the CENTERLINE endpoint's clip z/w, so the quad
depth-tests as the thin 3D line. The pipeline is the one depth
**test-without-write** pass (`Greater`, no write): solids occlude the path's
far side, the translucent line occludes nothing. Sample points are radially
lifted by `sec(pi/PATH_SEGMENTS)` in `prepare` so chord midpoints sit ON the
true arc — inscribed chords dip ~0.5 km inside it and render as depth-test
dashes where the path grazes Terra's limb. The fade tail (`path_fade`,
CPU-side per-endpoint alpha) holds full opacity until `PATH_FADE_START` of the
period, then smoothsteps to zero at one full period. Recomputed every frame
(a paused app renders zero frames, so idle stays free). Empty markers
(eclipse/solar_system scenarios, the headless binary) mean `path_count == 0`
and the draw is skipped.

## Bind group 0 layout

| binding | resource | format / notes |
|---|---|---|
| 0 | uniforms | visibility VERTEX_FRAGMENT |
| 1 | day texture | `Rgba8UnormSrgb`, 8192x4096 (decoded JPEG) |
| 2 | `terra_sampler` | repeat U (dateline seam), clamp V (poles), linear |
| 3 | night texture | `Rgba8UnormSrgb` (decoded JPEG) |
| 4 | normal map | **`Rgba8Unorm`** (linear — data, not color; decoded TIFF) |
| 5 | specular mask | `Rgba8Unorm` (linear), `.r` = water (decoded TIFF) |
| 6 | transmittance LUT | `Rgba16Float` 256x64 |
| 7 | `lut_sampler` | clamp both, linear |
| 8 | inscatter Rayleigh LUT | `Rgba16Float` 256x128 |
| 9 | inscatter Mie LUT | `Rgba16Float` 256x128 |
| 10 | stars texture | `Rgba8UnormSrgb` (decoded JPEG) |

`terra_sampler` is shared by all image textures including stars and the
impostor bodies; `lut_sampler` by the three LUTs. LUTs are read with
`textureSampleLevel(..., 0.0)` (used in non-uniform control flow; no mips
anyway). Normal map linear format is load-bearing — sRGB decode would warp the
tangent vectors.

## Bind group 1 layout (impostor bodies: 7 planets + Luna)

Used by the single body impostor pipeline; one bind group per body
(`IMPOSTOR_BODIES` order: `planet::ALL` then Luna), set per draw.

| binding | resource | notes |
|---|---|---|
| 0 | per-body `PlanetUniform` | VERTEX_FRAGMENT; `rot` + `pos` (render frame) + `sol_angular_radius` + `ndc_center` + `ndc_half_extent` + `radii` (triaxial km) + `depth` + `occluders[MAX_OCCLUDERS]` + `perspective` |
| 1 | body texture | `Rgba8UnormSrgb` (8K for inner/gas/Luna, 2K for ice giants) |
| 2 | sampler | the shared `terra_sampler` (repeat U / clamp V) |

## Depth buffer

`Depth32Float`, reversed-Z (see `shader.md` and `camera.md`). `Gfx` owns a
`depth_view` recreated on resize; `OffscreenRenderer` owns one sized to its
target. Shared helpers `create_depth_view` + `depth_attachment` (cleared to
`0.0`) build both. All six scene pipelines declare `depth_stencil` (the
body impostor writes `@builtin(frag_depth)`; the orbit path is the only
test-without-write pass); egui's overlay pipeline is built with
`depth_stencil_format: Some(DEPTH_FORMAT)`.

## Renderer constants

`STACKS 64`, `SLICES 128` (mesh resolution; the Terra mesh only — Luna and the
planets are
impostors), `MARKER_RADIUS_PX 6`, orbit path `PATH_SEGMENTS 256` /
`INITIAL_PATH_CAPACITY 512` / `PATH_FADE_START 0.85`,
`DEPTH_FORMAT Depth32Float`,
`SOL_ANGULAR_RADIUS_RAD 0.004652` (fs_main solar-eclipse penumbra width),
`SOL_RADIUS_KM 695700` (per-impostor-body Sol angular radius), projection
consts
`FOV_Y_DEG 45` / `NEAR_PLANE_RADII 0.01` / `FAR_PLANE_KM 500000` (far-plane
*floor*; `prepare` grows the actual far plane to `max(FAR_PLANE_KM,
|camera_pos| + 2*radius)` so a large orbited body is never clipped), body
impostor `IMPOSTOR_BODIES` (planet::ALL + Luna) /
`PLANET_PERSPECTIVE_MIN_ARCSEC 1800` / `PLANET_QUAD_MARGIN 1.3` /
`PLANET_MIN_DEPTH 1e-6` (clamps a beyond-far body's depth so it is not
z-clipped) / `MAX_OCCLUDERS 4` (eclipse-occluder slots, must match
`scene.wgsl`).

## The `headless` binary (`OffscreenRenderer` + `src/headless.rs`)

The single-frame render mode is its own binary (`cargo run --release --bin
headless -- --scene ... --output frame.png`) over the shared `engine` (no
`scenarios`; it calls none of the winit code); `src/offscreen.rs` is its
presenter, `src/headless.rs` its bin root (CLI + scene spec + mock-UI
driving). See `architecture.md`.

- Offscreen format is **`Rgba8Unorm` (non-sRGB), on purpose** — twin of the
  surface format rule. The stored bytes already equal the sRGB-encoded
  on-screen pixels; written verbatim to PNG.
- Shares `SceneRenderer` and `request_adapter_device` with the windowed path
  (both `pub(crate)` in `renderer`). `OffscreenRenderer` passes
  `compatible_surface: None` to the adapter.
- **No EOP range check** in the headless binary. Out-of-range datetimes render
  and silently degrade. This is deliberate — documented in `headless.rs` and
  `scenarios.md`.
- **No markers** in the headless binary (`RenderState.markers` is empty — so no
  predicted orbit paths either). The renderer
  derives every body from `RenderState.time`, so `camera.target` can be any of
  `"terra"`, `"luna"`, or a planet (`"mars"`, ..., `"neptune"`); the render
  origin is taken from the resolved `camera_target`.
- **One `--scene` JSON drives the whole frame** (`headless::SceneSpec`,
  `deny_unknown_fields`): a `simulation` section (datetime), a `camera` section,
  and an optional `ui` section (`Vec<ui::UiPanel>`). The output target
  (`--output`/`--width`/`--height`) stays as CLI flags, NOT in the JSON. A
  misspelled key at any level errors with exit 2 (the agent-debugging payoff of
  strict parsing).
- **Bodies-only by default; optional egui overlay when the scene has a `ui`.**
  `OffscreenRenderer` owns an `egui_wgpu::Renderer` and `render()` takes an
  `Option<UiFrame>`; when `Some`, panels composite over the scene exactly as in
  `Gfx::update` (apply `textures_delta.set`, `update_buffers`, `forget_lifetime`
  the pass, draw scene then egui, submit egui commands first, free deltas after).
  `headless.rs`'s `build_ui_frame` takes the already-parsed `Vec<ui::UiPanel>`,
  wraps it in `ui::PanelSet` — a `UiElement` tag enum over the bare instrument
  structs (which derive `Deserialize`), each cloned into an inert boxed
  `Instrument` — and drives it through the live `ui::control_panel`, so a mock
  renders identically to the real UI. **Two egui
  passes are required** (a throwaway warmup, then the real pass): egui lays out
  text + builds its font atlas lazily, so a single pass tessellates to nothing
  and the font-atlas texture delta lands on the warmup output — `build_ui_frame`
  merges the warmup's texture deltas into the second pass's. (The warmup also
  seeds egui_taffy's layout cache, and egui runs its own discard pass within
  `run_ui` - `install_theme` sets `max_passes = 2` - so the taffy layout is
  settled by the real pass.) ppp = 1.0, so mock panel sizes are in output
  pixels.
- Readback: `copy_texture_to_buffer` with 256-byte row alignment -> strip
  padding -> `image::RgbaImage::from_raw`.
