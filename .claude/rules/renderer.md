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

`create_shader_module` runs on one rayon task while the 4 group-0 texture
inputs (3 LUTs + stars) load in parallel via `into_par_iter` (the 12
impostor-body maps - 9 albedos + Terra's night/normal/specular - decode in a
separate `par_iter` into group-1 bind groups). A nested `rayon::join` compiles
the 4 group-0 render pipelines concurrently (atmosphere, stars, markers, orbit
path); the single **body impostor pipeline** (the 5th) is built after the join
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
it draws in local coordinates. For Terra/Luna (`render_origin` = Terra's
heliocentric center) the render frame is the Terra-centered (old geocentric)
frame - the `CelestialSphere` is heliocentric, but subtracting Terra's center
undoes that, keeping the Terra render frame bit-identical. `prepare` also rebuilds `view_proj` (and its
inverse) from the camera rig via `view_proj_reversed_z`.

## Uniforms struct layout (must match Rust `Uniforms` and WGSL `Uniforms`)

```
view_proj:       mat4x4<f32>         // rebuilt in prepare (view_proj_reversed_z)
inv_view_proj:   mat4x4<f32>         // per-fragment eye rays (impostor
                                     //   perspective trace, atmosphere, stars)
camera_pos:      vec3<f32> + 1 f32 pad (_pad0)   // eye, render frame km
star_rot_inv:    mat3x3<f32>         // Rust: 3 columns each padded to [f32;4]
marker:          vec4<f32>           // x,y = viewport px; z = radius px; w = unused
luna_occluder:   vec4<f32>           // xyz = Luna center render-frame km;
                                     //   w = Luna radius km (fs_atmosphere only)
sol_pos:         vec3<f32> + 1 f32 pad (_pad4)   // Sol, render frame km; lights every
                                     //   body + aims the backdrop disc
atmosphere_quad: vec4<f32>           // xy = NDC center, zw = NDC half-extent of
                                     //   the atmosphere quad ((0,0,1,1) = full
                                     //   screen, the usual case)
```

Key: WGSL `mat3x3` columns have `vec4` stride, so the Rust struct pads each
column to 4 floats. Per-marker position + visibility live in the **marker
instance buffer** (`MarkerInstance { position: vec3, visible: f32 }`), not
in the uniform block. The uniform and marker instances are written every frame
in `prepare` (`queue.write_buffer`). The group-0 `luna_occluder` exists for
the ONE pass that must know about Luna without drawing it - the atmosphere's
Luna occlusion check (`fs_atmosphere`) - sourced in `prepare` from the Luna
entry of the derived `CelestialSphere::at(time).bodies`, the occluder radius
from the identity (`CelestialBody::LUNA.mean_radius_km()`). (Luna shadowing
Terra - the solar-eclipse spot - rides the generic per-body occluder list.)

**Impostor bodies (group 1) — ALL NINE (Terra + 7 planets + Luna), single
shader impostor, no mesh.** Each body has a
per-body `PlanetUniform` (`rot` mat3x3 cols->vec4 + `pos` vec3 (render frame) +
`sol_angular_radius` + `ndc_center` vec2 + `ndc_half_extent` vec2 +
`radii` vec3 (triaxial semi-axes km; rx = rz spheroids for Terra/planets, all
three distinct for Luna) + `depth` + `occluders` (`[vec4; MAX_OCCLUDERS]`: xyz
= same-system occluder center render-frame km, w = caster sphere radius km, 0
= unused slot) + `perspective` + `flags` (the `BODY_FLAG_*` shading-feature
bits, packed from the body's `planet::Maps` + `has_atmosphere` - NIGHT /
NORMAL_MAP / SPECULAR / ATMO_LIT; Terra = all four, every other body = none))
and a group-1 bind group (uniform + albedo + optional night/normal/specular
maps - shared 1x1 dummies when absent - + the shared sampler). `prepare`
walks the entries of the derived `bodies`, every one with a GPU slot in
`IMPOSTOR_BODIES` (= `planet::ALL`, then Luna, then Terra - every body draws
from every vantage), **projects the center to screen space** (NDC center +
quad half-extent + reversed-Z depth), picks the trace mode by apparent
angular size of the largest semi-axis (`perspective` flag), fills the occluder
list from `CelestialBody::same_system` (Terra shadowing Luna = the lunar
eclipse, Luna shadowing Terra = the solar-eclipse spot; a future moon system
self-shadows by adding its enum arm there), and
computes the per-body Sol angular radius (`asin(SOL_RADIUS_KM / Sol
distance)`, the penumbra softness). The quad is placed per mode: a distant
(orthographic) body's quad is anchored at the projected center sized to the
angular radius; a near/orbited (perspective) body's quad is **full-screen**
(`[-1,1]^2`), because its projected center can land far off-screen at high tilt
while the near surface still fills the frame - a center-anchored quad would
follow the center off-screen and the body would vanish. All draw through one
shared pipeline (`vs_planet`/`fs_planet`, layout `[group0, group1]`, no vertex
buffer, solid-body reversed-Z depth write + `Greater` - the scene's ONLY
depth-writing pass): the fragment shader
ray-traces the triaxial ellipsoid — perspective (eye-ray via `inv_view_proj`)
for
near bodies, orthographic (parallel-ray) for far ones — writes per-fragment
depth, and shades per the feature flags (plain hard-terminator Lambert for a
bare-albedo body, up to the full Terra look: tangent-space normal map, GGX
ocean specular + wave shimmer, transmittance-tinted sunlight + day/night
blend, emissive city lights with the surface-anchored dither dissolve). The
draw list `planet_draw_indices` (bodies whose center projects in front of the
camera) is rebuilt each `prepare`; order is irrelevant (depth-tested). All
body maps live ONLY in group 1 (4 sampled textures per draw), and group 0
holds 4 — worst stage 8, clear of the portable 16-per-stage limit.

