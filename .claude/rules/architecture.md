# Architecture & file map

## Stack

Rust edition 2024. `wgpu 29` (GPU), `winit 0.30` (window), `egui 0.34`
(overlay), `satkit 0.18` (SGP4 + ephemeris + EOP), `glam 0.33` (math),
`rayon 1.10` (parallel init), `image 0.25` (texture decode + PNG encode),
`ktx2 0.5` (LUT parse/write), `humantime 2` (render-mode datetime parse).
Build-only: `ureq 3.3` (asset download), `half 2.7` (f16 LUT bake).
Crate name: `globe-experiment`.

## File map

```
build.rs                 downloads 5 textures (JPEG/TIFF verbatim) + JPL
                         DE440 ephemeris + EOP-All.csv into OUT_DIR; bakes
                         3 atmosphere LUTs as f16 KTX2. Contains mod atmosphere.
(no .cargo/config.toml)  deleted - was only for intel_tex_2's ISPC linkage
src/main.rs              clap CLI: `scenario <name>` | `render` subcommands
src/snapshot.rs          headless single-frame render mode (no EOP range check)
src/scenarios/mod.rs     scenario registry
src/scenarios/iss_and_hubble.rs  IssAndHubbleSimulation (Simulation impl); ISS_TLE/HST_TLE consts
src/scenarios/iss.rs     IssSimulation (Simulation impl); own ISS_TLE const (duplicated on purpose)
src/application/mod.rs   ApplicationState<S: Simulation> + winit ApplicationHandler + run()
src/application/camera.rs   orbital camera (inertial-frame rig, km world space)
src/application/input.rs    Controller: drag/tilt/wheel, flick inertia, smoothed zoom
src/ui.rs                egui control panel (clock + readouts)
src/earth.rs             WGS84 constants + surface_position / geodetic_normal helpers
src/renderer/mod.rs      Gfx: surface/device/queue + egui_wgpu + GlobeRenderer
src/renderer/headless.rs HeadlessRenderer: surfaceless Rgba8Unorm offscreen render
src/renderer/mesh.rs     WGS84 ellipsoid mesh generator (km, geodetic normals)
src/simulation/mod.rs    Simulation trait, SimulationState (core: clock + celestial sphere),
                         RenderState, TelemetryState
src/simulation/celestial_sphere.rs  ephemeris-driven Sun + star-map orientation
src/simulation/satellite.rs  TLE parse + satkit SGP4 + TEME->world-km conversion
src/simulation/clock.rs  simulation Clock: wall-dt x speed, play/pause
shaders/globe.wgsl       ALL shader code (4 passes in one module)
OUT_DIR/                 gitignored; include_bytes!'d: 5 JPEG/TIFF textures +
                         3 f16 LUT KTX2 + DE440 ephemeris + EOP-All.csv
```

## Module dependency graph

```
main        -> application, simulation, renderer, scenarios
application -> simulation, renderer, ui, earth, (winit, egui, egui_winit, glam)
ui          -> simulation, (egui)
renderer    -> simulation (RenderState), earth, (wgpu, egui_wgpu, ktx2, glam)
simulation  -> earth, (satkit, glam)   # NO winit / wgpu / egui / Camera type
earth       -> (glam)
scenarios   -> simulation, application
```

## `Simulation` trait

Defined in `src/simulation/mod.rs`. The sole interface `ApplicationState` uses;
adding a scenario requires no changes to the application layer.

```
advance(&mut self) -> bool
    Tick the clock + re-evaluate the celestial sphere. Returns whether the
    clock is running (keeps frames coming; paused = app goes idle).

celestial_to_world(&self) -> Mat3
    Rotation from the inertial (star-fixed) camera rig frame to the
    Earth-fixed world frame. Called by the application before each frame to
    resolve the camera into world space.

frame_state(&mut self, eye: Vec3, view_proj: Mat4) -> (RenderState, TelemetryState)
    Propagate all satellites once, fill RenderState (renderer) and
    TelemetryState (UI) from the same propagation.

clock_mut(&mut self) -> &mut Clock
    Direct clock mutation for the egui panel (play/pause + speed slider).
    The UI owns the Clock reference for a frame; no message queue.
```

`SimulationState` (clock + celestial sphere) is the shared core that every
scenario struct holds by composition. Satellites belong to the scenario struct,
not to `SimulationState`. `Clock` is re-exported from `simulation` so callers
need not know the `clock` submodule path.

## Purity rules (compiler-enforced)

- **`simulation` imports neither winit/wgpu/egui nor the `Camera` type.**
  The `Simulation` trait takes resolved `Vec3`/`Mat4` values for the camera
  and returns a `RenderState`. This keeps input scheme changes local to
  `application` and each scenario's `frame_state` impl independently testable.
- **`Camera` type lives in `application` only.** Other modules see only a
  resolved `eye`/`view_proj`. (`RenderState` is defined in `simulation` but
  consumed by `renderer` — the one allowed edge.)
