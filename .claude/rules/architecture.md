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
                         optional ui); camera.target "earth"/"moon"
                         (CameraTargetSpec, default earth); optional mock-panel
                         overlay (build_ui_frame)
src/scenarios/mod.rs     scenario registry
src/scenarios/iss_and_hubble.rs  IssAndHubbleSimulation (Simulation impl); ISS_TLE/HST_TLE consts
src/scenarios/iss.rs     IssSimulation (Simulation impl); own ISS_TLE const (duplicated on purpose)
src/scenarios/solar_eclipse.rs  SolarEclipseSimulation: empty (NO satellites);
                         clock starts from the 2024-04-08 eclipse datetime;
                         run() frames the Earth day side via Camera::looking_toward;
                         TargetSelector (default Earth) for the EARTH/MOON panel
src/scenarios/lunar_eclipse.rs  LunarEclipseSimulation: empty (NO satellites);
                         clock starts from the 2025-03-14 eclipse datetime;
                         run() launches orbiting the Moon (Moon-target
                         looking_toward); TargetSelector (default Moon)
src/scenarios/solar_system.rs  SolarSystemSimulation: empty (NO satellites);
                         clock starts 2025-06-01; draws all 7 planets at true
                         pos/scale; BodySelector (one key per body: Earth, Moon,
                         the 7 planets) drives camera_target; default Earth view
src/application/mod.rs   ApplicationState<S: Simulation> + winit ApplicationHandler + run()
src/application/camera.rs   orbital camera (inertial-frame rig, km world space)
                         orbiting a CameraTarget (Earth/Moon); per-frame retarget
src/application/input.rs    Controller: drag/tilt/wheel, flick inertia, smoothed
                         zoom, reset_animation (on target switch)
src/ui/mod.rs            UI module root: owns UIDrawable trait + UIDrawablePanel
                         + PanelAnchor (egui-free data), and the egui
                         control_panel that frames each panel at its anchored
                         position and renders its boxed Instrument trait objects
                         at panel-relative positions (interactivity via
                         callbacks). The shared-core impl UIDrawable for
                         SimulationState lives in src/simulation/mod.rs. Re-exports
                         the instrument structs (bare + Interactive*) + theme
                         install_theme + the spec types (PanelSet/UiPanel).
src/ui/instruments/mod.rs  the Instrument trait (position + render); one
                         self-contained instrument STRUCT per sibling file, each
                         impl Instrument with its own baked-in look (a producer
                         picks which instrument + content, never style)
src/ui/instruments/{header,readout,dual_readout,button,toggle,lamp,slider}.rs
                         one instrument each (header.rs amber title+rule;
                         readout.rs label/value window + shared readout_pair;
                         dual_readout.rs two readouts; button.rs momentary key;
                         toggle.rs latching green key + toggle_key; lamp.rs
                         status dot + LampStatus; slider.rs value track). Each
                         control is split in two: a bare data struct (inert,
                         derives Deserialize) + an Interactive* wrapper holding
                         the bare struct + a moved Box<dyn FnMut> callback
                         (InteractiveButton/InteractiveToggle/InteractiveSlider).
                         Shared draw lives on the bare struct. InteractiveButton
                         has no live producer yet (allow(dead_code)) - the full
                         set ships as a reusable instrument library
src/ui/theme.rs          install_theme: the Apollo-panel egui look (gunmetal
                         frame, monospace UPPERCASE cream readouts, green-active
                         keys, corner rivets/bevel), the palette consts shared by
                         the instruments, and panel chrome (panel_frame/bevel/
                         rivets). Stamped onto the egui Context by both the
                         windowed app and the headless render path
src/ui/spec.rs           the serde-deserialized render --scene `ui` overlay: a
                         tagged enum (UiElement) over the bare instrument structs
                         themselves + a UiPanel + PanelSet (UIDrawable). No mirror
                         type - the bare structs derive Deserialize, so each
                         element clones into an inert boxed Instrument
src/earth.rs             WGS84 constants + surface_position / geodetic_normal helpers
src/moon.rs              lunar constants (triaxial ellipsoid radii, mean radius)
                         + surface_position / geodetic_normal (body-fixed frame);
                         the Earth-style single source of truth for Moon geometry
src/planet.rs            the 7 planets: Planet enum + ALL + data-driven table
                         (oblate radii, IAU rotation constants, texture file) +
                         surface_position / geodetic_normal. satkit-free, like
                         earth/moon
src/renderer/mod.rs      Gfx: surface/device/queue + egui_wgpu + GlobeRenderer
                         (6 pipelines incl. Moon + planets; reversed-Z
                         Depth32Float buffer). Planets use a separate group-1
                         bind group (per-planet uniform + texture)
src/renderer/headless.rs HeadlessRenderer: surfaceless Rgba8Unorm offscreen render
                         (+ matching depth buffer)
src/renderer/mesh.rs     generic ellipsoid mesh generator (km, geodetic normals);
                         wgs84_ellipsoid + moon_ellipsoid + planet_ellipsoid
src/simulation/mod.rs    Simulation trait (UI-agnostic; camera_target() defaults
                         to Earth), SimulationState (core: clock + celestial
                         sphere) + its shared-core impl UIDrawable, RenderState
                         (moon fields + render_origin/sun_pos_world/planets),
                         SatelliteTelemetry, CameraTarget (Earth/Moon/Planet
                         orbit body + render_origin(), consumed by application's
                         Camera), TargetSelector (EARTH/MOON, eclipses),
                         BodySelector (one latching key per body, 9 bodies
                         ordered by distance from the Sun, solar_system)
