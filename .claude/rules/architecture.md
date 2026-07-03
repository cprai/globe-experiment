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
                         DE440 ephemeris + EOP-All.csv + 3 IERS tables +
                         EGM96.gfc gravity coefficients into
                         OUT_DIR; bakes 3 atmosphere LUTs as f16 KTX2.
                         Contains mod atmosphere.
(no .cargo/config.toml)  deleted - was only for intel_tex_2's ISPC linkage
src/main.rs              bin root of `globe-experiment` (the windowed app;
                         default-run): clap CLI with only the `scenario <name>`
                         subcommand; declares `mod engine;` + `mod scenarios;`
                         (NO offscreen/headless code)
src/headless.rs          bin root of `headless` (single-frame render to PNG; no
                         EOP range check): flat clap flags --scene --output
                         [--width --height], no subcommand; declares
                         `mod engine;` + `mod offscreen;` (NO scenarios) +
                         crate-level allow(dead_code) for the engine items only
                         the main tree uses (all of engine::application, plus
                         windowed-only items in the shared modules). SceneSpec =
                         --scene JSON (simulation + camera +
                         optional ui); camera.target "terra"/"luna"/planets
                         (CameraTargetSpec, default terra); optional mock-panel
                         overlay (build_ui_frame)
src/engine/mod.rs        the engine module root, declared identically by BOTH
                         bin roots: everything used to run the app (application,
                         camera, luna, planet, renderer, simulation, terra, ui).
                         The top level keeps only the bin roots, scenarios
                         (main tree), and offscreen (headless tree)
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
src/scenarios/manual_control.rs  ManualControlSimulation: ONE user-thrustable
                         satellite; own ISS_TLE const (duplicated on purpose),
                         used once to seed a GCRF OrbitState (no TLE after) that
                         advance() re-anchors to the clock each frame via
                         satellite::propagate_numerical; six disjoint burn_*
                         request flags fed by the bottom-center Burns panel's
                         hold-to-fire keys (prograde/retrograde, normal/
                         anti-normal, radial out/in), folded into dv = 10 m/s^2
                         * dt; marker via satellite::resolve_orbit +
                         Propagation::Numerical; apo/peri/speed readouts from
                         satellite::orbit_shape (dashes on escape)
src/scenarios/solar_system.rs  SolarSystemSimulation: empty (NO satellites);
                         clock starts 2025-06-01; draws all 7 planets at true
                         pos/scale; BodySelector (one key per body: Terra, Luna,
                         the 7 planets) drives camera_target; default Terra view
src/engine/application/mod.rs   ApplicationState<S: Simulation> + winit ApplicationHandler + run()
src/engine/application/gfx.rs   Gfx: the windowed presenter - GPU surface/swapchain
                         config/present + egui_wgpu overlay around the shared
                         renderer::SceneRenderer; FrameOutcome drives window
                         visibility/redraw. The winit-bound half of rendering
                         (called only by the main tree - the headless tree
                         compiles it dead; offscreen.rs is its headless twin)
src/engine/application/input.rs    Controller: drag/tilt/wheel, flick inertia, smoothed
                         zoom, reset_animation (on target switch)
src/engine/camera.rs            orbital camera (inertial-frame rig, km world space)
                         orbiting a CameraTarget (Terra/Luna/planet); per-frame
                         retarget. Winit-free so BOTH bin trees
                         build the same rig (application drives it interactively;
                         headless.rs constructs it from the --scene JSON)
src/engine/ui/mod.rs            UI module root: owns UIDrawable trait + UIDrawablePanel
                         + PanelAnchor (egui-free data), and the egui
                         control_panel that frames each panel at its anchored
                         corner (theme::PANEL_INSET) and lays out its rows with
                         taffy (egui_taffy): panel = flex column of rows, row =
                         flex row of instrument nodes, all content-sized (+ the
                         shared min width) - no pixel positions or fixed panel
                         boxes (interactivity via callbacks). Each scenario
                         implements UIDrawable itself (its own Time panel +
                         scenario panels). Re-exports the instrument structs
                         (bare + Interactive*) + theme install_theme + the spec
                         types (PanelSet/UiPanel).
