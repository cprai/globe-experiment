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
src/ui.rs                UI module: owns UIDrawable trait + UIDrawableElement
                         (egui-free data), impl UIDrawable for SimulationState,
                         and the egui control_panel that renders a flat element
                         list at absolute positions (interactivity via callbacks)
src/earth.rs             WGS84 constants + surface_position / geodetic_normal helpers
src/renderer/mod.rs      Gfx: surface/device/queue + egui_wgpu + GlobeRenderer
src/renderer/headless.rs HeadlessRenderer: surfaceless Rgba8Unorm offscreen render
src/renderer/mesh.rs     WGS84 ellipsoid mesh generator (km, geodetic normals)
src/simulation/mod.rs    Simulation trait (UI-agnostic), SimulationState
                         (core: clock + celestial sphere), RenderState,
                         SatelliteTelemetry
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
ui          -> simulation (SimulationState/Clock), (egui)   # owns UIDrawable
renderer    -> simulation (RenderState), earth, (wgpu, egui_wgpu, ktx2, glam)
simulation  -> earth, (satkit, glam)   # NO winit / wgpu / egui / ui / Camera
earth       -> (glam)
scenarios   -> simulation, ui, application
```

## `Simulation` trait

Defined in `src/simulation/mod.rs`. The sole simulation interface
`ApplicationState` uses; adding a scenario requires no changes to the
application layer. It is **UI-agnostic** - the panel reads/drives a scenario
through a *separate* `ui::UIDrawable` impl, so `simulation` has no UI
dependency. `ApplicationState<S>` bounds `S: Simulation + UIDrawable`.

```
advance(&mut self) -> bool
    Tick the clock + re-evaluate the celestial sphere. Returns whether the
    clock is running (keeps frames coming; paused = app goes idle).

celestial_to_world(&self) -> Mat3
    Rotation from the inertial (star-fixed) camera rig frame to the
    Earth-fixed world frame. Called by the application before each frame to
    resolve the camera into world space.

frame_state(&mut self, eye: Vec3, view_proj: Mat4) -> RenderState
    Propagate all satellites once, fill RenderState (renderer). Stashes the
    same-propagation per-satellite readout (Vec<SatelliteTelemetry>) on the
    scenario for the immediately-following get_drawables call.
```

## `UIDrawable` trait + `UIDrawableElement`

Defined in `src/ui.rs` (egui-free: plain data + boxed closures - egui only
enters in `control_panel`). Decouples panel *rendering* from *interactivity*.
Lives in `ui`, not `simulation`, so the simulation core stays UI-free.

```
UIDrawable::get_drawables(&mut self) -> Vec<UIDrawableElement<'_>>
    A flat list of self-positioned elements for one frame.

UIDrawableElement::{ Text, Button, Slider }
    Each carries an absolute screen position [f32;2]. Button/Slider carry an
    Option<Box<dyn FnMut(..)>> callback (None = inert, e.g. a mock panel).
```

- `impl UIDrawable for SimulationState` emits the shared-core block from live
  state: subsolar + datetime readouts, and the play/pause button + speed slider
  whose callbacks mutate the live clock (each captures a *disjoint* clock field -
  `paused` vs `multiplier` - via direct field assignment, so both coexist with
  no interior mutability; do not call a `Clock` method in those closures, it
  would borrow the whole clock).
- Each scenario's `impl UIDrawable` prepends `self.simulation.get_drawables()`
  then appends its per-satellite readout from the stashed `last_telemetry`
  (positioned below `SCENARIO_UI_TOP_Y`). `ui::control_panel(&mut impl
  UIDrawable)` renders them, firing callbacks on interaction.

`SimulationState` (clock + celestial sphere) is the shared core that every
scenario struct holds by composition. Satellites belong to the scenario struct,
not to `SimulationState`. `Clock` is re-exported from `simulation` so callers
need not know the `clock` submodule path.

## Purity rules (compiler-enforced)

- **`simulation` imports neither winit/wgpu/egui/`ui` nor the `Camera` type.**
  The `Simulation` trait takes resolved `Vec3`/`Mat4` values for the camera
  and returns a `RenderState`. This keeps input scheme changes local to
  `application` and each scenario's `frame_state` impl independently testable.
  The `UIDrawable`/`UIDrawableElement` split extends this: those types live in
  `ui` (not `simulation`), and the panel renders a flat element list with
  optional callbacks, so it can drive a mock UI (all callbacks `None`) with no
  live `Clock`.
- **`Camera` type lives in `application` only.** Other modules see only a
  resolved `eye`/`view_proj`. (`RenderState` is defined in `simulation` but
  consumed by `renderer` — the one allowed edge.)
