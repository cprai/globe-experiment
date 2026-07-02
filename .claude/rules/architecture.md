# Architecture & file map

## Stack

Rust edition 2024. `wgpu 29` (GPU), `winit 0.30` (window), `egui 0.34`
(overlay), `egui_taffy 0.12` (taffy flexbox layout for the panels),
`satkit 0.18` (SGP4 + ephemeris + EOP), `glam 0.33` (math),
`rayon 1.10` (parallel init), `image 0.25` (texture decode + PNG encode),
`ktx2 0.5` (LUT parse/write), `humantime 2` (render-mode datetime parse).
Build-only: `ureq 3.3` (asset download), `half 2.7` (f16 LUT bake).
Crate name: `globe-experiment`.

## File map

```
build.rs                 downloads 13 textures (JPEG/TIFF verbatim) + JPL
                         DE440 ephemeris + EOP-All.csv + 3 IERS tables into
                         OUT_DIR; bakes 3 atmosphere LUTs as f16 KTX2.
                         Contains mod atmosphere.
(no .cargo/config.toml)  deleted - was only for intel_tex_2's ISPC linkage
src/main.rs              clap CLI: `scenario <name>` | `render` subcommands
                         (render takes one --scene JSON + --output/width/height)
src/snapshot.rs          headless single-frame render mode (no EOP range check);
                         SceneSpec = --scene JSON (simulation + camera +
                         optional ui); camera.target "terra"/"luna"
                         (CameraTargetSpec, default terra); optional mock-panel
                         overlay (build_ui_frame)
src/scenarios/mod.rs     scenario registry
src/scenarios/iss_and_hubble.rs  IssAndHubbleSimulation (Simulation impl); ISS_TLE/HST_TLE consts
src/scenarios/iss.rs     IssSimulation (Simulation impl); own ISS_TLE const (duplicated on purpose)
src/scenarios/solar_eclipse.rs  SolarEclipseSimulation: empty (NO satellites);
                         clock starts from the 2024-04-08 eclipse datetime;
                         run() frames the Terra day side via Camera::looking_toward;
                         TargetSelector (default Terra) for the Terra/Luna panel
src/scenarios/lunar_eclipse.rs  LunarEclipseSimulation: empty (NO satellites);
                         clock starts from the 2025-03-14 eclipse datetime;
                         run() launches orbiting Luna (Luna-target
                         looking_toward); TargetSelector (default Luna)
src/scenarios/solar_system.rs  SolarSystemSimulation: empty (NO satellites);
                         clock starts 2025-06-01; draws all 7 planets at true
                         pos/scale; BodySelector (one key per body: Terra, Luna,
                         the 7 planets) drives camera_target; default Terra view
src/application/mod.rs   ApplicationState<S: Simulation> + winit ApplicationHandler + run()
src/application/camera.rs   orbital camera (inertial-frame rig, km world space)
                         orbiting a CameraTarget (Terra/Luna); per-frame retarget
src/application/input.rs    Controller: drag/tilt/wheel, flick inertia, smoothed
                         zoom, reset_animation (on target switch)
src/ui/mod.rs            UI module root: owns UIDrawable trait + UIDrawablePanel
                         + PanelAnchor (egui-free data), and the egui
                         control_panel that frames each panel at its anchored
                         corner (theme::PANEL_INSET) and lays out its rows with
                         taffy (egui_taffy): panel = flex column of rows, row =
                         flex row of instrument nodes, all content-sized (+ the
                         shared min width) - no pixel positions or fixed panel
                         boxes (interactivity via callbacks). The shared-core
                         impl UIDrawable for SimulationState lives in
                         src/simulation/mod.rs. Re-exports the instrument structs
                         (bare + Interactive*) + theme install_theme + the spec
                         types (PanelSet/UiPanel).
src/ui/instruments/mod.rs  the Instrument trait (render(&mut Tui): each
                         instrument adds its own flex node into its row, owning
                         its node style - e.g. keys grow to share the row) + the
                         shared `leaf` helper (top-down layout, wrap disabled);
                         one self-contained instrument STRUCT per sibling file,
                         each impl Instrument with its own baked-in look (a
                         producer picks which instrument + content, never style)
src/ui/instruments/{header,readout,dual_readout,button,toggle,lamp,slider}.rs
                         one instrument each (header.rs amber title + rule spanning
                         the panel (grown node); readout.rs digit-window readout
                         (dim label above a large cream value in an outlined
                         recessed window + optional inverted unit block) + shared
                         readout_block; dual_readout.rs two readouts; button.rs
                         momentary key; toggle.rs latching green key + shared
                         key_style (keys flex-grow: a lone key fills its row,
                         paired keys split it); lamp.rs status dot + LampStatus;
                         slider.rs value track (percent-width node - the track
                         follows the panel width without driving it)). Each
                         control is split in two: a bare data struct (inert,
                         derives Deserialize) + an Interactive* wrapper holding
                         the bare struct + a moved Box<dyn FnMut> callback
                         (InteractiveButton/InteractiveToggle/InteractiveSlider).
                         Shared draw lives on the bare struct. InteractiveButton
                         has no live producer yet (allow(dead_code)) - the full
                         set ships as a reusable instrument library
src/ui/theme.rs          install_theme: the Apollo-panel egui look (gunmetal
                         frame, monospace UPPERCASE cream readouts, green-active
                         keys, corner rivets/bevel; sets egui max_passes=2 so
                         egui_taffy's relayout discard pass settles same-frame),
                         the palette consts AND the metric tokens (SPACE_*/
                         FONT_*/RADIUS_*/HAIRLINE/PANEL_INSET/PANEL_MIN_WIDTH)
                         shared by the instruments, the taffy styles
                         (panel_layout/row_layout), and panel chrome
                         (panel_frame/bevel/rivets). Stamped onto the egui
                         Context by both the windowed app and the headless
                         render path
src/ui/spec.rs           the serde-deserialized render --scene `ui` overlay: a
                         tagged enum (UiElement) over the bare instrument structs
                         themselves + a UiPanel (anchor + rows of elements; no
                         pixel coordinates) + PanelSet (UIDrawable). No mirror
                         type - the bare structs derive Deserialize, so each
                         element clones into an inert boxed Instrument
src/terra.rs             WGS84 constants + surface_position / geodetic_normal helpers
src/luna.rs              lunar constants (triaxial ellipsoid radii, mean radius)
                         + surface_position / geodetic_normal (body-fixed frame);
                         the Terra-style single source of truth for Luna geometry
src/planet.rs            the 7 planets' data, hung off the CelestialBody planet
                         variants (no separate Planet enum): ALL[CelestialBody;7]
                         + data-driven table (oblate radii, IAU rotation
                         constants, texture file) accessed via impl CelestialBody
                         + surface_position / geodetic_normal free fns. satkit-
                         free, like terra/luna; references simulation::body for
                         the CelestialBody type
src/renderer/mod.rs      Gfx: surface/device/queue + egui_wgpu + SceneRenderer
                         (6 pipelines incl. Luna + a single planet impostor;
                         reversed-Z Depth32Float buffer). Derives all body
                         positions from RenderState.time via CelestialSphere::at
                         and rebuilds view_proj (view_proj_reversed_z) from the
                         camera rig. Planets use a separate group-1 bind group
                         (per-planet impostor uniform + texture)
src/renderer/headless.rs HeadlessRenderer: surfaceless Rgba8Unorm offscreen render
                         (+ matching depth buffer)
src/renderer/mesh.rs     generic ellipsoid mesh generator (km, geodetic normals);
                         wgs84_ellipsoid + luna_ellipsoid (planets are
                         impostors, not meshes)
src/simulation/body.rs   the celestial-body hierarchy: CelestialBody identity
                         enum (TerraSystem(TerraSystemEntity Terra|Luna), then
                         each planet Mercury..Neptune as its own variant) +
                         total geometry accessors (name/mean_radius/surface/
                         normal; planet data hangs off these variants in
                         src/planet.rs), Placement (pos+rot), BodyState (identity
                         + placement). The shared vocabulary for the celestial
                         sphere, CameraTarget, and the selectors
src/simulation/mod.rs    Simulation trait (UI-agnostic; camera_target() defaults
                         to Terra), SimulationState (core: clock + celestial
                         sphere) + its shared-core impl UIDrawable, RenderState
                         (time + camera rig (camera_pos/camera_look_at/camera_up)
                         + camera_target + markers - the renderer derives the
                         rest from time), SatelliteTelemetry, CameraTarget (enum:
                         Body(CelestialBody) | Coordinate(Vec3) - a pure
                         identity; center_world()/render_origin() resolve the
                         moving center from the CelestialSphere on demand),
                         TargetSelector (Terra/Luna, eclipses), BodySelector (one
                         latching key per body, 9 bodies ordered by distance from
                         Sol, solar_system)
src/simulation/celestial_sphere.rs  ephemeris-driven Sol + star-map orientation
                         + Luna position (DE440) and IAU lunar rotation;
                         sol_pos_world + the 7 planets' position (DE440) and IAU
                         planet rotation, all assembled into bodies:
                         Vec<BodyState> (Terra, Luna, 7 planets in planet::ALL
                         order); iau_body_to_gcrf helper. Called by the renderer
                         each frame (keyed on RenderState.time)
src/simulation/satellite.rs  TLE parse + satkit SGP4 + TEME->world-km conversion
src/simulation/clock.rs  simulation Clock: wall-dt x speed, play/pause
shaders/scene.wgsl       ALL shader code (6 passes in one module: a single
                         distance-adaptive planet impostor (perspective/
                         orthographic ray trace, writes frag_depth); analytic
                         eclipse shadows). Planet uniform/texture are group 1
OUT_DIR/                 gitignored; include_bytes!'d: 13 textures (11 JPEG +
                         2 TIFF: Terra x4, stars, Luna, 7 planets) + 3 f16 LUT
                         KTX2 + DE440 ephemeris + EOP-All.csv + 3 IERS tables
```

