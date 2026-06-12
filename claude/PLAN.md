# Google Earth Clone — Implementation Plan

A 3D globe viewer built on the existing iced 0.14 app, rendering the Earth with
wgpu inside an `iced::widget::shader` widget, with pan, tilt, and zoom controls.

## How iced and wgpu fit together

iced 0.14 ships a `shader` widget (enabled by the default `wgpu` feature) that
embeds custom GPU rendering into the widget tree. No windowing or surface code
is needed — iced owns the window, surface, and frame loop. We implement two traits:

- **`iced::widget::shader::Program<Message>`** — the widget's logic. Holds a
  `State`, receives mouse/keyboard events in `update()` (returning an
  `Action<Message>` to publish messages, capture events, or request redraws),
  and produces a `Primitive` in `draw()`.
- **`iced::widget::shader::Primitive`** — the GPU side. `prepare(pipeline,
  device, queue, bounds, viewport)` uploads uniforms/buffers each frame;
  `draw(pipeline, render_pass)` records draw calls into iced's existing render
  pass (preferred), or `render(...)` issues a separate pass with our own
  attachments. A `Pipeline` type shared by all primitive instances holds the
  long-lived wgpu objects (render pipeline, mesh buffers, textures).

iced re-exports its exact wgpu version as `iced::wgpu` (currently wgpu 27.0),
so we use that re-export and never declare wgpu in Cargo.toml — this avoids
version-mismatch type errors.

## Dependencies to add

| Crate | Purpose |
|-------|---------|
| `glam` | Camera math (Mat4, Vec3, quaternions) |
| `bytemuck` (derive) | Casting vertex/uniform structs to GPU byte buffers |
| `image` | Decoding the Earth texture (JPEG/PNG) at startup |

## Module layout

```
src/
  main.rs           iced app: state, Message, update, view (hosts the shader widget)
  globe/
    mod.rs          shader::Program impl — translates input events to Messages
    camera.rs       orbital camera model + matrix computation
    mesh.rs         UV-sphere generation (positions, UVs, indices)
    pipeline.rs     shader::Primitive + Pipeline impls — all wgpu objects
shaders/
  globe.wgsl        vertex + fragment shader
assets/
  earth.jpg         equirectangular Earth texture (see Assets below)
```

## Camera model

Google Earth's camera is an **orbital camera** anchored to a look-at point on
the globe surface, not a free-fly camera. State:

| Field | Meaning | Range/clamp |
|-------|---------|-------------|
| `longitude`, `latitude` | look-at point on the sphere | lat clamped to ±89° |
| `distance` | camera distance from the look-at point | min ~1.01×R, max ~10×R |
| `tilt` | angle off straight-down (nadir) | 0°–80° |
| `heading` | compass rotation around the look-at point | free (stretch goal) |

Per frame, derive `view` and `projection` (perspective, ~45° FOV, aspect from
widget bounds) with glam and upload `view_proj` as a uniform. The Earth is a
unit sphere at the origin; the camera moves around it.

Control feel notes:
- **Pan sensitivity scales with altitude** — degrees-per-pixel proportional to
  `distance - R`, so panning feels constant-speed at any zoom level.
- **Zoom is exponential** — each scroll tick multiplies `distance - R` by a
  factor (e.g. 0.9), giving smooth approach without overshooting the surface.

## Input mapping

Handled in `Program::update()`, which receives `shader::Event` + cursor:

| Input | Action |
|-------|--------|
| Left mouse drag | Pan: drag right moves view west (globe follows cursor) |
| Scroll wheel | Zoom in/out toward the look-at point |
| Right mouse drag (or Ctrl+left drag) | Vertical: tilt; horizontal: heading |

Architecture decision: the camera lives in the **app state** (`main.rs`), not
in the widget's internal `State`. `Program::update()` only tracks the drag
gesture (button held, last cursor position) in widget state and emits messages
like `Message::Pan { dx, dy }`, `Message::Zoom(delta)`, `Message::Tilt(delta)`.
The app's `update()` applies them to the camera. This keeps all mutable domain
state in one place, makes a HUD (coordinates/altitude readout) trivial, and
follows iced's data flow.

