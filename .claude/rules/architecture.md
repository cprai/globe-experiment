# Architecture & file map

## Stack

Rust edition 2024. `wgpu 29` (GPU), `winit 0.30` (window), `egui 0.34`
(overlay), `egui_taffy 0.12` (taffy flexbox layout for the panels),
`satkit 0.18` (SGP4 + ephemeris + EOP), `glam 0.33` (math),
`pyo3 0.29` (embedded CPython for the `*_py` scenes' script-produced UI
panels; unconditional - every build links libpython and needs Python 3 dev
headers), `rayon 1.10` (parallel init), `image 0.25` (texture decode + PNG
encode), `ktx2 0.5` (LUT parse/write), `humantime 2` (render-mode datetime
parse). Build-only: `ureq 3.3` (asset download), `half 2.7` (f16 LUT bake).
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
                         default-run): clap CLI with only the `scene`
                         subcommand, itself a SceneCommand subcommand enum -
                         one subcommand per scene, each wrapping that scene
                         module's own #[derive(clap::Args)] Args struct, so
                         each scene declares exactly its own arguments (only
                         the *_py scenes have --script; clap itself requires
                         it there and rejects it elsewhere - no hand-enforced
                         pairing). Bare `scene` prints the subcommand list
                         via arg_required_else_help (no hand-rolled
                         list_scenes). Declares `mod engine;` + `mod scenes;`
                         (NO offscreen/headless code)
src/headless.rs          bin root of `headless` (single-frame render to PNG; no
                         EOP range check): flat clap flags --scene --output
                         [--width --height], no subcommand; declares
                         `mod engine;` + `mod offscreen;` (NO scenes) +
                         crate-level allow(dead_code) for the engine items only
                         the main tree uses (all of engine::application, plus
                         windowed-only items in the shared modules). SceneSpec =
                         --scene JSON (simulation + camera +
                         optional ui); camera.target "terra"/"luna"/planets
                         (CameraTargetSpec, default terra); optional mock-panel
                         overlay (build_ui_frame)
src/engine/mod.rs        the engine module root, declared identically by BOTH
                         bin roots: everything used to run the app (application,
                         camera, planet, py, renderer, scene, ui).
                         The top level keeps only the bin roots, scenes
                         (main tree), and offscreen (headless tree)
src/engine/py.rs         embedded Python: Once-guarded init() (append_to_inittab
                         of the `globe` pymodule STRICTLY before
                         Python::initialize), the `globe` module registration
                         (instruments + PanelAnchor/LampStatus + ui::py types +
                         Clock/BodySelector/SatelliteTelemetry/OrbitShape; the
                         *_py scene pyclasses are NOT registered - they live in
                         src/scenes, which engine must not reference, and reach
                         Python as instances), and load_get_drawables (reads
                         the caller-given script path at RUNTIME - the *_py
                         scenes' required --script argument, no resolution
                         here -
                         and returns the script's get_drawables; load/compile
                         failure = traceback + panic). Compiled dead by the
                         headless tree (links libpython, never initializes it)