src/engine/ui/instruments/mod.rs  the Instrument trait (render(&mut Tui): each
                         instrument adds its own flex node into its row, owning
                         its node style - e.g. keys grow to share the row) + the
                         shared `leaf` helper (top-down layout, wrap disabled);
                         one self-contained instrument STRUCT per sibling file,
                         each impl Instrument with its own baked-in look (a
                         producer picks which instrument + content, never style)
src/engine/ui/instruments/{header,readout,dual_readout,button,toggle,lamp,slider}.rs
                         one instrument each (header.rs amber title + rule spanning
                         the panel (grown node); readout.rs digit-window readout
                         (dim label above a large cream value in an outlined
                         recessed window + optional inverted unit block) + shared
                         readout_block; dual_readout.rs two readouts; button.rs
                         momentary key (wrap disabled - a long label widens its
                         panel, not folds); toggle.rs latching green key + shared
                         key_style (keys flex-grow: a lone key fills its row,
                         paired keys split it); lamp.rs status dot + LampStatus;
                         slider.rs value track (percent-width node - the track
                         follows the panel width without driving it)). Each
                         control is split in two: a bare data struct (inert,
                         derives Deserialize) + an Interactive* wrapper holding
                         the bare struct + a moved Box<dyn FnMut> callback
                         (InteractiveButton/InteractiveHoldButton/
                         InteractiveToggle/InteractiveSlider; the Hold variant
                         fires every frame the key is held - the burn keys'
                         producer). Shared draw lives on the bare struct.
                         Click-fired InteractiveButton has no live producer yet
                         (allow(dead_code)) - the full set ships as a reusable
                         instrument library
src/engine/ui/theme.rs          install_theme: the Apollo-panel egui look (gunmetal
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
src/engine/ui/spec.rs           the serde-deserialized headless --scene `ui` overlay:
                         a tagged enum (UiElement) over the bare instrument
                         structs themselves + a UiPanel (anchor + rows of
                         elements; no pixel coordinates) + PanelSet (UIDrawable).
                         No mirror type - the bare structs derive Deserialize,
                         so each element clones into an inert boxed Instrument.
                         Constructed only by the headless bin's tree (main tree
                         allows dead_code on the module)
src/engine/terra.rs             WGS84 constants + surface_position / geodetic_normal helpers
src/engine/luna.rs              lunar constants (triaxial ellipsoid radii, mean radius)
                         + surface_position / geodetic_normal (body-fixed frame);
                         the Terra-style single source of truth for Luna geometry
src/engine/planet.rs            the 7 planets' data, hung off the CelestialBody planet
                         variants (no separate Planet enum): ALL[CelestialBody;7]
                         + data-driven table (oblate radii, IAU rotation
                         constants, texture file) accessed via impl CelestialBody
                         + surface_position / geodetic_normal free fns. satkit-
                         free, like terra/luna; references simulation::body for
                         the CelestialBody type
src/engine/renderer/mod.rs      the winit-free shared scene core, compiled into BOTH
                         binaries: SceneRenderer (7 pipelines incl. Luna, a
                         single planet impostor, and the predicted orbit path
                         (mitered screen-space line strip, depth test-no-write);
                         reversed-Z Depth32Float buffer) + the shared device/
                         depth helpers (request_adapter_device,
                         create_depth_view, depth_attachment, DEPTH_FORMAT) +
                         UiFrame + the projection consts/view_proj_reversed_z.
                         Derives all body positions from RenderState.time via
                         CelestialSphere::at and rebuilds view_proj from the
                         camera rig; SGP4-propagates each marker's TLE one
                         period ahead (satellite::orbit_path_inertial) for the
                         path. Planets use a separate group-1 bind group
                         (per-planet impostor uniform + texture). Gfx does NOT
                         live here anymore (it is winit-bound ->
                         application/gfx.rs)
src/offscreen.rs         OffscreenRenderer: surfaceless Rgba8Unorm offscreen
                         render + readback (+ matching depth buffer) around the
                         shared SceneRenderer; owns MAX_FRAME_DIMENSION. The
                         headless bin's presenter (its tree only; the windowed
                         twin is application/gfx.rs)
