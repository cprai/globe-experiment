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
  `request_redraw()` — Windows does not deliver `RedrawRequested` to a hidden
  window, and the reveal code lives inside `redraw`.
- Window is created hidden and revealed right after the first present, so it
  appears with the scene already drawn.
- **`Occluded` first-frame guard**: macOS can report a still-hidden window as
  `Occluded`; if that happens before `shown`, show and retry rather than
  deadlocking invisible.
- winit's event loop must run on the main thread (macOS panics otherwise),
  and `resumed` can fire more than once (guarded by `if self.gfx.is_some()`).
- **WSL: force X11** (`build_event_loop` checks `WSL_DISTRO_NAME`) — WSLg's
  Wayland compositor drops EGL connections under GPU load (broken-pipe
  crash), and Wayland-without-Vulkan needs the X11 fallback to find any
  adapter at all.

## egui texture-delta ordering (easy to regress)

`textures_delta.set` must apply BEFORE the surface acquire in `Gfx::update`;
`free` deltas stay after present. egui emits each texture delta exactly once:
if a frame carrying a `set` exits early (`Occluded`/`Lost`/...) before the
apply, the delta is lost for good and the next partial update for that id
panics ("texture that has not been allocated"). A dropped `free` is benign.

## Render loop policy (continuous, owner-approved 2026-07-11)

The app renders unconditionally at the vsync rate (`ControlFlow::Wait`, each
presented frame requests the next; `AutoVsync` paces). A paused clock just
depicts a frozen instant. The **one brake is occlusion**: the `Occluded` arm
returns without requesting a redraw (an occluded acquire is unpaced by vsync
and would busy-spin). Do not reintroduce animation-gated redraws (see
`rejected.md`).

## `Gfx::init` device setup

- Passes `OwnedDisplayHandle` into the wgpu instance — required for the
  GLES/EGL backend to open its display; without it, systems with no Vulkan
  (WSL, some CI) panic with "no GPU adapter found".
- **Non-sRGB surface format** and **`present_mode = AutoVsync`** are both
  deliberate overrides of `get_default_config` (sRGB would break the shader's
  look calibration; Mailbox on DX12 judders).
- `Features::empty()` — see `constraints.md`.
- `SceneRenderer::new` decodes textures and compiles pipelines in parallel
  via rayon — the sanctioned design (the phase-1 `thread::scope` version was
  reverted; see `rejected.md`).

## Render frame

All positions the GPU sees are camera-target-local, subtracted on the CPU in
f64 — canonical statement in `camera.md`. `prepare` derives every body from
`RenderState.time` (`CelestialSphere::at`), rebuilds `view_proj` (and its
inverse) from the camera rig, and casts to f32 only when packing uniforms.

## Reversed-Z depth (canonical statement)

`Depth32Float` cleared to `0.0`, `depth_compare: Greater`, and a projection
that post-multiplies a Z-flip onto `perspective_rh` (near -> depth 1). Across
the huge near/far span (tens of km to >500,000 km) a forward-Z float buffer
has almost no precision near Luna — reversed-Z is load-bearing for
Terra-occludes-Luna. The clear value, compare op, and projection sign must
all agree or geometry vanishes. The far plane const is a **floor**: `prepare`
grows it to `max(FAR_PLANE_KM, |camera_pos| + 2*radius)` so a large orbited
body (gas giant at max zoom-out) is never clipped; a beyond-far body's
impostor depth is clamped instead of z-clipped.

Per-pass policy: the body impostor is the ONLY depth-writing pass (via
`frag_depth`); the orbit path is the only test-without-write pass; backdrop,
atmosphere, and markers neither write nor test. egui's overlay pipeline must
be built with the depth format to stay attachment-compatible.

## Impostor draw (all nine bodies, one pipeline)

`prepare` projects each body's center to screen space, picks
perspective/orthographic per frame by apparent angular size, fills the
same-system eclipse-occluder list, and computes a per-body Sol angular radius
(true penumbra softness). **Quad placement depends on the mode**: a distant
body gets a tight center-anchored quad, but a perspective (near/orbited)
body's quad is **full-screen** — its projected center can land far off-screen
at high tilt while the near surface still fills the frame, so a
center-anchored quad would carry the body off-screen with it. Draw order
among bodies is irrelevant (depth-tested).

Gates: the atmosphere draws only when a `has_atmosphere` body sits bit-exactly
at the render origin; orbit paths + markers only when the render origin is
Terra (their positions are Terra-frame). Everything else always draws.

## Orbit paths & markers

- Paths are recomputed every frame, no caching (batch SGP4 ~65 us, numerical
  ~0.4 ms per object).
- Path segments carry neighbor samples for **mitered, watertight joints** —
  any alpha-blended quad overlap (even AA fringes) beads at every joint.
- Sample points are radially lifted by `sec(pi/PATH_SEGMENTS)` so chord
  midpoints sit ON the true arc — inscribed chords dip ~0.5 km inside it and
  render as depth-test dashes where the path grazes the limb.
- Marker/path quads multiply the corner offset by `clip.w` before emitting,
  pre-compensating the perspective divide for constant pixel size; path
  vertices keep the centerline endpoint's clip z/w so the fat quad
  depth-tests as the thin 3D line.

## The `headless` binary

`src/headless.rs` (CLI + `--scene` JSON spec, `deny_unknown_fields` so typos
error) + `src/offscreen.rs` (surfaceless presenter + readback).

- Offscreen format is **`Rgba8Unorm` (non-sRGB), on purpose** — the stored
  bytes already equal the sRGB-encoded on-screen pixels, written verbatim to
  PNG. Do not "fix" it.
- **No EOP range check and no markers** — deliberate; out-of-range datetimes
  render and silently degrade (the caller owns the time).
- Mock UI: **two egui passes are required** (a throwaway warmup, then the
  real pass) — egui builds its font atlas lazily, so a single pass
  tessellates to nothing; the warmup's texture deltas merge into the real
  pass's. ppp = 1.0, so panel sizes are output pixels.