scenes/*.py              the repo-root scene-script directory (NOT src/scenes):
                         manual_control_py.py + solar_system_py.py, the
                         reference panel producers a *_py scene is pointed at
                         via its required --script path (also what the two
                         scene tests load explicitly). Read at runtime - edit +
                         relaunch, no rebuild (the one deliberate exception to
                         everything-embedded). Contract: module-level
                         get_drawables(scene) -> list[Panel]
src/scenes/mod.rs     scene registry (module decls + the shared scene
                         conventions doc; the SceneClock trait lives with
                         Clock in src/engine/scene/clock.rs)
src/scenes/iss_and_hubble.rs  IssAndHubbleScene (Scene impl); ISS_TLE/HST_TLE consts
src/scenes/iss.rs     IssScene (Scene impl); own ISS_TLE const (duplicated on purpose)
src/scenes/solar_eclipse.rs  SolarEclipseScene: empty (NO satellites);
                         clock starts from the 2024-04-08 eclipse datetime;
                         new() seeds its PtzCamera framing the Terra day side
                         (PtzCamera::looking_toward);
                         TargetSelector (default Terra) for the Terra/Luna panel
src/scenes/lunar_eclipse.rs  LunarEclipseScene: empty (NO satellites);
                         clock starts from the 2025-03-14 eclipse datetime;
                         new() seeds its PtzCamera orbiting Luna (Luna-target
                         looking_toward); TargetSelector (default Luna)
src/scenes/manual_control.rs  ManualControlScene: ONE user-thrustable
                         satellite; own ISS_TLE const (duplicated on purpose),
                         used once to seed a GCRF OrbitState (no TLE after) that
                         advance() re-anchors to the clock each frame via
                         satellite::propagate_numerical; six burn_* request
                         flags (kept - the burn is dt-scaled, only advance()
                         knows dt) set by the bottom-center Burns panel's
                         hold-to-fire keys (prograde/retrograde, normal/
                         anti-normal, radial out/in) via their `&mut Self`
                         callbacks, folded into dv = 10 m/s^2
                         * dt; marker via satellite::resolve_orbit +
                         Propagation::Numerical; apo/peri/speed readouts from
                         satellite::orbit_shape (dashes on escape)
src/scenes/manual_control_py.rs  the manual_control clone whose get_drawables
                         delegates to the CLI-given script (the repo ships
                         scenes/manual_control_py.py; side by side for API
                         comparison). ManualControlSceneInner is a
                         #[pyclass] (plain clock: Clock behind its SceneClock
                         impl, re-exposed to the script as paused/multiplier
                         getter+setter properties + datetime_label();
                         telemetry()/orbit_shape()/speed_m_s readout
                         methods; six request_* burn methods - the script's
                         hold-key callbacks) wrapped by ManualControlPyScene
                         (Py<Inner> + the stashed script fn), whose four trait
                         impls each attach + borrow the cell for their own
                         duration only (no borrow ever spans a call into
                         Python). Script errors: traceback + panic. Own
                         duplicated ISS_TLE. Holds the round-trip unit test
src/scenes/solar_system.rs  SolarSystemScene: empty (NO satellites);
                         clock starts 2025-06-01; draws all 7 planets at true
                         pos/scale; BodySelector (one key per body: Terra, Luna,
                         the 7 planets) drives camera_target; default Terra view
src/scenes/solar_system_py.rs  the solar_system clone whose get_drawables
                         delegates to the CLI-given script (the repo ships
                         scenes/solar_system_py.py); same
                         Inner-pyclass + wrapper pattern as manual_control_py
                         (plain clock behind SceneClock + the same clock
                         properties), with selector: Py<BodySelector> shared
                         live with the script (the script rebuilds the
                         9-key selector panel via selector.selected /
                         BodySelector.body_names() / selector.request(i))
src/engine/application/mod.rs   ApplicationState<S: Scene + CameraControl +
                         CameraView + UIDrawable>
                         + winit ApplicationHandler + run(). Keeps NO camera or
                         input state: translate_camera_event statelessly maps
                         each winit input event onto one device-neutral
                         CameraControl-trait call (raw positions/deltas pass
                         through), and cursor_icon maps CursorHint back onto winit
src/engine/application/gfx.rs   Gfx: the windowed presenter - GPU surface/swapchain
                         config/present + egui_wgpu overlay around the shared
                         renderer::SceneRenderer; FrameOutcome drives window
                         visibility/redraw. The winit-bound half of rendering
                         (called only by the main tree - the headless tree
                         compiles it dead; offscreen.rs is its headless twin)
src/engine/camera/mod.rs        the CameraControl + CameraView TRAIT PAIR every
                         scene implements (CameraControl = the input methods
                         with default no-op impls + tick + cursor_hint;
                         CameraView = frame_state, so a non-interactive camera
                         implements only CameraView) and the winit-free input
                         vocabulary
                         (PointerButton, ScrollDelta (Lines|Pixels), CursorHint).
                         Winit-free so BOTH bin trees build it and future
                         gamepad/touch input stays device-neutral
src/engine/camera/ptz.rs        PtzCamera: the interactive pan/tilt/zoom orbital
                         camera - the inertial-frame rig (km world space,
                         orbiting a scene-owned CameraTarget passed by ref
                         into every call that depends on the orbited body -
                         no stored target; `reframe`, called by the scene on
                         a genuine body switch, resets framing and cancels
                         in-flight animation) AND
                         all input/animation state (drag pan/tilt, flick
                         inertia, smoothed zoom glide; feel constants at file
                         top). Also ScenePtzCamera: the three-accessor hookup
                         trait (camera/camera_mut/camera_target - the
                         SceneClock pattern) whose blanket impl supplies the
                         whole CameraControl surface, so scenes write no
                         forwarding block; headless.rs constructs a PtzCamera
                         from the --scene JSON (PtzCamera::new)
src/engine/ui/mod.rs            UI module root: owns UIDrawable trait +
                         UIDrawablePanel<S> (owned/'static - never borrows the
                         scene) + PanelAnchor (egui-free data), and the egui
                         control_panel(ctx, &mut panels, &mut scene) that
                         renders panels PRE-BUILT once per frame outside
                         run_ui (discard-pass idempotency), frames each at its
                         anchored corner (theme::PANEL_INSET), lays out its
                         rows with taffy (egui_taffy): panel = flex column of
                         rows, row = flex row of instrument nodes, all
                         content-sized (+ the shared min width) - no pixel
                         positions or fixed panel boxes - and threads the
                         `&mut S` scene into whichever callback fires. Each
                         scene implements UIDrawable itself (its own Time
                         panel + scene panels). Re-exports the instrument
                         structs (bare + Interactive*<S> + the Callback<S>/
                         ValueCallback<S> aliases) + theme install_theme + the
                         spec types (PanelSet/UiPanel). PanelAnchor is also a
                         pyclass (eq) - part of the dual Rust/Python UI API.
src/engine/ui/py.rs             the Python face of the panel API: the Panel pyclass
                         (anchor + rows of instrument objects - the
                         UIDrawablePanel twin a script returns), the four
                         Interactive* script twins (same names as the Rust
                         wrappers, deliberately; each = bare instrument +
                         Py<PyAny> callable instead of a boxed closure), and
                         panels_from_python::<S> - the per-frame conversion:
                         cast chain over the 11 instrument pyclasses, bare
                         instruments clone out inert (the spec.rs path), twins
                         become the Rust Interactive*<S> with a GIL-attaching
                         closure that ignores its `&mut S` argument (the
                         script drives the scene through its own bound
                         pymethods). Callback exceptions print + continue;
                         a bad element type is a TypeError the scene fail-fasts
                         on. Holds the render-free conversion round-trip tests
src/engine/ui/instruments/mod.rs  the Instrument<S> trait (render(&mut Tui,
                         &mut S): each instrument adds its own flex node into
                         its row, owning its node style - e.g. keys grow to
                         share the row - and an interactive one hands the
                         scene to its callback) + the Callback<S>/
                         ValueCallback<S> aliases + the
                         shared `leaf` helper (top-down layout, wrap disabled);
                         one self-contained instrument STRUCT per sibling file,
                         each impl Instrument<S> for every S with its own
                         baked-in look (a
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
                         derives Deserialize; also a #[pyclass] with a #[new]
                         constructor + get/set fields - the dual Rust/Python
                         API, Slider's range surfacing in Python as a
                         (min, max) tuple since RangeInclusive has no pyo3
                         conversion) + an Interactive* wrapper holding
                         the bare struct + a moved Box<dyn FnMut> callback
                         (InteractiveButton/InteractiveHoldButton/
                         InteractiveToggle/InteractiveSlider; the Hold variant
                         fires every frame the key is held - the burn keys'
                         producer - and senses click_and_drag so the press
                         survives egui's 0.8 s max_click_duration). Shared
                         draw lives on the bare struct.
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
src/engine/planet.rs            EVERY body's data (Terra + the 7 planets + Luna;
                         there is NO separate terra or luna module), hung off
                         the CelestialBody variants: ALL[CelestialBody;7]
                         + data-driven table (triaxial radii via radii_km() -
                         rx = rz spheroids for Terra (WGS84) and the planets,
                         Luna genuinely triaxial; mean_radius_km() in f64; the
                         simple IAU rotation constants, Some for planets /
                         None for the Terra system (Terra's body frame IS the
                         world frame; Luna's full lunar series lives in
                         celestial_sphere); a Maps struct (albedo + optional
                         night/normal/specular) + has_atmosphere)
                         accessed via impl CelestialBody
                         + surface_position / geodetic_normal free fns
                         (shape-driven latitude: geodetic for spheroids -
                         bit-for-bit the old WGS84 math for Terra - and
                         parametric for triaxial Luna) + the WGS84 defining
                         consts and TERRA_MEAN_RADIUS_KM. satkit-free;
                         references scene::body for the CelestialBody
                         type
src/engine/renderer/mod.rs      the winit-free shared scene core, compiled into BOTH
                         binaries: SceneRenderer (5 pipelines: stars (full-
                         screen quad), the single body impostor shared by ALL
                         NINE bodies - Terra + 7 planets + Luna - (triaxial
                         ray trace + data-driven shading flags + generic
                         same-system eclipse occluders, IMPOSTOR_BODIES slot
                         order; the scene's only depth-writing pass), the
                         atmosphere (CPU-sized screen quad, gated on a
                         has_atmosphere body at the render origin), the
                         predicted orbit path (mitered screen-space line
                         strip, depth test-no-write), and markers;
                         reversed-Z Depth32Float buffer; NO meshes or vertex
                         buffers anywhere - every pass builds its geometry
                         from the vertex index) + the shared device/
                         depth helpers (request_adapter_device,
                         create_depth_view, depth_attachment, DEPTH_FORMAT) +
                         UiFrame + the projection consts/view_proj_reversed_z.
                         Derives all body positions from RenderState.time via
                         CelestialSphere::at and rebuilds view_proj from the
                         camera rig; SGP4-propagates each marker's TLE one
                         period ahead (satellite::orbit_path_inertial) for the
                         path. Impostor bodies use a separate group-1 bind
                         group (per-body impostor uniform + albedo + optional
                         night/normal/specular maps, shared 1x1 dummies when
                         absent). Gfx does NOT live here anymore (it is
                         winit-bound -> application/gfx.rs)
src/offscreen.rs         OffscreenRenderer: surfaceless Rgba8Unorm offscreen
                         render + readback (+ matching depth buffer) around the
                         shared SceneRenderer; owns MAX_FRAME_DIMENSION. The
                         headless bin's presenter (its tree only; the windowed
                         twin is application/gfx.rs)
src/engine/scene/body.rs   the celestial-body hierarchy: CelestialBody identity
                         enum (TerraSystem(TerraSystemEntity Terra|Luna), then
                         each planet Mercury..Neptune as its own variant) +
                         total geometry accessors (name/mean_radius/surface/
                         normal; planet data hangs off these variants in
                         src/engine/planet.rs) + same_system (the generic
                         mutual-eclipse rule: same-system bodies shadow each
                         other - the renderer builds impostor occluder lists
                         from it), Placement (pos+rot), BodyState (identity
                         + placement). The shared vocabulary for the celestial
                         sphere, CameraTarget, and the selectors
src/engine/scene/mod.rs    Scene trait (UI- and camera-agnostic; just
                         advance(); the clock lives directly in each scene
                         struct - the celestial sphere is NOT stored anywhere:
                         CelestialSphere::at is a pure function of time,
                         evaluated on the spot by frame_state and by the
                         renderer), RenderState
                         (time + camera rig (camera_pos/camera_look_at/camera_up)
                         + camera_target + markers (each SatelliteMarker carries
                         a satellite::Propagation - cloned TLE or GCRF state
                         vector - for the renderer's orbit-path propagation) -
                         the renderer derives the
                         rest from time), SatelliteTelemetry, CameraTarget (enum:
                         Body(CelestialBody) | Coordinate(DVec3) - a pure
                         identity; center_world()/render_origin() resolve the
                         moving center from the CelestialSphere on demand),
                         TargetSelector (Terra/Luna, eclipses), BodySelector (one
                         latching key per body, 9 bodies ordered by distance from
                         Sol, solar_system; both selectors' panel<S>(access)
                         builders take an accessor closure re-finding the
                         selector in the scene, and each key callback writes
                         the selection directly - frame_state resolves it next
                         frame; BodySelector is also a #[pyclass] - selected
                         getter + body_names() staticmethod + request(index),
                         the pymethod twin of a key's direct write, so a *_py
                         scene's script can rebuild the panel).
                         SatelliteTelemetry is a
                         #[pyclass(get_all)] readout
src/engine/scene/celestial_sphere.rs  ephemeris-driven Sol + star-map orientation
                         + Luna position (DE440) and IAU lunar rotation;
                         sol_pos_world + the 7 planets' position (DE440) and IAU
                         planet rotation, all assembled into bodies:
                         Vec<BodyState> (Terra, Luna, 7 planets in planet::ALL
                         order); iau_body_to_gcrf helper. Called by the renderer
                         each frame (keyed on RenderState.time)
src/engine/scene/satellite.rs  TLE parse + satkit SGP4 + TEME->world-km conversion
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
                         (osculating apo/peri/speed, None for e >= 1;
                         OrbitShape is a #[pyclass(get_all)] readout); plus a
                         render-free circular-LEO unit test of that pipeline
src/engine/scene/clock.rs  simulation Clock (wall-dt x speed, play/pause) +
                         the SceneClock trait every scene implements. Clock
                         is plain data (new() + ALL-private fields); the
                         whole clock API AND its logic are SceneClock's
                         default methods (required clock_mut() -> &mut Clock
                         per scene; tick_clock/clock_now/
                         clock_datetime_label/clock_paused/set_clock_paused/
                         clock_multiplier/set_clock_multiplier), which as
                         same-module code are
                         the only thing that can reach the fields - scene
                         code never touches Clock internals, compiler-
                         enforced. The Time panel's callbacks call the
                         setters directly (they receive the scene as `&mut
                         Self` at fire time; keep them snapshot-idempotent -
                         see the trait doc). Clock is still a #[pyclass], but
                         registered only for the MIN/MAX_MULTIPLIER
                         classattrs (a script's slider bounds): no Clock
                         instance crosses into Python - the *_py scenes
                         re-expose the clock as scene-pyclass properties
shaders/scene.wgsl       ALL shader code (5 passes in one module: a single
                         distance-adaptive body impostor for all 9 bodies
                         (perspective/orthographic ray trace, writes
                         frag_depth, data-driven BODY_FLAG_* shading up to the
                         full Terra look); the atmosphere + stars as screen
                         quads (per-fragment eye ray via inv_view_proj); the
                         orbit path (vs_path/fs_path, mitered constant-pixel-
                         width line); markers; analytic eclipse shadows).
                         Planet uniform/maps are group 1
OUT_DIR/                 gitignored; include_bytes!'d: 13 textures (11 JPEG +
                         2 TIFF: Terra x4, stars, Luna, 7 planets) + 3 f16 LUT
                         KTX2 + DE440 ephemeris + EOP-All.csv + 3 IERS tables +
                         EGM96.gfc gravity coefficients
```

## Module dependency graph

Two bin roots over the one shared `engine` (no lib crate); the trees differ
only in their top-level extras:

```
main (bin globe-experiment) -> engine, scenes (NO offscreen/headless code)
headless (bin headless)     -> engine, offscreen (NO scenes; compiles
                                       # engine::application dead - covered by
                                       # its crate-level allow(dead_code))
engine      = application, camera, planet, py, renderer, scene, ui
                                       # - declared identically by both roots
application -> camera (the CameraControl/CameraView traits + their input
                                       # types, NOT PtzCamera),
                                       # scene (Scene trait +
                                       # RenderState only - NO CelestialSphere
                                       # access anymore), renderer, ui, (winit,
                                       # egui, egui_winit). Contains gfx.rs
                                       # (the windowed Gfx presenter around
                                       # renderer's SceneRenderer)
camera      -> scene (CameraTarget + CelestialSphere + RenderState),
                                       # renderer::FOV_Y_DEG, (glam)  # winit-free
offscreen   -> renderer (SceneRenderer + shared device/depth helpers + UiFrame),
                                       # scene (RenderState), (wgpu,
                                       # egui_wgpu, image)  # headless tree only
ui          -> (egui, egui_taffy, pyo3)   # defines UIDrawable trait +
                                       # control_panel; the bare instruments are
                                       # pyclasses and ui::py holds the Panel/
                                       # Interactive* twins + panel conversion
py          -> ui (class registration + ui::py types), scene (Clock,
                                       # BodySelector, SatelliteTelemetry,
                                       # satellite::OrbitShape), (pyo3)
                                       # interpreter init + the `globe` module +
                                       # the runtime script loader; never
                                       # references src/scenes. Compiled dead by
                                       # the headless tree (links libpython,
                                       # never initializes it)
renderer    -> scene (RenderState + CelestialSphere::at),
                                       # planet, (wgpu, egui_wgpu, ktx2, glam).
                                       # winit-free (Gfx moved to application);
                                       # derives all body geometry from
                                       # RenderState.time itself (so it pulls in
                                       # satkit transitively at runtime).
scene       -> planet, ui, (satkit, egui via ui, glam, pyo3)  # selector
                                       # panel builders use ui; NO winit/wgpu/
                                       # camera types; pyo3 only for the
                                       # Clock/BodySelector/readout pyclasses
planet      -> scene::body (CelestialBody), (glam)   # satkit-free; hangs
                                       # EVERY body's data (Terra + 7 planets
                                       # + Luna) off the CelestialBody variants
                                       # (mutual ref with scene::body)
scenes      -> scene, ui, application, camera (CameraView trait + PtzCamera +
                                       # ScenePtzCamera, whose blanket impl
                                       # supplies CameraControl), (clap - each
                                       # scene module owns its CLI Args
                                       # struct); the *_py scenes
                                       # also -> py (init + script loader) and
                                       # ui::py (panel conversion), (pyo3)
```

## `Scene` trait

Defined in `src/engine/scene/mod.rs`. One of the four traits every
scene implements (`ApplicationState<S>` bounds
`S: Scene + CameraControl + CameraView + UIDrawable`); adding a scene
requires no changes
to the application layer. It is **UI- and camera-agnostic** - the panel
reads/drives a scene through a *separate* `ui::UIDrawable` impl, and the
frame's `RenderState` comes from the scene's *separate* `camera::CameraView`
impl. (The `Scene` trait itself takes no UI or camera types; the
`scene` module does depend on `ui` for the selector panel builders,
`TargetSelector::panel` / `BodySelector::panel`.)

```
advance(&mut self) -> bool
    Tick the clock (plus any scene-specific per-frame state, e.g.
    manual_control's orbit re-anchor). Returns whether the clock is running
    (keeps frames coming; paused = app goes idle). The celestial sphere is
    not touched here - frame_state re-derives it at the frame's clock
    instant.
```

## `CameraControl` + `CameraView` traits + `PtzCamera`

The camera interface is a trait PAIR: **`CameraControl`** (the interactive
surface - the input methods, `tick`, `cursor_hint`; every method has a no-op
default) and **`CameraView`** (`frame_state`, the frame production). Both +
the winit-free input vocabulary (`PointerButton`,
`ScrollDelta::{Lines,Pixels}`, `CursorHint::{Default,Grab,Grabbing}`) live in
`src/engine/camera/mod.rs`; the reusable interactive implementation
`PtzCamera` lives in `src/engine/camera/ptz.rs`. A scene usually embeds a
`camera: PtzCamera` field (plus the scene-owned `camera_target:
CameraTarget`) and implements **`ScenePtzCamera`** (also in `ptz.rs`) - three
accessors, `camera()`/`camera_mut()`/`camera_target()`, the `SceneClock`
pattern - and the blanket `impl<S: ScenePtzCamera> CameraControl for S`
forwards every input event into the embedded camera, passing the
scene-owned target into the calls that depend on the orbited body. A scene
that must diverge implements `CameraControl` directly instead of
`ScenePtzCamera` (the `*_py` wrappers do: no `&mut PtzCamera` escapes their
pyclass cell's borrow guard, so `ScenePtzCamera` sits on their Inner and
each wrapper method attach+borrows and delegates to the Inner's blanket
impl) - or flies a future scripted/fixed camera type by implementing only
`CameraView` and leaving `CameraControl`'s defaults.

```
CameraControl:

pointer_press(&mut self, button) -> bool      [default: no-op, false]
pointer_release(&mut self, button) -> bool    [default: no-op, false]
pointer_move(&mut self, position, viewport_height) -> bool   [default no-op]
scroll(&mut self, delta) -> bool              [default: no-op, false]
    Device-neutral input, called by the application's stateless
    translate_camera_event (one winit event -> one call; raw pixel
    positions/deltas pass through). Presses carry NO position (winit gives
    none) - the camera uses the position last given to pointer_move; ALL
    input state, cursor tracking included, lives in the camera. Return =
    "camera changed, request a redraw". A future gamepad/touch scheme = new
    defaulted methods here + translation arms in application.

tick(&mut self, viewport_height) -> bool      [default: no-op, false]
    Advance one frame of camera animation (flick coast, zoom glide) with
    real frame time. Called at the top of every redraw, BEFORE
    Scene::advance. Returns true while another frame is needed; with a
    paused clock this reaching false is what lets the app go idle.

cursor_hint(&self) -> CursorHint              [default: CursorHint::Default]
    The scene cursor while the pointer is not over an egui panel; the
    application maps it onto winit's icon set.

CameraView:

frame_state(&mut self) -> RenderState
    Produce the frame: resolve the scene-owned camera_target (selector
    scenes refresh it from their selector, calling PtzCamera::reframe on a
    genuine body switch - which resets framing and cancels in-flight
    animation; satellite scenes keep it fixed at Terra), resolve the rig
    against the celestial sphere evaluated on the spot at the frame's clock
    instant (let sphere = CelestialSphere::at(&now) - `at` is a pure
    function of time, so no sphere is stored; world_rig(&target, &sphere,
    ..)), propagate all satellites, fill RenderState (time +
    rig + camera_target + markers). The immediately-following
    get_drawables call re-derives its readouts at the same clock instant
    (Clock::now() is pure, propagation deterministic), so they match the
    rendered markers with no stashed state. The camera_target packed into
    RenderState MUST be the same one the rig was built for.
```

Per-frame application order: `tick` -> `Scene::advance` ->
`frame_state` (idle invariant: redraw is re-requested only while
`tick() || advance()` reports motion, or egui asks for a repaint).

## `UIDrawable` trait + `UIDrawablePanel` + `Instrument`

The trait + panel live in `src/engine/ui/mod.rs`; each instrument is a struct in its
own `src/engine/ui/instruments/*.rs` (egui-free *data* + boxed closures - egui only
enters in each instrument's `render` and in `control_panel`). Decouples panel
*rendering* from *interactivity*. The trait stays separate from `Scene`;
each scene implements it itself, building its own Time panel from its
directly-held clock plus its scene panels.

```
UIDrawable::get_drawables(&mut self) -> Vec<UIDrawablePanel<Self>>
    The anchored panels for one frame. Called ONCE per frame, BEFORE the
    egui run_ui (application::redraw / the headless build_ui_frame / the
    test harnesses all do this) - rebuilding inside run_ui would refresh
    the callbacks' build-time snapshots on egui's discard pass and break
    their idempotency.

UIDrawablePanel<S> { anchor: PanelAnchor,
                     rows: Vec<Vec<Box<dyn Instrument<S>>>> }
    A panel owns only its corner anchor (inset by theme::PANEL_INSET). It is
    fully OWNED ('static - it never borrows the producing scene): an
    interactive callback receives the scene as its `&mut S` argument at fire
    time instead of capturing it, so every callback coexists AND can call
    `&mut self` scene APIs (the SceneClock setters) directly; captures are
    limited to owned build-time snapshots. Its
    size and every instrument's place are computed by taffy from `rows`
    (outer = top-to-bottom rows, inner = left-to-right instruments): a flex
    column (theme::panel_layout - stretch, MD row gap, PANEL_MIN_WIDTH) of
    flex rows (theme::row_layout - bottom-aligned, LG gap). Content-driven
    sizing; there are NO pixel positions or fixed panel boxes.

trait Instrument<S> { render(&mut self, tui: &mut Tui, scene: &mut S) }
    One struct per file impls it for every S (bare instruments ignore the
    scene): Header, Readout, DualReadout, Button, Toggle,
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
    and an Interactive*<S> wrapper that owns the bare struct + a moved
    callback receiving the live scene (Callback<S> = Box<dyn FnMut(&mut S)>;
    the slider's ValueCallback<S> adds the new f32)
    (InteractiveButton/InteractiveHoldButton/
    InteractiveToggle/InteractiveSlider; the Hold variant fires its callback
    every frame the key is held down - the burn keys). A bare control renders
    inert (e.g. a deserialized mock); the wrapper fires its callback with the
    `&mut S` control_panel threads through render. Callbacks MUST be
    idempotent (write-only or snapshot-based, never read-modify-write): the
    discard pass can fire one twice in a frame. Shared
    draw lives on the bare struct.

PanelAnchor::{ TopLeft, TopRight, BottomCenter }   # add more when needed
```

- Each scene's `impl UIDrawable` emits the **Time panel** (top-left) first,
  built from live state read through the `SceneClock` API: the UTC datetime +
  speed readouts, and the Run toggle + speed slider whose callbacks receive
  the scene as `&mut Self` at fire time and call the `SceneClock` setters
  directly (`move |scene| scene.set_clock_paused(running)` /
  `|scene, exp| scene.set_clock_multiplier(exp.exp())`). Keep those closures
  idempotent - write the build-time snapshot (the pre-click `running`),
  never flip live state they re-read: the discard pass can fire them twice
  per frame. The
  panel-building code is **deliberately duplicated per scene** (like the
  propagation loop) so each scene can diverge in what it exposes.
- After the Time panel, each scene pushes its own panel(s): top-right
  telemetry (re-propagated on the spot at the frame's clock instant, into
  owned values - deterministic, so it matches
  the rendered markers) or the
  selector panel, plus manual_control's bottom-center Burns panel. All panels
  are independently anchored - no stacking constant.
  `ui::control_panel(ctx, &mut panels, &mut scene)` renders the pre-built
  panels (framing each at its anchor, taffy rows), threading the scene into
  whichever callback fires; the caller built `panels` once per frame via
  `get_drawables` before entering run_ui.
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

Every scene struct holds the clock as a direct field (there is no shared
core struct), alongside its own satellites/selector. No scene stores a
`CelestialSphere`: `CelestialSphere::at` is a pure function of time, so
`frame_state` (and, where framing needs it, `new()`) evaluates it on the
spot - the same pattern as the renderer's per-frame `at`.
`Clock` is re-exported from `scene` so callers need not know the `clock`
submodule path.

## Purity rules (compiler-enforced)

- **`scene` imports neither winit/wgpu nor any camera type.**
  `RenderState` is plain data (time + resolved `DVec3` rig + `CameraTarget` +
  markers), produced by the scene's `camera::CameraView` impl and consumed
  by the renderer. This keeps input scheme changes local to `camera` (+ one
  translation arm in `application`) and each scene independently testable.
  `scene` *does* depend on `ui` (hence egui, transitively) for the
  selector panel builders (`TargetSelector::panel` / `BodySelector::panel`).
  The `UIDrawable`/`UIDrawablePanel`/`Instrument`
  types are still defined in `ui`, and interactivity is carried by the
  `Interactive*` wrappers (the bare instrument structs are inert), so the same
  code can drive a mock UI (bare deserialized instruments, no callbacks) with
  no live `Clock`.
- **The `CameraControl`/`CameraView` traits and `PtzCamera` live in the shared
  `engine::camera` module, winit-free** (so the `headless` binary builds the
  same rig without calling any winit code, and the traits' input vocabulary
  stays
  device-neutral). ALL camera mechanics - the rig math AND the input/animation
  state that used to be `application`'s `Controller` (drag, flick inertia,
  zoom glide, cursor tracking) - live in `PtzCamera`; `application` keeps no
  camera or input state and only translates winit events into trait calls
  (`translate_camera_event`, stateless). Other modules see only the resolved
  rig inside `RenderState`; the renderer rebuilds the projection from it via
  `renderer::view_proj_reversed_z` (the FOV/near/far projection consts also
  live in `renderer`). (`RenderState` is defined in `scene` but consumed
  by `renderer`, and `CameraTarget` is defined in `scene` but consumed by
  `camera` — the two allowed edges. `CameraTarget` is plain **identity** data:
  it names no camera/winit/wgpu type, only the orbit subject (a
  `CelestialBody` identity, or a free `Coordinate`). It does **not** store the
  body's moving center; the center is resolved from the `CelestialSphere` on
  demand via `center_world(&sphere)` / `render_origin(&sphere)`, with the
  static geometry accessors delegating through the identity to `planet`. The
  scene owns its camera outright - there is no scene→application camera
  channel anymore.)
- **`renderer` is winit-free (by convention, kept by review).** The windowed
  `Gfx` presenter (the only winit-touching render code) lives in
  `application/gfx.rs`; the headless bin's `offscreen.rs` is its surfaceless
  twin. Both wrap the shared `renderer::SceneRenderer`. Since the engine
  re-org (2026-07-03, owner-approved) both bin roots declare the whole
  `engine` — the headless tree *compiles* `engine::application` (winit
  included) but never calls it, so "headless runs no winit code" is no longer
  compiler-enforced module-by-module. What the compiler still enforces is the
  top level: the `headless` bin root must never declare `scenes`, and
  `main.rs` must never declare `offscreen`.
- **`application` does not touch the `CelestialSphere`.** (The 2026-07 camera
  re-home reinstated this: target resolution and rig resolution moved into each
  scene's `CameraView::frame_state`, which reads the scene's own sphere, so
  the old "relaxed" exception - `application` reading it via
  `Scene::celestial()` - is gone along with that method.) `application`
  consumes only the finished `RenderState`.