src/simulation/celestial_sphere.rs  ephemeris-driven Sun + star-map orientation
                         + Moon position (DE440) and IAU lunar rotation;
                         sun_pos_world + the 7 planets' position (DE440) and IAU
                         planet rotation (PlanetState[]); iau_body_to_gcrf helper
src/simulation/satellite.rs  TLE parse + satkit SGP4 + TEME->world-km conversion
src/simulation/clock.rs  simulation Clock: wall-dt x speed, play/pause
shaders/globe.wgsl       ALL shader code (6 passes in one module: + planets; all
                         vertex passes subtract uniforms.render_origin; analytic
                         eclipse shadows). Planet uniform/texture are group 1
OUT_DIR/                 gitignored; include_bytes!'d: 13 JPEG textures (Earth x4,
                         stars, Moon, 7 planets) + 3 f16 LUT KTX2 + DE440
                         ephemeris + EOP-All.csv
```

## Module dependency graph

```
main        -> application, simulation, renderer, scenarios
application -> simulation, renderer, ui, earth, (winit, egui, egui_winit, glam)
ui          -> (egui)   # defines UIDrawable trait + control_panel
renderer    -> simulation (RenderState), earth, moon, planet, (wgpu, egui_wgpu, ktx2, glam)
simulation  -> earth, moon, planet, ui, (satkit, egui via ui, glam)  # impl UIDrawable
                                       # for SimulationState; NO winit/wgpu/Camera
earth       -> (glam)
moon        -> (glam)
planet      -> (glam)   # satkit-free body geometry, like earth/moon
scenarios   -> simulation, ui, application
```

## `Simulation` trait

Defined in `src/simulation/mod.rs`. The sole simulation interface
`ApplicationState` uses; adding a scenario requires no changes to the
application layer. It is **UI-agnostic** - the panel reads/drives a scenario
through a *separate* `ui::UIDrawable` impl, kept distinct from `Simulation`.
(The `Simulation` trait itself takes no UI types; the `simulation` module does
depend on `ui` for the shared-core `impl UIDrawable for SimulationState` that
now lives there.) `ApplicationState<S>` bounds `S: Simulation + UIDrawable`.

```
advance(&mut self) -> bool
    Tick the clock + re-evaluate the celestial sphere. Returns whether the
    clock is running (keeps frames coming; paused = app goes idle).

celestial_to_world(&self) -> Mat3
    Rotation from the inertial (star-fixed) camera rig frame to the
    Earth-fixed world frame. Called by the application before each frame to
    resolve the camera into world space.

camera_target(&self) -> CameraTarget   [defaulted: CameraTarget::Earth]
    Which body the orbital camera orbits this frame. The application reads it
    and calls Camera::retarget before resolving eye/view_proj. Earth-only
    scenarios inherit the default; the eclipse scenarios override it from a
    TargetSelector (panel-driven).

frame_state(&mut self, eye: Vec3, view_proj: Mat4) -> RenderState
    Propagate all satellites once, fill RenderState (renderer). Stashes the
    same-propagation per-satellite readout (Vec<SatelliteTelemetry>) on the
    scenario for the immediately-following get_drawables call.
```

## `UIDrawable` trait + `UIDrawablePanel` + `Instrument`

The trait + panel live in `src/ui/mod.rs`; each instrument is a struct in its
own `src/ui/instruments/*.rs` (egui-free *data* + boxed closures - egui only
enters in each instrument's `render` and in `control_panel`). Decouples panel
*rendering* from *interactivity*. The trait stays separate from `Simulation`;
the shared-core `impl UIDrawable for SimulationState` lives next to the type in
`src/simulation/mod.rs` (so `simulation` depends on `ui` for these types).

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
    track. Each control is two types: a bare struct (inert; derives Deserialize)
    and an Interactive* wrapper that owns the bare struct + a moved
    Box<dyn FnMut(..)> callback (InteractiveButton/InteractiveToggle/
    InteractiveSlider). A bare control renders inert (e.g. a deserialized mock);
    the wrapper fires its callback. Shared draw lives on the bare struct.

PanelAnchor::{ TopLeft, TopRight }   # add bottom corners when needed
```

- `impl UIDrawable for SimulationState` (in `src/simulation/mod.rs`) emits
  **one** panel (top-left) from live state: subsolar + datetime readouts, and
  the Run toggle + speed slider
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

- **`simulation` imports neither winit/wgpu nor the `Camera` type.**
  The `Simulation` trait takes resolved `Vec3`/`Mat4` values for the camera
  and returns a `RenderState`. This keeps input scheme changes local to
  `application` and each scenario's `frame_state` impl independently testable.
  `simulation` *does* depend on `ui` (hence egui, transitively) for the
  shared-core `impl UIDrawable for SimulationState`, which lives next to the
  type rather than in `ui`. The `UIDrawable`/`UIDrawablePanel`/`Instrument`
  types are still defined in `ui`, and interactivity is carried by the
  `Interactive*` wrappers (the bare instrument structs are inert), so the same
  code can drive a mock UI (bare deserialized instruments, no callbacks) with
  no live `Clock`.
- **`Camera` type lives in `application` only.** Other modules see only a
  resolved `eye`/`view_proj`. (`RenderState` is defined in `simulation` but
  consumed by `renderer`, and `CameraTarget` is defined in `simulation` but
  consumed by `application`'s `Camera` — the two allowed edges. `CameraTarget`
  is plain data: it names no `Camera`/winit/wgpu type, only the orbit body +
  its world center, with geometry accessors delegating to `earth`/`moon`. The
  scenario→application *camera* channel is the `Simulation::camera_target`
  return value, so the application still owns all camera mechanics.)
