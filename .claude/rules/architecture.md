# Architecture

High-level map only — query serena/codebase-memory for symbols, signatures,
and per-file detail. This file should only change on major refactors.

## Stack

Rust edition 2024. wgpu (GPU), winit (window), egui + egui_taffy (overlay
UI), satkit (SGP4 + ephemeris + EOP), glam (math), pyo3 (embedded CPython,
unconditional), rayon (parallel init), image, ktx2, clap. Build-only: ureq
(asset download), half (f16 LUT bake). Versions in `Cargo.toml`.

## Two binaries, one engine

```
main (bin globe-experiment) -> engine, scenes    (no offscreen/headless code)
headless (bin headless)     -> engine, offscreen (no scenes; compiles
                               engine::application dead under its crate-level
                               allow(dead_code))
```

Both bin roots declare the shared `mod engine;` (no lib crate). The compiler
enforces only the top level: `headless.rs` never declares `scenes`, `main.rs`
never declares `offscreen`. One workspace member besides the root package:
`macros/`, the proc-macro crate behind `#[derive(SceneClock)]` and
`#[derive(ScenePtzCamera)]` (derives cannot live in the crate that uses
them).

Engine modules and their roles:

- **`application`** — winit shell + egui wiring + the windowed presenter
  (`gfx.rs`). Keeps NO camera/input state: it statelessly translates winit
  events into `CameraControl` trait calls. All winit-touching code lives
  here.
- **`camera`** — winit-free. `mod.rs`: the `CameraControl` (input, all
  methods defaulted) + `CameraView` (`frame_state`) trait pair every scene
  implements, and the device-neutral input vocabulary. `ptz.rs`: `PtzCamera`
  (the interactive orbital rig + all input/animation state) and
  `ScenePtzCamera` (three accessors; a blanket impl supplies the whole
  `CameraControl` surface). See `camera.md`.
- **`scene`** — `Scene` trait, `RenderState`, `Clock`/`SceneClock`,
  `CameraTarget`, `CelestialBody` (`body.rs`), the celestial sphere, the
  satellite pipeline. No winit/wgpu/egui/camera-type imports.
- **`planet`** — every body's physical data (Terra + 7 planets + Luna, one
  table keyed by `CelestialBody`) + the WGS84 consts and surface helpers.
  satkit-free.
- **`renderer`** — winit-free shared render core (`SceneRenderer`, 5
  pipelines, device/depth helpers, projection consts). See `renderer.md`.
- **`ui`** — `UIDrawable` trait + `UIDrawablePanel` + `control_panel` (egui +
  taffy layout), one instrument struct per file under `instruments/`, the
  Apollo theme, the headless mock-panel spec (`spec.rs`), and the Python face
  of the panel API (`py.rs`).
- **`py`** — embedded-interpreter init, the `globe` pymodule registration,
  and the runtime script loader. Never references `src/scenes`.

Top-level extras: **`scenes`** (main tree; one module per past scene, each
with its own clap `Args` + `run()`) and **`offscreen`** (headless tree; the
surfaceless presenter).

## The four scene traits

`ApplicationState<S>` bounds `S: Scene + CameraControl + CameraView +
UIDrawable`; adding a scene never touches the application layer.

- **`Scene`** — `advance()` (scene-specific per-frame work, usually empty)
  plus the provided `tick_scene` (clock tick + advance; for every
  `Self: SceneClock`), the application's per-frame entry point.
- **`SceneClock`** — the whole clock API as trait default methods over one
  required `clock_mut()`, supplied per scene by `#[derive(SceneClock)]` over
  the scene's `clock` field. `Clock` is plain data with private fields; only
  the same-module trait defaults reach them (compiler-enforced).
- **`CameraControl`/`CameraView`** — input vs frame production.
  `CameraControl` normally via `ScenePtzCamera`, supplied per scene by
  `#[derive(ScenePtzCamera)]` over the scene's `camera` + `camera_target`
  fields.
- **`UIDrawable`** — `get_drawables()`, the frame's owned panels. Called
  once per frame BEFORE egui's `run_ui`.

Per-frame order: camera `tick` -> `tick_scene` -> `frame_state` ->
`get_drawables` -> egui pass. The loop renders unconditionally at vsync
(see `renderer.md`).

## Cross-cutting invariants

- **`CelestialSphere::at` is a pure function of time and is stored nowhere.**
  Scenes and the renderer evaluate it on the spot; `RenderState` carries only
  time + resolved camera rig + `CameraTarget` + satellite markers.
- **All CPU-side computation is f64 end to end** (sphere, camera, satellites,
  renderer `prepare`); f32 only at GPU upload and egui readouts. See
  `coordinates.md` for why.
- **All rendering is camera-target-local** (floating origin). See `camera.md`.
- **UI panels are fully owned (`'static`)**: an interactive callback receives
  the scene as `&mut S` at fire time instead of capturing it. Callbacks MUST
  be idempotent (write build-time snapshots, never read-modify-write) —
  egui's discard pass can fire one twice per frame.
- **Deliberate duplication**: the Time-panel builder, the propagation loop,
  the `set_camera_target` helper, and the `ISS_TLE` literal are duplicated
  per scene on purpose so scenes can diverge. Do not factor them out.
- **The scene owns its `camera_target`**; `PtzCamera` stores no target and
  takes `&CameraTarget` in every call that depends on the orbited body.

## Purity rules (kept by review)

- `scene` imports neither winit/wgpu nor any camera or ui type.
  (`RenderState` and `CameraTarget` are defined in `scene` and consumed by
  `renderer`/`camera` — the two sanctioned edges.)
- `camera` and `renderer` are winit-free; the windowed presenter (`Gfx`)
  lives in `application/gfx.rs`, its headless twin in `src/offscreen.rs`.
- `application` never touches the `CelestialSphere`; it consumes only the
  finished `RenderState`.
