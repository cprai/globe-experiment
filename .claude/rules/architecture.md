# Architecture

High-level map only — query serena/codebase-memory for symbols, signatures,
and per-file detail. This file should only change on major refactors.

## Stack

Rust edition 2024. wgpu (GPU), winit (window), egui + egui_taffy (overlay
UI), satkit (SGP4 + ephemeris + EOP), glam (math), pyo3 (embedded CPython,
unconditional), rayon (parallel init), image, ktx2, clap. Build-only: ureq
(asset download), half (f16 LUT bake). Versions in `Cargo.toml`.

## Three crates: engine lib, windowed app, proc macros

```
globe-experiment (root pkg, bin)  -> engine        src/main.rs + src/scenes/
engine (lib + bin headless)       -> engine-macros crates/engine/src/ (lib.rs), owns
                                                   offscreen.rs + headless.rs
                                                   + build.rs (assets/OUT_DIR)
engine-macros (proc-macro)                         the scene derives
```

The crate boundary enforces the dependency direction: engine never sees
`scenes`, and the windowed app pulls in no offscreen/headless code (those
live behind engine's `headless` bin, run via `cargo run -p engine --bin
headless`). `engine-macros` holds the scene derives (`SceneClock`,
`ScenePtzCamera`, `SceneOrbitalBodies`, `SceneKinematicBodies` — derives
cannot live in the crate that uses them); their generated impls emit
`::engine::...` paths and are re-exported through `engine::scene` /
`engine::camera`, so consumers never depend on the macro crate directly.
Root `Cargo.toml` keeps the `[profile.dev.package.*]` decode-speed overrides
(profiles only apply from the workspace root).

Engine modules and their roles:

- **`application`** — winit shell + egui wiring + the windowed presenter
  (`gfx.rs`). Keeps NO camera/input state: it statelessly translates winit
  events into `CameraControl` trait calls. All winit-touching code lives
  here.
- **`camera`** — winit-free. `mod.rs`: the `CameraControl` (input, all
  methods defaulted) + `CameraView` (`frame_state`) trait pair every scene
  implements, and the device-neutral input vocabulary. `ptz.rs`: `PtzCamera`
  (the interactive orbital rig + all input/animation state) and
  `ScenePtzCamera` (three accessors; blanket impls supply the whole
  `CameraControl` surface and — together with the clock/body traits — all of
  `CameraView`). See `camera.md`.
- **`scene`** — `Scene` trait, `RenderState`, `Clock`/`SceneClock`,
  `CameraTarget`, `CelestialBody` (`celestial_body.rs`), the celestial
  sphere, and the tracked-body pipeline (`body.rs`: `TrackedBody` + shared
  state types/occlusion; `orbital_body.rs`: SGP4 + `SceneOrbitalBodies`;
  `kinematic_body.rs`: numerical + thrust + `SceneKinematicBodies`). No
  winit/wgpu/egui/camera-type imports.
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

Also in engine: **`offscreen`** (the surfaceless presenter behind the
`headless` bin). In the root crate: **`scenes`** (one module per past scene,
each with its own clap `Args` + `run()`).

## The four scene traits

`ApplicationState<S>` bounds `S: Scene + CameraControl + CameraView +
UIDrawable`; adding a scene never touches the application layer.

- **`Scene`** — `advance()` (scene-specific per-frame work, usually empty)
  plus the provided `tick_scene` (clock tick + advance; for every
  `Self: SceneClock`), the application's per-frame entry point, and the
  provided `tracked_bodies` (body -> `TrackedBody` conversion; for every
  `Self: SceneOrbitalBodies + SceneKinematicBodies`).
- **`SceneOrbitalBodies`/`SceneKinematicBodies`** — one `&mut`-slice
  accessor each over the scene's per-kind body `Vec`, supplied per scene by
  the same-named derives. Unlike the other scene derives, a missing field is
  not an error: the derived impl returns the empty slice, so every scene
  derives both and body-less scenes get an empty `tracked_bodies` for free.
- **`SceneClock`** — the whole clock API as trait default methods over one
  required `clock_mut()`, supplied per scene by `#[derive(SceneClock)]` over
  the scene's `clock` field. `Clock` is plain data with private fields; only
  the same-module trait defaults reach them (compiler-enforced).
- **`CameraControl`/`CameraView`** — input vs frame production.
  `CameraControl` normally via `ScenePtzCamera`, supplied per scene by
  `#[derive(ScenePtzCamera)]` over the scene's `camera` + `camera_target`
  fields. `CameraView` comes from a second blanket impl (in `ptz.rs`) over
  `Scene + SceneClock + ScenePtzCamera` + the two body traits — no scene
  implements `frame_state` itself; a scripted/fixed camera would implement
  `CameraView` directly instead of `ScenePtzCamera`.
- **`UIDrawable`** — `get_drawables()`, the frame's owned panels. Called
  once per frame BEFORE egui's `run_ui`.

Per-frame order: camera `tick` -> `tick_scene` -> `frame_state` ->
`get_drawables` -> egui pass. The loop renders unconditionally at vsync
(see `renderer.md`).

## Cross-cutting invariants

- **`CelestialSphere::at` is a pure function of time and is stored nowhere.**
  The `CameraView` blanket impl and the renderer evaluate it on the spot;
  `RenderState` carries only
  time + resolved camera rig + `CameraTarget` + tracked bodies (dot +
  precomputed trail, plain data).
- **Scenes own their tracked bodies** as direct per-kind `Vec` fields
  (`Vec<OrbitalBody>` / `Vec<KinematicBody>`, only the kinds the scene
  tracks), exposed through the derived `SceneOrbitalBodies` /
  `SceneKinematicBodies` accessors; `frame_state` converts them to
  plain-data `TrackedBody` via the provided `Scene::tracked_bodies`
  (`state_at`/`trail` + `body_occluded`). The bodies' only mutation surface
  is `KinematicBody::apply_thrust`.
- **All CPU-side computation is f64 end to end** (sphere, camera, tracked
  bodies, renderer `prepare`); f32 only at GPU upload and egui readouts. See
  `coordinates.md` for why.
- **All rendering is camera-target-local** (floating origin). See `camera.md`.
- **UI panels are fully owned (`'static`)**: an interactive callback receives
  the scene as `&mut S` at fire time instead of capturing it. Callbacks MUST
  be idempotent (write build-time snapshots, never read-modify-write) —
  egui's discard pass can fire one twice per frame.
- **Deliberate duplication**: the Time-panel builder, the body ->
  `BodyTelemetry` conversion loops (`get_drawables`; the `TrackedBody`
  conversion is shared via `Scene::tracked_bodies` since 2026-07-12), the
  `set_camera_target` helper, and the `ISS_TLE` literal are duplicated per
  scene on purpose so scenes can diverge. Do not factor them out.
- **The scene owns its `camera_target`**; `PtzCamera` stores no target and
  takes `&CameraTarget` in every call that depends on the orbited body.

## Purity rules (kept by review)

- `scene` imports neither winit/wgpu nor any camera or ui type. (The
  sanctioned dependency direction is the reverse: `renderer`/`camera` consume
  `scene` types — `RenderState`, `CameraTarget`, and, for the `CameraView`
  blanket impl in `ptz.rs`, the scene traits + `CelestialSphere`.)
- `camera` and `renderer` are winit-free; the windowed presenter (`Gfx`)
  lives in `application/gfx.rs`, its headless twin in `crates/engine/src/offscreen.rs`.
- `application` never touches the `CelestialSphere`; it consumes only the
  finished `RenderState`.