## Module dependency graph

```
main        -> application, simulation, renderer, scenarios
application -> simulation (incl. CelestialSphere, to resolve the camera
                                       # target's center), renderer, ui, terra,
                                       # (winit, egui, egui_winit, glam)
ui          -> (egui, egui_taffy)   # defines UIDrawable trait + control_panel
renderer    -> simulation (RenderState + CelestialSphere::at), terra, luna,
                                       # planet, (wgpu, egui_wgpu, ktx2, glam).
                                       # Derives all body geometry from
                                       # RenderState.time itself (so it pulls in
                                       # satkit transitively at runtime).
simulation  -> terra, luna, planet, ui, (satkit, egui via ui, glam)  # impl UIDrawable
                                       # for SimulationState; NO winit/wgpu/Camera
terra       -> (glam)
luna        -> (glam)
planet      -> simulation::body (CelestialBody), (glam)   # satkit-free; hangs
                                       # the 7 planets' data off the CelestialBody
                                       # variants (mutual ref with simulation::body)
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

celestial(&self) -> &CelestialSphere
    This frame's celestial sphere (Sol/Luna/planets + star matrices). The
    application reads it to map the rig into world space (celestial_to_world =
    star_rot_inv.transpose()) and to resolve the camera target's moving center.
    (The renderer separately re-derives the sphere from RenderState.time.)

camera_target(&self) -> CameraTarget   [defaulted: CameraTarget::terra()]
    Which subject the orbital camera orbits this frame. The application reads it
    and calls Camera::retarget before resolving the camera rig. Terra-only
    scenarios inherit the default; the eclipse scenarios override it from a
    TargetSelector (panel-driven).

frame_state(&mut self, camera_pos: Vec3, look_at: Vec3, up: Vec3) -> RenderState
    Propagate all satellites once, fill RenderState (the frame's time + the
    camera rig + camera_target + markers - the renderer derives Sol/Luna/planet
    geometry from the time). Stashes the same-propagation per-satellite readout
    (Vec<SatelliteTelemetry>) on the scenario for the immediately-following
    get_drawables call.
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
    The anchored panels for one frame.

UIDrawablePanel { anchor: PanelAnchor,
                  rows: Vec<Vec<Box<dyn Instrument + 'a>>> }
    A panel owns only its corner anchor (inset by theme::PANEL_INSET). Its
    size and every instrument's place are computed by taffy from `rows`
    (outer = top-to-bottom rows, inner = left-to-right instruments): a flex
    column (theme::panel_layout - stretch, MD row gap, PANEL_MIN_WIDTH) of
    flex rows (theme::row_layout - bottom-aligned, LG gap). Content-driven
    sizing; there are NO pixel positions or fixed panel boxes.

trait Instrument { render(&mut self, tui: &mut Tui) }
    One struct per file impls it: Header, Readout, DualReadout, Button, Toggle,
    Lamp, Slider. Pre-styled INSTRUMENTS, not logical primitives: a producer
    picks which instrument + its content, never its color/font/emphasis/metrics
    (style lives in each `render`, pulling the palette + SPACE_*/FONT_*/RADIUS_*
    tokens from `theme`). Each instrument adds its own flex node(s) into its
    row's tui and owns that node's flex style: Header grows across the row (its
    rule spans the panel), keys flex-grow (a lone key fills its row, paired keys
    split it), the Slider is percent-width (track follows the panel width
    without driving it), readouts stay content-sized (their fixed-width
    monospace values set the panel width). The shared `leaf` helper (in
    instruments/mod.rs) scopes plain-egui instruments (top-down layout, wrap
    disabled - a wrapping label would ratchet).
    Header=amber title+rule. Readout/DualReadout=digit-window readout(s): dim
    label above a large cream value in an outlined recessed window, with an
    optional `unit` (serde-defaulted, e.g. "km") stamped as an inverted cream
    block at the window's end - the game-UI reference look (readout_block
    shared by both).
    Button=momentary key, Toggle=latching key (lit solid green w/ dark text
    while `active`),
    Lamp=status dot keyed to LampStatus{Ok/Caution/Fault/Off}, Slider=value
    track. Each control is two types: a bare struct (inert; derives Deserialize)
    and an Interactive* wrapper that owns the bare struct + a moved
    Box<dyn FnMut(..)> callback (InteractiveButton/InteractiveToggle/
    InteractiveSlider). A bare control renders inert (e.g. a deserialized mock);
    the wrapper fires its callback. Shared draw lives on the bare struct.

PanelAnchor::{ TopLeft, TopRight }   # add bottom corners when needed
```

