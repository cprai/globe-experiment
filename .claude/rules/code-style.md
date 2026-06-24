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

Six top-level modules + `earth`:
- **`application`** — window, camera, input, egui logic. Owns `Camera` +
  `Controller`. `ApplicationState<S: Simulation>` is generic over the
  simulation — nothing outside this module names the `Camera` type.
- **`simulation`** — the `Simulation` trait, `SimulationState` (clock +
  celestial sphere; **no satellites**), `RenderState`/`SimulationUIState`/
  `ScenarioUIState`, and helpers. **No winit/wgpu/egui dependency. No `Camera`
  type.** Takes resolved `Vec3`/`Mat4`; returns
  `RenderState`/`SimulationUIState`/`ScenarioUIState`.
- **`renderer`** — `Gfx` + `HeadlessRenderer`. Camera is NOT here.
- **`ui`** — egui control panel.
- **`earth`** — WGS84 constants + helpers. Single source of truth for all
  geometry; mesh and camera both call it.
- **`scenarios`** — one `<Name>Simulation` struct + `Simulation` impl per
  past scenario, each with a `run()`. Satellites live here, not in
  `simulation`.
- **`snapshot`** — headless single-frame render mode.

## Where things live

- **Shader look knobs**: `shaders/globe.wgsl` top `const` block.
- **Atmosphere medium constants**: `build.rs mod atmosphere` (bake) AND
  `shaders/globe.wgsl` (shader twins) — both must stay in sync.
- **Input feel constants**: `src/application/input.rs` top.
- **Earth physical constants + helpers**: `src/earth.rs`.
- **Camera limits**: `Camera` associated consts in `src/application/camera.rs`
  (in km).
- **All build assets**: in `OUT_DIR`, `include_bytes!`-ed. No `assets/` dir.
- **TLE data**: inline source `const`s in the scenario files, not in
  `satellite.rs`. The `ISS_TLE` literal is **deliberately duplicated** across
  scenarios that need it — do not factor into a shared const.

## Documentation rule

Keep all docs current in the same change: code comments, `.claude/CLAUDE.md`,
rules files, and `README.md`. Stale docs are bugs. Exception: the live
constant snapshot in `shader.md` may lag owner tuning — for live values the
source (`globe.wgsl`) is always authoritative.