src/engine/renderer/mesh.rs     generic ellipsoid mesh generator (km, geodetic normals);
                         wgs84_ellipsoid + luna_ellipsoid (planets are
                         impostors, not meshes)
src/engine/simulation/body.rs   the celestial-body hierarchy: CelestialBody identity
                         enum (TerraSystem(TerraSystemEntity Terra|Luna), then
                         each planet Mercury..Neptune as its own variant) +
                         total geometry accessors (name/mean_radius/surface/
                         normal; planet data hangs off these variants in
                         src/engine/planet.rs), Placement (pos+rot), BodyState (identity
                         + placement). The shared vocabulary for the celestial
                         sphere, CameraTarget, and the selectors
src/engine/simulation/mod.rs    Simulation trait (UI-agnostic; camera_target() defaults
                         to Terra; the clock + celestial sphere live directly
                         in each scenario struct), RenderState
                         (time + camera rig (camera_pos/camera_look_at/camera_up)
                         + camera_target + markers (each SatelliteMarker carries
                         a satellite::Propagation - cloned TLE or GCRF state
                         vector - for the renderer's orbit-path propagation) -
                         the renderer derives the
                         rest from time), SatelliteTelemetry, CameraTarget (enum:
                         Body(CelestialBody) | Coordinate(Vec3) - a pure
                         identity; center_world()/render_origin() resolve the
                         moving center from the CelestialSphere on demand),
                         TargetSelector (Terra/Luna, eclipses), BodySelector (one
                         latching key per body, 9 bodies ordered by distance from
                         Sol, solar_system)
src/engine/simulation/celestial_sphere.rs  ephemeris-driven Sol + star-map orientation
                         + Luna position (DE440) and IAU lunar rotation;
                         sol_pos_world + the 7 planets' position (DE440) and IAU
                         planet rotation, all assembled into bodies:
                         Vec<BodyState> (Terra, Luna, 7 planets in planet::ALL
                         order); iau_body_to_gcrf helper. Called by the renderer
                         each frame (keyed on RenderState.time)
src/engine/simulation/satellite.rs  TLE parse + satkit SGP4 + TEME->world-km conversion
                         (marker state also carries the GCRF pos/vel as
                         OrbitState); Propagation enum (Sgp4(Box<TLE>) |
                         Numerical(OrbitState)) + orbit_path_inertial, which
                         propagates one period ahead per arm - batch SGP4, or
                         numerical satkit orbitprop (EGM96 4x4 + Sun/Moon, no
                         drag/SRP, dense-output interp_batch; empty path for a
                         non-elliptic escape state) - in the shared
                         single-rotation inertial-ellipse frame (direct
                         P-permutation to world km) for the renderer's path.
                         Also the TLE-free manual-control pipeline:
                         propagate_numerical (one orbitprop step, the per-frame
                         re-anchor), resolve_orbit (GCRF state -> the same
                         SatelliteState as the SGP4 arm), orbit_shape
                         (osculating apo/peri/speed, None for e >= 1); plus a
                         render-free circular-LEO unit test of that pipeline
src/engine/simulation/clock.rs  simulation Clock: wall-dt x speed, play/pause
shaders/scene.wgsl       ALL shader code (7 passes in one module: a single
                         distance-adaptive planet impostor (perspective/
                         orthographic ray trace, writes frag_depth); the orbit
                         path (vs_path/fs_path, mitered constant-pixel-width
                         line); analytic
                         eclipse shadows). Planet uniform/texture are group 1
OUT_DIR/                 gitignored; include_bytes!'d: 13 textures (11 JPEG +
                         2 TIFF: Terra x4, stars, Luna, 7 planets) + 3 f16 LUT
                         KTX2 + DE440 ephemeris + EOP-All.csv + 3 IERS tables +
                         EGM96.gfc gravity coefficients
