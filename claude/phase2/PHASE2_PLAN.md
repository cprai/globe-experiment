# PHASE2 — Remove iced: raw wgpu + winit event loop + egui overlay

Goal: delete the iced dependency entirely and run the existing globe
renderer on a self-owned stack — winit window/event loop, raw wgpu
surface and render loop, mouse controls reimplemented on winit events,
and the two sun sliders rebuilt with egui via egui-wgpu/egui-winit.

Prerequisite reading: `claude/phase1/PHASE1.md` is the canonical
technical context for everything being ported. This plan only describes
the *migration*; the rendering design itself does not change.

## What stays exactly as it is

These files have zero iced coupling and are ported untouched (only
`use` paths change from `iced::wgpu` to `wgpu`):

- `src/globe/camera.rs` — orbital camera (glam only).
- `src/globe/sun.rs` — subsolar point + star rotation (glam only).
- `src/globe/mesh.rs` — UV sphere (bytemuck only).
- `src/globe/atmosphere.rs` — LUT bake (half only).
- `shaders/globe.wgsl` — all shader code, unchanged.
- `build.rs` + `assets/` flow — texture download, `include_bytes!`
  embedding, sequential loading. **Loading stays sequential** — the
  parallel version was deliberately reverted by the owner; do not
  reintroduce it.
- The three-pass, no-depth-buffer draw order (stars → surface →
  atmosphere) and every shader constant / look-tuning value.

Behavioral invariants to preserve:

- Idle = zero GPU work. The new event loop uses `ControlFlow::Wait`;
  frames are driven by `request_redraw()` only when input changes the
  camera, inertia is coasting, or egui wants a repaint. No unconditional
  render loop spinning at vsync.
- Input feel: identical pan/zoom/tilt mapping, flick-inertia constants,
  cursor-stable panning, grab cursor icons.
- Visual output: pixel-identical globe (same uniforms, same passes,
  same sRGB surface handling).

## Target dependency set (checked against crates.io, 2026-06-11)

| Crate | Version | Notes |
|---|---|---|
| `wgpu` | 29 | Now a direct dependency — no longer iced's re-export. egui-wgpu 0.34 requires `wgpu ^29.0.1`, so wgpu 29 is the forced choice. |
| `winit` | 0.30 | egui-winit 0.34 requires `^0.30.13`. |
| `egui` | 0.34.3 | Sliders + panel. |
| `egui-wgpu` | 0.34.3 | Paints egui primitives into our render pass. |
| `egui-winit` | 0.34.3 | Translates winit events into egui input. |
| `pollster` | 0.4 | Blocks on the async wgpu adapter/device request at startup. |
| `iced` | **removed** | |

Kept: `glam`, `bytemuck`, `half`, `image`, `ureq` (build), and all the
`[profile.dev.package.*] opt-level = 3` decode overrides.

**Migration note:** the renderer code is written against wgpu 27 (iced's
re-export). wgpu 27 → 29 has API churn (instance/surface creation,
descriptor field changes, render-pass lifetimes). Strategy: port the
code as-is and fix compiler errors against the wgpu 29 docs; the
pipeline/bind-group/texture concepts are unchanged. One consequence of
owning the device: iced's `Features::empty()` limitation is gone — we
still request empty features for parity, but BC7/compressed textures
become *possible* later (out of scope here).

## Target module layout

```
src/main.rs            winit entry: ApplicationHandler, event loop,
                       wgpu instance/surface/device, frame loop,
                       egui integration, redraw policy
src/globe/mod.rs       module re-exports only (no logic, no iced)
src/globe/renderer.rs  was pipeline.rs: GlobeRenderer { new, prepare,
                       render } — same wgpu objects, no iced traits
src/globe/input.rs     was mod.rs logic: Controller — drag tracking,
                       velocity EMA, flick inertia; mutates Camera
                       directly (the Interaction message enum dies)
src/globe/camera.rs    unchanged
src/globe/sun.rs       unchanged
src/globe/mesh.rs      unchanged
src/globe/atmosphere.rs unchanged
src/ui.rs              egui sun panel (two sliders + labels)
```

Architecture changes vs. phase 1:

- **No message indirection.** iced's `Program::update → publish →
  app::update` loop existed because camera state lived in the app and
  the widget couldn't touch it. Now everything lives in one `App`
  struct, so `input::Controller` applies pan/zoom/tilt to `&mut Camera`
  directly and returns whether a redraw is needed.
- **`GlobeRenderer` replaces `shader::Primitive`/`shader::Pipeline`.**
  - `GlobeRenderer::new(device, queue, surface_format)` — verbatim port
    of `Pipeline::new` (textures, LUT bake, bind group, 3 pipelines).
    Called eagerly at startup, not lazily on first frame.
  - `prepare(queue, camera, sun, aspect)` — the uniform write, was
    `Primitive::prepare`.
  - `render(&self, pass: &mut wgpu::RenderPass)` — the three
    draw_indexed calls, was `Primitive::draw`.
- **One render pass per frame**: clear → stars → globe → atmosphere →
  egui. egui-wgpu paints into an existing pass (it needs a
  `RenderPass<'static>`; `forget_lifetime()` is the documented pattern).
- **Event routing order**: every `WindowEvent` goes to
  `egui_winit::State::on_window_event` first; if egui consumed it
  (slider drag, pointer over panel), the globe controller never sees
  it. Otherwise it goes to the controller. This replaces iced's
  `stack![]` overlay event capture.
- **Redraw policy** (the "independent event loop for animation"):
  `ControlFlow::Wait`; `window.request_redraw()` is called when
  (a) the controller changed the camera, (b) inertia is active —
  inertia integrates dt inside `RedrawRequested` exactly as before and
  re-requests until velocity decays, (c) egui's output reports a
  zero repaint delay, or (d) the window was resized. This keeps the
  phase-1 idle-is-free property while giving animation a real frame
  loop to live on (future sun animation just becomes another
  "animating" flag).

## Milestones

### M1 — Dependency swap + blank window

- Rewrite `Cargo.toml`: drop `iced`; add `wgpu`, `winit`, `egui`,
  `egui-wgpu`, `egui-winit`, `pollster`.
- New `main.rs`: `winit::application::ApplicationHandler` impl;
  on `resumed` create the window, wgpu instance → surface → adapter →
  device/queue (empty features), configure the surface (prefer an sRGB
  format — the shader relies on hardware sRGB encode); handle
  `Resized` (reconfigure) and `CloseRequested`; `RedrawRequested`
  clears to near-black and presents.
