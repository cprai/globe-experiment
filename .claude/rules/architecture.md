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
                         (render takes one --scene JSON + --output/width/height)
src/snapshot.rs          headless single-frame render mode (no EOP range check);
                         SceneSpec = --scene JSON (simulation + camera +
                         optional ui); optional mock-panel overlay
                         (build_ui_frame)
src/scenarios/mod.rs     scenario registry
src/scenarios/iss_and_hubble.rs  IssAndHubbleSimulation (Simulation impl); ISS_TLE/HST_TLE consts
src/scenarios/iss.rs     IssSimulation (Simulation impl); own ISS_TLE const (duplicated on purpose)
src/application/mod.rs   ApplicationState<S: Simulation> + winit ApplicationHandler + run()
src/application/camera.rs   orbital camera (inertial-frame rig, km world space)
src/application/input.rs    Controller: drag/tilt/wheel, flick inertia, smoothed zoom
src/ui/mod.rs            UI module root: owns UIDrawable trait + UIDrawablePanel
                         + PanelAnchor (egui-free data), impl UIDrawable for
                         SimulationState, and the egui control_panel that frames
                         each panel at its anchored position and renders its
                         boxed Instrument trait objects at panel-relative
                         positions (interactivity via callbacks). Re-exports the
                         instrument structs + theme install_theme + mock types.
src/ui/instruments/mod.rs  the Instrument trait (position + render); one
                         self-contained instrument STRUCT per sibling file, each
                         impl Instrument with its own baked-in look (a producer
                         picks which instrument + content, never style)
src/ui/instruments/{header,readout,dual_readout,button,toggle,lamp,slider}.rs
                         one instrument each (header.rs amber title+rule;
                         readout.rs label/value window + shared readout_pair;
                         dual_readout.rs two readouts; button.rs momentary key;
                         toggle.rs latching green key + toggle_key; lamp.rs
                         status dot + LampStatus; slider.rs value track). Control
                         instruments carry an Option<Box<dyn FnMut>> (None=inert)
src/ui/theme.rs          install_theme: the Apollo-panel egui look (gunmetal
                         frame, monospace UPPERCASE cream readouts, green-active
                         keys, corner rivets/bevel), the palette consts shared by
                         the instruments, and panel chrome (panel_frame/bevel/
                         rivets). Stamped onto the egui Context by both the
                         windowed app and the headless render path
src/ui/mock.rs           the serde-derived mock spec (UiPanelSpec/UiElementSpec)
                         + MockUi for the render --scene `ui` overlay; maps each
                         owned spec to an inert (None-callback) boxed Instrument
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

## `UIDrawable` trait + `UIDrawablePanel` + `Instrument`

The trait + panel live in `src/ui/mod.rs`; each instrument is a struct in its
own `src/ui/instruments/*.rs` (egui-free *data* + boxed closures - egui only
enters in each instrument's `render` and in `control_panel`). Decouples panel
*rendering* from *interactivity*. Lives in `ui`, not `simulation`, so the
simulation core stays UI-free.

```
UIDrawable::get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>>
    The positioned panels for one frame.

UIDrawablePanel { anchor: PanelAnchor, offset: [f32;2], size: [f32;2],
                  elements: Vec<Box<dyn Instrument + 'a>> }
    A panel owns its on-screen place (corner anchor + inset, resolved against
    the live window) and a fixed box `size` (fixes the frame and pins the egui
    Area so it can't auto-shrink). Its elements are positioned RELATIVE to it.

trait Instrument { position(&self) -> [f32;2];
                   render(&mut self, ui, child_rect, panel_size) }
    One struct per file impls it: Header, Readout, DualReadout, Button, Toggle,
    Lamp, Slider. Pre-styled INSTRUMENTS, not logical primitives: a producer
    picks which instrument + its content, never its color/font/emphasis (style
    lives in each `render`, pulling palette consts from `theme`). control_panel
    scopes a child Ui per instrument (position + wrap setup) then calls render;
    only Header uses child_rect/panel_size (its full-width rule).
    Header=amber title+rule. Readout/DualReadout=dim label(s) + cream value(s)
    in recessed windows (the label/value split; readout_pair shared by both).
    Button=momentary key, Toggle=latching key (lit green while `active`),
    Lamp=status dot keyed to LampStatus{Ok/Caution/Fault/Off}, Slider=value
    track. Button/Toggle/Slider carry an Option<Box<dyn FnMut(..)>> callback
    (None = inert, e.g. a mock).

PanelAnchor::{ TopLeft, TopRight }   # add bottom corners when needed
```

- `impl UIDrawable for SimulationState` emits **one** panel (top-left) from live
  state: subsolar + datetime readouts, and the Run toggle + speed slider
  whose callbacks mutate the live clock (each captures a *disjoint* clock field -
  `paused` vs `multiplier` - via direct field assignment, so both coexist with
  no interior mutability; do not call a `Clock` method in those closures, it
  would borrow the whole clock).
- Each scenario's `impl UIDrawable` returns `self.simulation.get_drawables()`
  (the core panel) plus **one** scenario panel (top-right) built from the stashed
  `last_telemetry` (a disjoint field). The two panels are independently
  positioned - no stacking constant. `ui::control_panel(&mut impl UIDrawable)`
  frames each panel and renders its instruments, firing callbacks on
  interaction.
- **Theme**: `ui::install_theme(ctx)` stamps the Apollo-panel look onto an egui
  `Context` and must be called once per context (both `ApplicationState::new`
  and `snapshot::build_ui_frame` do). **All color lives in `ui::theme` (the
  palette consts) + each instrument's `render`** - producers pick instruments,
  never colors. Each instrument's `render` uppercases its text; `control_panel`
  frames each panel with the gunmetal `panel_frame` and paints the bevel
  highlight + corner rivets per panel.

`SimulationState` (clock + celestial sphere) is the shared core that every
scenario struct holds by composition. Satellites belong to the scenario struct,
not to `SimulationState`. `Clock` is re-exported from `simulation` so callers
need not know the `clock` submodule path.

## Purity rules (compiler-enforced)

- **`simulation` imports neither winit/wgpu/egui/`ui` nor the `Camera` type.**
  The `Simulation` trait takes resolved `Vec3`/`Mat4` values for the camera
  and returns a `RenderState`. This keeps input scheme changes local to
  `application` and each scenario's `frame_state` impl independently testable.
  The `UIDrawable`/`UIDrawablePanel`/`Instrument` split extends this:
  those types live in `ui` (not `simulation`), and the panels render with
  optional callbacks, so the same code can drive a mock UI (all callbacks
  `None`) with no live `Clock`.
- **`Camera` type lives in `application` only.** Other modules see only a
  resolved `eye`/`view_proj`. (`RenderState` is defined in `simulation` but
  consumed by `renderer` — the one allowed edge.)