```

## Module dependency graph

Two bin roots over the one shared `engine` (no lib crate); the trees differ
only in their top-level extras:

```
main (bin globe-experiment) -> engine, scenarios (NO offscreen/headless code)
headless (bin headless)     -> engine, offscreen (NO scenarios; compiles
                                       # engine::application dead - covered by
                                       # its crate-level allow(dead_code))
engine      = application, camera, luna, planet, renderer, simulation, terra,
                                       # ui - declared identically by both roots
application -> camera, simulation (incl. CelestialSphere, to resolve the camera
                                       # target's center), renderer, ui, terra,
                                       # (winit, egui, egui_winit, glam).
                                       # Contains gfx.rs (the windowed Gfx
                                       # presenter around renderer's
                                       # SceneRenderer)
camera      -> simulation (CameraTarget + CelestialSphere), terra,
                                       # renderer::FOV_Y_DEG, (glam)  # winit-free
offscreen   -> renderer (SceneRenderer + shared device/depth helpers + UiFrame),
                                       # simulation (RenderState), (wgpu,
                                       # egui_wgpu, image)  # headless tree only
ui          -> (egui, egui_taffy)   # defines UIDrawable trait + control_panel
renderer    -> simulation (RenderState + CelestialSphere::at), terra, luna,
                                       # planet, (wgpu, egui_wgpu, ktx2, glam).
                                       # winit-free (Gfx moved to application);
                                       # derives all body geometry from
                                       # RenderState.time itself (so it pulls in
                                       # satkit transitively at runtime).
simulation  -> terra, luna, planet, ui, (satkit, egui via ui, glam)  # selector
                                       # panel builders use ui; NO winit/wgpu/Camera
terra       -> (glam)
luna        -> (glam)
planet      -> simulation::body (CelestialBody), (glam)   # satkit-free; hangs
                                       # the 7 planets' data off the CelestialBody
                                       # variants (mutual ref with simulation::body)
scenarios   -> simulation, ui, application, camera
```

## `Simulation` trait

Defined in `src/engine/simulation/mod.rs`. The sole simulation interface
`ApplicationState` uses; adding a scenario requires no changes to the
application layer. It is **UI-agnostic** - the panel reads/drives a scenario
through a *separate* `ui::UIDrawable` impl, kept distinct from `Simulation`.
(The `Simulation` trait itself takes no UI types; the `simulation` module does
depend on `ui` for the selector panel builders, `TargetSelector::panel` /
`BodySelector::panel`.) `ApplicationState<S>` bounds `S: Simulation + UIDrawable`.

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

The trait + panel live in `src/engine/ui/mod.rs`; each instrument is a struct in its
own `src/engine/ui/instruments/*.rs` (egui-free *data* + boxed closures - egui only
enters in each instrument's `render` and in `control_panel`). Decouples panel
*rendering* from *interactivity*. The trait stays separate from `Simulation`;
each scenario implements it itself, building its own Time panel from its
directly-held clock plus its scenario panels.

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
    Box<dyn FnMut(..)> callback (InteractiveButton/InteractiveHoldButton/
    InteractiveToggle/InteractiveSlider; the Hold variant fires its callback
    every frame the key is held down - the burn keys). A bare control renders
    inert (e.g. a deserialized mock); the wrapper fires its callback. Shared
    draw lives on the bare struct.

PanelAnchor::{ TopLeft, TopRight, BottomCenter }   # add more when needed
```

- Each scenario's `impl UIDrawable` emits the **Time panel** (top-left) first,
  built from live state: the UTC datetime + speed readouts, and the Run toggle
  + speed slider whose callbacks mutate the live clock (each captures a
  *disjoint* clock field - `paused` vs `multiplier` - via direct field
  assignment, so both coexist with no interior mutability; do not call a
  `Clock` method in those closures, it would borrow the whole clock). The
  panel-building code is **deliberately duplicated per scenario** (like the
  propagation loop) so each scenario can diverge in what it exposes.
- After the Time panel, each scenario pushes its own panel(s): top-right
  telemetry from the stashed `last_telemetry` (a disjoint field) or the
  selector panel, plus manual_control's bottom-center Burns panel. All panels
  are independently anchored - no stacking constant.
  `ui::control_panel(&mut impl UIDrawable)` frames each panel and lays out its
  rows with taffy, firing callbacks on interaction.
- **Theme**: `ui::install_theme(ctx)` stamps the Apollo-panel look onto an egui
  `Context` and must be called once per context (both `ApplicationState::new`
  and the headless bin's `build_ui_frame` do). It also sets egui `max_passes = 2`:
  egui_taffy measures content immediate-mode and requests a discard pass when
  the layout it drew from is stale, so the settled layout needs the second
  pass to land same-frame. **All color and every metric live in `ui::theme`
  (the palette consts + the SPACE_*/FONT_*/RADIUS_*/HAIRLINE/PANEL_INSET/
  PANEL_MIN_WIDTH tokens) + each instrument's `render`** - producers pick
  instruments and group them into rows, never colors or pixels. Each
  instrument's `render` uppercases its text; `control_panel` frames each panel
  with the gunmetal `panel_frame` and paints the bevel highlight + corner
  rivets per panel.

Every scenario struct holds the clock + celestial sphere as direct fields
(there is no shared core struct), alongside its own satellites/selector.
`Clock` is re-exported from `simulation` so callers need not know the `clock`
submodule path.

## Purity rules (compiler-enforced)

- **`simulation` imports neither winit/wgpu nor the `Camera` type.**
  The `Simulation` trait takes resolved `Vec3` values for the camera rig (eye,
  look-at point, up) and returns a `RenderState`. This keeps input scheme
  changes local to `application` and each scenario's `frame_state` impl
  independently testable.
  `simulation` *does* depend on `ui` (hence egui, transitively) for the
  selector panel builders (`TargetSelector::panel` / `BodySelector::panel`).
  The `UIDrawable`/`UIDrawablePanel`/`Instrument`
  types are still defined in `ui`, and interactivity is carried by the
  `Interactive*` wrappers (the bare instrument structs are inert), so the same
  code can drive a mock UI (bare deserialized instruments, no callbacks) with
  no live `Clock`.
- **`Camera` type lives in the shared `engine::camera` module** (winit-free;
  owner-approved re-home from `application` for the two-binary split, so the
  `headless` binary builds the same rig without calling any winit code). Its
  *input mechanics* (the `Controller`, all drag/zoom/animation) stay in
  `application`; other modules see only the resolved rig (eye / look-at / up);
  the renderer rebuilds the projection from it via
  `renderer::view_proj_reversed_z` (the FOV/near/far projection consts also
  live in `renderer`). (`RenderState` is defined in `simulation` but consumed
  by `renderer`, and `CameraTarget` is defined in `simulation` but consumed by
  `camera` — the two allowed edges. `CameraTarget` is plain **identity** data:
  it names no `Camera`/winit/wgpu type, only the orbit subject (a
  `CelestialBody` identity, or a free `Coordinate`). It does **not** store the
  body's moving center; the center is resolved from the `CelestialSphere` on
  demand via `center_world(&sphere)` / `render_origin(&sphere)`, with the
  static geometry accessors delegating through the identity to
  `terra`/`luna`/`planet`. The scenario→application *camera* channel is the
  `Simulation::camera_target` return value, so the application still owns all
  camera mechanics.)
- **`renderer` is winit-free (by convention, kept by review).** The windowed
  `Gfx` presenter (the only winit-touching render code) lives in
  `application/gfx.rs`; the headless bin's `offscreen.rs` is its surfaceless
  twin. Both wrap the shared `renderer::SceneRenderer`. Since the engine
  re-org (2026-07-03, owner-approved) both bin roots declare the whole
  `engine` — the headless tree *compiles* `engine::application` (winit
  included) but never calls it, so "headless runs no winit code" is no longer
  compiler-enforced module-by-module. What the compiler still enforces is the
  top level: the `headless` bin root must never declare `scenarios`, and
  `main.rs` must never declare `offscreen`.
- **Relaxed: `application` may read the `CelestialSphere`.** The camera now
  resolves its target's center from the sphere (via `Simulation::celestial()`),
  so `application` touches `simulation`'s ephemeris-backed type and pulls in
  satkit transitively at runtime. This was a deliberate trade (owner-approved)
  to make `CameraTarget` a pure identity with a single source of truth for
  centers, rather than baking a resolved snapshot into the type. `application`
  still imports no winit-in-`simulation` / wgpu types.