- `impl UIDrawable for SimulationState` (in `src/simulation/mod.rs`) emits
  **one** panel (top-left) from live state: the UTC datetime + speed readouts,
  and the Run toggle + speed slider
  whose callbacks mutate the live clock (each captures a *disjoint* clock field -
  `paused` vs `multiplier` - via direct field assignment, so both coexist with
  no interior mutability; do not call a `Clock` method in those closures, it
  would borrow the whole clock).
- Each scenario's `impl UIDrawable` returns `self.simulation.get_drawables()`
  (the core panel) plus **one** scenario panel (top-right) built from the stashed
  `last_telemetry` (a disjoint field). The two panels are independently
  anchored - no stacking constant. `ui::control_panel(&mut impl UIDrawable)`
  frames each panel and lays out its rows with taffy, firing callbacks on
  interaction.
- **Theme**: `ui::install_theme(ctx)` stamps the Apollo-panel look onto an egui
  `Context` and must be called once per context (both `ApplicationState::new`
  and `snapshot::build_ui_frame` do). It also sets egui `max_passes = 2`:
  egui_taffy measures content immediate-mode and requests a discard pass when
  the layout it drew from is stale, so the settled layout needs the second
  pass to land same-frame. **All color and every metric live in `ui::theme`
  (the palette consts + the SPACE_*/FONT_*/RADIUS_*/HAIRLINE/PANEL_INSET/
  PANEL_MIN_WIDTH tokens) + each instrument's `render`** - producers pick
  instruments and group them into rows, never colors or pixels. Each
  instrument's `render` uppercases its text; `control_panel` frames each panel
  with the gunmetal `panel_frame` and paints the bevel highlight + corner
  rivets per panel.

