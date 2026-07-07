---
paths:
  - "src/**/*.rs"
  - "build.rs"
---

# Code style & conventions

- Match surrounding code: dense explanatory comments on *why* (the non-obvious
  GPU/winit/precision reasons), small focused structs, descriptive names.
- Comments explain the WHY (hidden constraint, subtle invariant, workaround
  for a specific bug). Do not explain what the code does or reference the
  caller or task.

## Module conventions

Two bin roots (`main.rs` = the windowed `globe-experiment`, `headless.rs` =
the single-frame `headless` bin), both declaring the one shared `src/engine/`
module (no lib crate) plus their own top-level extra (`scenarios` for main,
`offscreen` for headless). The engine modules:
- **`application`** — window, egui logic, and the windowed presenter. Keeps
  NO camera or input state: `translate_camera_event` statelessly maps each
  winit input event onto one `CameraControl`-trait call. Owns `gfx.rs` (`Gfx`:
  surface/swapchain/present around the shared `renderer::SceneRenderer`). All
  the winit-touching code lives here; only the main tree calls it (the
  headless tree compiles it dead).
- **`camera`** — directory module, winit-free, shared by both trees.
  `camera/mod.rs`: the `CameraControl` + `CameraView` traits every scenario
  implements (input/tick/cursor_hint vs frame_state) + the
  device-neutral input types (`PointerButton`/`ScrollDelta`/`CursorHint`).
  `camera/ptz.rs`: `PtzCamera`, the interactive pan/tilt/zoom rig + ALL its
  input/animation state; scenarios embed one and forward both traits to it, the
  headless bin constructs one from the `--scene` JSON (`PtzCamera::new`).
- **`simulation`** — the `Simulation` trait (UI-agnostic), `RenderState`,
  `SatelliteTelemetry`, `Clock`, the celestial sphere, the selectors, and
  helpers. The clock + celestial sphere are held **directly by each scenario
  struct** (there is no shared core struct). **No winit/wgpu dependency. No
  camera type** (the trait shrank to `advance()`; the frame's `RenderState` -
  plain data defined here - is produced by the scenario's `camera::CameraView`
  impl, the UI readout pulled separately via `ui::UIDrawable`). Depends on
  `ui` (hence egui) only for the selector panel builders.
- **`renderer`** — the winit-free shared scene core: `SceneRenderer` + the
  device/depth helpers + `UiFrame` + projection consts. Camera is NOT here;
  `Gfx` is NOT here (it is winit-bound, in `application/gfx.rs`).
- **`ui`** — directory module. `ui/mod.rs` owns the `UIDrawable` trait +
  `UIDrawablePanel` + `PanelAnchor` (egui-free data) and the egui
  `control_panel` that frames each panel at its anchored corner and lays out
  its rows of boxed `Instrument`s with taffy (`egui_taffy`; content-sized, no
  pixel positions; no `Clock`/scenario knowledge). Each scenario implements
  `UIDrawable` itself (its own Time panel + scenario panels).
  `ui/instruments/*.rs` is one `Instrument`-impl struct per file;
  `ui/theme.rs` the Apollo look + palette + the metric tokens and taffy
  panel/row styles; `ui/spec.rs` the serde `ui`-overlay spec (deserialized
  straight into the bare instrument structs).

The top-level (non-engine) modules:
- **`offscreen`** — `OffscreenRenderer` (`src/offscreen.rs`): the headless
  bin's surfaceless presenter + readback around `SceneRenderer`. Headless bin
  tree only.
- **`scenarios`** — one `<Name>Simulation` struct per past scenario
  implementing `Simulation` + `CameraControl` + `CameraView` + `UIDrawable`,
  each with a `run()`.
  Each struct holds its `Clock` + `CelestialSphere` + `camera: PtzCamera`
  directly and builds its own Time panel (the panel code and the
  camera-trait forwarding block are deliberately duplicated per scenario so
  scenarios can diverge). Satellites live here, not in `simulation`.
- **`headless` bin root** (`src/headless.rs`) — the single-frame render
  binary: flat `--scene`/`--output` CLI, scene-spec parsing, mock-UI
  `build_ui_frame`, calls `OffscreenRenderer`. Carries a crate-level
  `allow(dead_code)` (the engine includes items only the main tree calls,
  chiefly all of `engine::application`); the main tree keeps full dead-code
  checking.

## Where things live

- **Shader look knobs**: `shaders/scene.wgsl` top `const` block.
- **Atmosphere medium constants**: `build.rs mod atmosphere` (bake) AND
  `shaders/scene.wgsl` (shader twins) — both must stay in sync.
- **Input feel constants**: `src/engine/camera/ptz.rs` top.
- **All body physical constants + helpers (Terra included)**:
  `src/engine/planet.rs` (the per-body table; WGS84 consts live beside
  Terra's row).
- **Camera limits**: `PtzCamera` associated consts in `src/engine/camera/ptz.rs`
  — the distance/default limits are radius *ratios* (`*_RADII`), scaled at use by
  the orbit target's `mean_radius_km()`. **Projection** consts live in `renderer`
  (`FOV_Y_DEG`, `NEAR_PLANE_RADII`, `FAR_PLANE_KM`); the far plane is a *floor* —
  `prepare` grows it to enclose a large orbited body (see `camera.md`).
- **All build assets**: in `OUT_DIR`, `include_bytes!`-ed. No `assets/` dir.
- **TLE data**: inline source `const`s in the scenario files, not in
  `satellite.rs`. The `ISS_TLE` literal is **deliberately duplicated** across
  scenarios that need it — do not factor into a shared const.

## Documentation rule

Keep all docs current in the same change: code comments, `.claude/CLAUDE.md`,
rules files, and `README.md`. Stale docs are bugs. Exception: the live
constant snapshot in `shader.md` may lag owner tuning — for live values the
source (`scene.wgsl`) is always authoritative.