## Milestones

### 1. Shader widget scaffolding ("red triangle")
- Add `glam`, `bytemuck`, `image`.
- Implement a minimal `Program`/`Primitive`/`Pipeline` that clears nothing and
  draws a hardcoded colored triangle via `draw()` into iced's render pass.
- Mount it in `view()` with `.width(Fill).height(Fill)`.
- **Done when:** triangle renders inside the iced window and resizes with it.

### 2. Sphere mesh + camera matrices
- `mesh.rs`: generate a UV sphere (~64 stacks × 128 slices) with position +
  UV per vertex; upload vertex/index buffers once in `Pipeline::new`.
- `camera.rs`: orbital camera (fixed initial pose), `view_proj` uniform buffer
  updated in `prepare()`.
- Enable back-face culling. **No depth buffer needed** — a single convex
  sphere with culling renders correctly without one, which lets us stay in
  iced's shared render pass (which doesn't expose a depth attachment).
- Fragment shader: shade by normal or UV gradient to verify geometry.
- **Done when:** a correctly proportioned sphere is visible from a sane
  starting pose (e.g. above 0°N 0°E, distance 3×R).

### 3. Earth texture
- Load `assets/earth.jpg` (equirectangular) with `image`, upload as
  `Rgba8UnormSrgb` texture with mipmaps; sample in the fragment shader using
  the sphere's UVs (longitude → U, latitude → V).
- Address mode: repeat on U (dateline seam), clamp on V (poles).
- **Done when:** recognizable Earth with no visible seam at the dateline.

### 4. Pan / tilt / zoom controls
- Implement the input mapping and Message flow described above.
- Clamp distance and tilt; wrap longitude; clamp latitude.
- Return `Action::request_redraw()` (and capture the event) whenever the
  camera changes, so the scene only re-renders on interaction.
- **Done when:** can fly from a full-globe view down to country level and tilt
  to see the horizon, with cursor-stable, non-jumpy motion.

### 5. Polish
- Simple sun lighting (N·L plus small ambient) and a subtle atmosphere rim
  glow (fresnel term or a slightly larger translucent shell).
- Dark space / starfield background.
- Optional: pan inertia (decay velocity after release — needs a
  `window::frames()`-style redraw subscription while animating).
- HUD overlay in iced (lat/lon/altitude text) — trivial since camera state is
  in the app.

### 6. Stretch goals (out of scope for v1)
- **Map tile streaming**: quadtree LOD with web-mercator tiles (OSM or similar),
  async fetch via `Task`, cache, render onto sphere patches. This is the big
  step from "textured globe" to "Google Earth".
- Terrain elevation (heightmap displacement), city labels, search/fly-to
  animation.

## Assets

NASA **Blue Marble Next Generation** (public domain) equirectangular imagery:
https://visibleearth.nasa.gov/collection/1484/blue-marble
Use the 5400×2700 JPEG for v1 — comfortably under wgpu's guaranteed 8192px
texture limit. (The 21600×10800 originals would need tiling — stretch goal.)
Download manually into `assets/earth.jpg`; don't commit large originals.

## Risks / technical notes

- **wgpu version coupling**: always use `iced::wgpu`; adding `wgpu` to
  Cargo.toml independently will eventually drift and produce confusing
  cross-version type errors.
- **Redraw discipline**: iced only redraws on events by default. Camera
  changes must explicitly request redraws; continuous animation (inertia)
  needs a frame subscription gated on "is animating" to avoid 100% GPU usage.
- **WSL2 GPU**: wgpu under WSLg may fall back to a software Vulkan rasterizer
  (llvmpipe) depending on driver setup. A 5400×2700 textured sphere should
  still be fine; if performance is bad, test natively on Windows or force the
  GL backend via `WGPU_BACKEND=gl`.
- **sRGB/mipmaps**: use an sRGB texture format and generate mipmaps (or accept
  shimmer at far zoom for v1); wgpu doesn't auto-generate mipmaps, so v1 can
  ship with a single mip and a note to add a mip-gen pass later.
- **Event capture**: the shader widget must capture drag/scroll events it
  handles so the rest of the UI doesn't also react, but let unrelated events
  pass through.