`SimulationState` (clock + celestial sphere) is the shared core that every
scenario struct holds by composition. Satellites belong to the scenario struct,
not to `SimulationState`. `Clock` is re-exported from `simulation` so callers
need not know the `clock` submodule path.

## Purity rules (compiler-enforced)

- **`simulation` imports neither winit/wgpu nor the `Camera` type.**
  The `Simulation` trait takes resolved `Vec3` values for the camera rig (eye,
  look-at point, up) and returns a `RenderState`. This keeps input scheme
  changes local to `application` and each scenario's `frame_state` impl
  independently testable.
  `simulation` *does* depend on `ui` (hence egui, transitively) for the
  shared-core `impl UIDrawable for SimulationState`, which lives next to the
  type rather than in `ui`. The `UIDrawable`/`UIDrawablePanel`/`Instrument`
  types are still defined in `ui`, and interactivity is carried by the
  `Interactive*` wrappers (the bare instrument structs are inert), so the same
  code can drive a mock UI (bare deserialized instruments, no callbacks) with
  no live `Clock`.
- **`Camera` type lives in `application` only.** Other modules see only the
  resolved rig (eye / look-at / up); the renderer rebuilds the projection from
  it via `renderer::view_proj_reversed_z` (the FOV/near/far projection consts
  also live in `renderer`). (`RenderState` is defined in `simulation` but
  consumed by `renderer`, and `CameraTarget` is defined in `simulation` but
  consumed by `application`'s `Camera` — the two allowed edges. `CameraTarget`
  is plain **identity** data: it names no `Camera`/winit/wgpu type, only the
  orbit subject (a `CelestialBody` identity, or a free `Coordinate`). It does
  **not** store the body's moving center; the center is resolved from the
  `CelestialSphere` on demand via `center_world(&sphere)` / `render_origin(&
  sphere)`, with the static geometry accessors delegating through the identity
  to `terra`/`luna`/`planet`. The scenario→application *camera* channel is the
  `Simulation::camera_target` return value, so the application still owns all
  camera mechanics.)
- **Relaxed: `application` may read the `CelestialSphere`.** The camera now
  resolves its target's center from the sphere (via `Simulation::celestial()`),
  so `application` touches `simulation`'s ephemeris-backed type and pulls in
  satkit transitively at runtime. This was a deliberate trade (owner-approved)
  to make `CameraTarget` a pure identity with a single source of truth for
  centers, rather than baking a resolved snapshot into the type. `application`
  still imports no winit-in-`simulation` / wgpu types, and the `Camera` type
  still lives only in `application`.