**Gates.** `prepare` sets `draw_atmosphere` (a `has_atmosphere` body sits
bit-exactly at the render origin — Terra under a Terra/Luna target today) and
computes the `atmosphere_quad` placement (an orthographic-impostor-style quad
over the top-of-atmosphere silhouette; full-screen when the camera is
inside/near the shell or the shell is perspective-sized, which at current
camera limits is always). It also sets `draw_satellite_overlays = (origin ==
ZERO)` for the orbit paths + markers, whose positions are Terra-frame world
coordinates. **Draw order: stars -> body impostors (all nine) -> atmosphere
-> orbit paths -> markers** (the depth buffer keeps any body behind another
hidden; the paths draw before the markers so each dot sits on its own line).

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
and the draw is skipped; the paths/markers also gate on
`draw_satellite_overlays` (render origin at Terra).

## Bind group 0 layout

| binding | resource | format / notes |
|---|---|---|
| 0 | uniforms | visibility VERTEX_FRAGMENT |
| 1 | `map_sampler` | repeat U (dateline seam), clamp V (poles), linear |
| 2 | transmittance LUT | `Rgba16Float` 256x64 |
| 3 | `lut_sampler` | clamp both, linear |
| 4 | inscatter Rayleigh LUT | `Rgba16Float` 256x128 |
| 5 | inscatter Mie LUT | `Rgba16Float` 256x128 |
| 6 | stars texture | `Rgba8UnormSrgb` (decoded JPEG) |

`map_sampler` is shared by the stars map and (as group 1's sampler slot) every
body map; `lut_sampler` by the three LUTs. LUTs are read with
`textureSampleLevel(..., 0.0)` (used in non-uniform control flow; no mips
anyway).

## Bind group 1 layout (impostor bodies: Terra + 7 planets + Luna)

Used by the single body impostor pipeline; one bind group per body
(`IMPOSTOR_BODIES` order: `planet::ALL`, Luna, Terra), set per draw.

| binding | resource | notes |
|---|---|---|
| 0 | per-body `PlanetUniform` | VERTEX_FRAGMENT; `rot` + `pos` (render frame) + `sol_angular_radius` + `ndc_center` + `ndc_half_extent` + `radii` (triaxial km) + `depth` + `occluders[MAX_OCCLUDERS]` + `perspective` + `flags` (BODY_FLAG_*) |
| 1 | albedo map | `Rgba8UnormSrgb` (8K for Terra/inner/gas/Luna, 2K for ice giants) |
| 2 | sampler | the shared `map_sampler` (repeat U / clamp V) |
| 3 | night map | `Rgba8UnormSrgb`; Terra only (shared 1x1 black dummy otherwise) |
| 4 | normal map | **`Rgba8Unorm`** (linear — data, not color; sRGB decode would warp the tangent vectors); Terra only (1x1 flat dummy otherwise) |
| 5 | specular mask | `Rgba8Unorm` (linear), `.r` = water; Terra only (1x1 black dummy otherwise) |

The dummies are never sampled (the matching `flags` bit is clear), they only
satisfy the fixed layout.

## Depth buffer

`Depth32Float`, reversed-Z (see `shader.md` and `camera.md`). `Gfx` owns a
`depth_view` recreated on resize; `OffscreenRenderer` owns one sized to its
target. Shared helpers `create_depth_view` + `depth_attachment` (cleared to
`0.0`) build both. All five scene pipelines declare `depth_stencil` (the
body impostor writes `@builtin(frag_depth)` and is the ONLY depth-writing
pass; the orbit path is the only test-without-write pass); egui's overlay
pipeline is built with `depth_stencil_format: Some(DEPTH_FORMAT)`.

## Renderer constants

`MARKER_RADIUS_PX 6`, orbit path `PATH_SEGMENTS 256` /
`INITIAL_PATH_CAPACITY 512` / `PATH_FADE_START 0.85`,
`DEPTH_FORMAT Depth32Float`,
`SOL_RADIUS_KM 695700` (per-impostor-body Sol angular radius =
`asin(SOL_RADIUS_KM / dist)`; every eclipse penumbra, Terra's included, uses
it), `ATMOSPHERE_TOP_KM 6460` (the CPU twin of the shader/bake constant — the
three must stay in sync; sizes the atmosphere quad), projection consts
`FOV_Y_DEG 45` / `NEAR_PLANE_RADII 0.01` / `FAR_PLANE_KM 500000` (far-plane
*floor*; `prepare` grows the actual far plane to `max(FAR_PLANE_KM,
|camera_pos| + 2*radius)` so a large orbited body is never clipped), body
impostor `IMPOSTOR_BODIES` (planet::ALL + Luna + Terra, the GPU-slot order) /
`PLANET_PERSPECTIVE_MIN_ARCSEC 1800` / `PLANET_QUAD_MARGIN 1.3` /
`PLANET_MIN_DEPTH 1e-6` (clamps a beyond-far body's depth so it is not
z-clipped) / `MAX_OCCLUDERS 4` (eclipse-occluder slots, must match
`scene.wgsl`) / `BODY_FLAG_*` (shading-feature bits, must match `scene.wgsl`).

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