- Globe modules temporarily out of the build (`mod` lines commented or
  a stub `main` that doesn't reference them) so this compiles alone.
- **Done when:** `cargo run` opens a window that clears, resizes
  without validation errors, and closes cleanly.

### M2 — Globe renders under raw wgpu

- `pipeline.rs` → `renderer.rs` as `GlobeRenderer` (above); fix
  wgpu 27→29 API churn; swap `iced::Rectangle`-derived aspect for
  surface width/height.
- Construct `GlobeRenderer` eagerly after device creation (the blank
  startup window now shows *before* the multi-second texture decode —
  same total wait as phase 1, slightly better perceived).
- Wire `prepare` + `render` into the frame: clear → stars → surface →
  atmosphere, default camera and sun.
- **Done when:** the rendered globe is visually identical to phase 1
  (terminator, atmosphere limb, star backdrop, sun disc), and the
  smoke test (`timeout 12 cargo run`, output to file) shows no wgpu
  validation errors.

### M3 — Mouse controls + inertia on the winit loop

- `globe/mod.rs` input logic → `globe/input.rs` `Controller`:
  - winit `MouseInput` (Left/Right) ↔ press/release, `CursorMoved` ↔
    drag with the same velocity EMA, `MouseWheel` (`LineDelta` = ticks,
    `PixelDelta` y/60) ↔ zoom — same constants and formulas throughout.
  - Flick detection on left release (`FLICK_SPEED`, `FLICK_TIMEOUT`),
    coast integration in `RedrawRequested` with real dt
    (`Instant::now()` based, capped 0.1 s), `HALF_LIFE` decay,
    `STOP_SPEED` cutoff.
  - Cursor icon: `CursorIcon::Grab`/`Grabbing` via the window.
- Redraw policy as described above; verify idle frames are actually
  zero (no redraw requests when nothing moves).
- **Done when:** pan/zoom/tilt/flick all feel identical to phase 1 and
  the app renders no frames while idle.

### M4 — egui sun sliders

- Add `egui_winit::State` + `egui_wgpu::Renderer` to the app.
- Per frame: `egui_state.take_egui_input(&window)` → `ctx.run(...)`
  building the panel → tessellate → `update_buffers` /
  `update_texture` → paint into the main render pass after the
  atmosphere, with `ScreenDescriptor { size_in_pixels,
  pixels_per_point }`.
- `src/ui.rs`: `egui::Window` or `Area` pinned top-left, fixed width
  ~260: "Sun latitude" label + slider −23.44..=23.44 step 0.1,
  "Sun longitude" label + slider −180.0..=180.0 step 0.5 — mutating
  `&mut Sun` directly.
- Event routing: egui first; `response.consumed` gates the globe
  controller. Request redraws while egui needs repaints (slider drag).
- **Done when:** sliders move the sun/stars exactly like phase 1,
  dragging a slider never pans the globe, dragging the globe under the
  panel still works elsewhere, idle stays frame-free.

### M5 — Cleanup, docs, verification

- Delete dead code: `Interaction` enum, any iced-era plumbing;
  `cargo build` warning-free.
- Confirm `iced` is gone from `Cargo.lock` (no stray transitive pin).
- Update `README.md` (project layout + tech description: winit/wgpu/
  egui instead of iced) — controls table is unchanged.
- Final smoke test + a manual interaction pass (pan, flick, zoom to
  min/max, tilt clamp, both sliders, resize, minimize/restore).
- **Done when:** clean build, clean run, README accurate.

## Risks / gotchas

- **wgpu 27→29 churn** is the main unknown (29 post-dates prior work
  here). Expect signature changes in instance/surface creation and
  descriptors; concepts are stable. Fix by compiler error + docs.
- **egui on an sRGB surface**: egui-wgpu supports sRGB targets but
  egui's colors are tuned for non-sRGB framebuffers; minor color
  dilation on widgets is possible and acceptable (two sliders). The
  globe shader *requires* the sRGB target, so the globe wins.
- **`RenderPass<'static>` for egui-wgpu**: use
  `render_pass.forget_lifetime()` — documented egui-wgpu pattern, fine
  as long as the pass doesn't outlive the encoder submission.
- **Surface format choice**: pick `Bgra8UnormSrgb`/`Rgba8UnormSrgb`
  from the surface caps explicitly rather than `caps.formats[0]`, which
  may be non-sRGB on some platforms (washed-out globe).
- **WSLg flakiness**: app launch intermittently fails with
  libEGL/MESA errors — transient, retry; not a code bug. The owner
  perf-tests on native Windows.
- **Windows mount tooling**: `cargo add` can fail with a bogus
  "found cargo.toml please rename" error here — edit Cargo.toml
  directly and trust `cargo metadata`.
- **Per-frame egui texture updates**: `update_texture` must run for
  `textures_delta.set` before the pass and `free` after submission, or
  fonts vanish/leak.

---

## Status (2026-06-11)

All five milestones implemented in one pass. Notes for future sessions:

- Final versions: wgpu 29.0.3, winit 0.30.13, egui/egui-wgpu/egui-winit
  0.34.3. Actual wgpu 27→29 churn encountered: `Instance::new` takes
  `InstanceDescriptor` by value and the descriptor lost `Default`
  (use `new_without_display_handle()`); `DeviceDescriptor` gained
  `experimental_features` (`ExperimentalFeatures::disabled()`);
  `Surface::get_current_texture()` now returns the
  `CurrentSurfaceTexture` enum instead of `Result` (`Success`/
  `Suboptimal` carry the frame; `Lost`/`Outdated` → reconfigure);
  `PipelineLayoutDescriptor` takes `&[Option<&BindGroupLayout>]` and
  `immediate_size: 0` replaces `push_constant_ranges`;
  `multiview` → `multiview_mask` on both pipeline and render-pass
  descriptors; sampler `mipmap_filter` is the new `MipmapFilterMode`.
- egui 0.34 deprecated `Context::run` in favor of `run_ui` (closure
  gets a transparent fullscreen `&mut Ui`; the Area-based panel hangs
  off `ui.ctx()`), and renamed `is_pointer_over_area` to
  `is_pointer_over_egui`. `egui_wgpu::Renderer::new` takes a
  `RendererOptions` struct.
- The cursor icon is reasserted after `handle_platform_output` each
  frame (egui resets it); skipped while the pointer is over the panel.
- Smoke-tested: clean 15 s run, no validation errors, warning-free
  build, `iced` absent from Cargo.lock. Interaction feel and the
  egui panel still need a manual pass on native Windows (the owner's
  perf/feel environment).
