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
src/scenarios/iss_and_hubble.rs  ISS+HST scenario; owns ISS_TLE/HST_TLE consts
src/scenarios/iss.rs     ISS-only; owns its own ISS_TLE const (duplicated on purpose)
src/application/mod.rs   ApplicationState + winit ApplicationHandler + run()
src/application/camera.rs   orbital camera (inertial-frame rig, km world space)
src/application/input.rs    Controller: drag/tilt/wheel, flick inertia, smoothed zoom
src/ui.rs                egui control panel (clock + readouts)
src/earth.rs             WGS84 constants + surface_position / geodetic_normal helpers
src/renderer/mod.rs      Gfx: surface/device/queue + egui_wgpu + GlobeRenderer
src/renderer/headless.rs HeadlessRenderer: surfaceless Rgba8Unorm offscreen render
src/renderer/mesh.rs     WGS84 ellipsoid mesh generator (km, geodetic normals)
src/simulation/mod.rs    SimulationState, RenderState, TelemetryState, frame_state
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

## Purity rules (compiler-enforced)

- **`simulation` imports neither winit/wgpu/egui nor the `Camera` type.**
  It takes resolved `Vec3`/`Mat4` values for the camera and returns a
  `RenderState`. This makes `frame_state` testable and keeps input scheme
  changes local to `application`.
- **`Camera` type lives in `application` only.** Other modules see only a
  resolved `eye`/`view_proj`. (`RenderState` is defined in `simulation` but
  consumed by `renderer` — the one allowed edge.)
