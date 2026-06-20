# MEMORY.md

Technical reference for **Globe**, an **astronomically-accurate satellite
simulation tool** built on a Google-Earth-style globe renderer:
architecture, subsystem behavior, the rendering and atmosphere math, exact
constants, and the project's history. Companion file: `CLAUDE.md` holds the
rules, conventions, and constraints (what you must / must not do). This file
is the "how and why."

**Scope:** the tool only ever simulates **past** scenarios (events before the
build date) — never live or future times. This is what makes full
Earth-orientation accuracy attainable (a past date's EOP is a fixed record).
See §16.8 (init) and §16.9 (accuracy & the valid time range). Scenarios are a
planned feature (none exist yet; the embedded TLE is the stand-in).

**Constants drift between sessions** — the values quoted here are a
snapshot (2026-06-18). Always read the source for live values; the §
"Live constant snapshot" at the end lists where each lives.

---

## 1. What it is, the stack, the file map

An astronomically-accurate satellite simulation tool on a Google-Earth-style
3D globe renderer (past scenarios only — see the scope note above). Rust
edition 2024. The crate is
named `globe-experiment` (formerly `iced-test-app`, until iced was
removed in phase 2). Feature set: day/night Earth with procedural city lights,
normal-mapped terrain relief, GGX ocean sun-glint, Hillaire-2020
precomputed-LUT atmospheric scattering, star + sun backdrop, orbital
camera with pan/tilt/zoom + flick inertia + smoothed zoom, two egui
sliders driving the sun position.

### Dependencies (`Cargo.toml`)

Runtime:
- `wgpu = "29"` — GPU (direct dep; phase 1 used iced's wgpu-27 re-export).
  Version forced by `egui-wgpu` 0.34, which needs `wgpu ^29.0.1`.
- `winit = "0.30"` — window + event loop (egui-winit needs `^0.30.13`).
- `egui = "0.34.3"`, `egui-wgpu = "0.34.3"`, `egui-winit = "0.34.3"` —
  sun-slider overlay + its wgpu paint and winit input bridges.
- `pollster = "0.4"` — blocks on the async wgpu adapter/device requests.
- `glam = "0.33.1"` — camera/sun math (Mat3/Mat4/Quat/Vec3).
- `bytemuck = "1.25" (derive)` — Pod casts for vertex/uniform data.
- `ktx2 = "0.5"` — parses the build-produced KTX2 containers at runtime.
- `rayon = "1.10"` — parallelizes `GlobeRenderer::new` (the scene builder inside `Gfx::init`).
- `satkit = "0.18"` (`default-features = false`) — SGP4 propagation of the
  station TLE (`sgp4` + `qteme2itrf` + `ITRFCoord`) **and** the
  JPL DE440 ephemeris for the Sun (`jplephem::geocentric_pos`) + Earth
  orientation. Defaults are off to drop its `download` feature (we download the
  data ourselves in build.rs) and `omm-xml`. **Two satkit data files are
  bundled** (embedded via `include_bytes!`, loaded via `init_from_bytes`): the
  DE440 ephemeris binary (`linux_p1550p2650.440`) and CelesTrak's `EOP-All.csv`
  (real Earth-orientation params — polar motion + UT1-UTC). The satellite's
  `qteme2itrf` consumes the real EOP (sub-arcsec); the Sun/star backdrop uses
  the `*_approx` transforms (~1 arcsec). Note: frame transforms *read* satkit's
  global EOP table on first use, which would create a stray `satkit-data` dir;
  `celestial_sphere::init_satkit` pre-seeds the table to suppress that — see §16.8. Pulls
  `numeris` (its linear-algebra crate; `Vector3`/`Quaternion`/`DMatrix`)
  transitively.

Build-dependencies:
- `ureq = "3.3"` — asset download.
- `image = "0.25"` (default-features off, `jpeg` + `tiff`) — decodes the
  downloaded source textures for transcoding.
- `intel_tex_2 = "0.5"` — ISPC BC7 encoder.
- `ktx2 = "0.5"` — writes the KTX2 containers (same crate as runtime, so
  writer and reader cannot drift).
- `half = "2.7" (bytemuck)`, `bytemuck = "1.25"` — the atmosphere LUT bake
  produces f16 texels.

`image` and `half` are **build-only** now (runtime decodes nothing).

Profile overrides: `[profile.dev.package.X] opt-level = 3` for `image`,
`zune-jpeg`, `zune-core`, `tiff`, `miniz_oxide`, `weezl` — they speed up
the build-script transcode decode (5 × 33 MP images). They no longer
affect runtime.

### File map

```
build.rs                 download 5 textures -> BC7/KTX2 transcode +
                         bake 3 atmosphere LUTs to f16 KTX2, all into
                         OUT_DIR; also download the JPL DE440 ephemeris into
                         data/. Contains inline `mod atmosphere`.
.cargo/config.toml       Linux-only -lstdc++ for intel_tex_2's ISPC objs
src/main.rs              tiny: clap CLI parse (`scenario <name>` subcommand),
                         dispatch to the matching scenarios::*::run
src/scenarios/mod.rs     scenario registry (one module per past scenario)
src/scenarios/iss_and_hubble.rs  ISS+HST scenario: owns the ISS_TLE/HST_TLE
                         consts; run() seeds satkit (simulation::init), assembles
                         the Satellite array from them, builds SimulationState +
                         ApplicationState, application::run
src/scenarios/iss.rs     ISS-only scenario: same as iss_and_hubble minus HST;
                         owns its own ISS_TLE const (duplicated on purpose)
src/application/mod.rs   ApplicationState + winit ApplicationHandler + run():
                         window, egui logic side (Context + egui_winit::State),
                         camera, controller, frame orchestration
src/application/camera.rs  orbital camera (km world space, inertial-frame rig)
src/application/input.rs   Controller: drag/tilt/wheel, flick inertia,
                         smoothed zoom
src/ui.rs                egui control panel (clock + readouts, no sliders);
                         control_panel(ctx, &TelemetryState, &mut Clock)
src/earth.rs             WGS84 + Earth physical constants (axes, eccentricity,
                         mean radius, GM, rotation) + surface_position /
                         geodetic_normal helpers. Top-level shared module, the
                         single source of truth; mesh + camera both call it.
src/renderer/mod.rs      Gfx: surface/device/queue/config + egui_wgpu +
                         private GlobeRenderer scene; init/resize/viewport/
                         update; FrameOutcome, UiFrame
src/renderer/mesh.rs     WGS84-ellipsoid generator (km, with geodetic normals)
src/simulation/mod.rs    SimulationState (clock+satellite+celestial_sphere), RenderState +
                         TelemetryState, advance / celestial_to_world /
                         frame_state -> (RenderState, TelemetryState);
                         marker_occluded; init() wrapper for satkit seeding
src/simulation/celestial_sphere.rs    ephemeris-driven Sun dir + star-map orientation
                         (JPL DE440 + GCRF<->ITRF); embeds + loads the ephemeris
src/simulation/satellite.rs  one Satellite per tracked object: TLE parse +
                         satkit SGP4 -> world-space km marker pos; state_at(time)
                         propagates on demand (no stored pos); from_tle ctor.
                         Element-set agnostic - TLEs live in the scenario
src/simulation/clock.rs  simulation Clock: wall-dt x speed, play/pause
shaders/globe.wgsl       ALL shader code (4 passes in one module)
OUT_DIR/                 gitignored build artifacts, include_bytes!'d: 5 BC7
                         textures + 3 f16 LUTs (*.ktx2) + the verbatim JPL DE440
                         ephemeris (linux_p1550p2650.440) + EOP-All.csv. The
                         source textures are downloaded+transcoded in memory and
                         never hit disk (the .ktx2 is the cache); only the two
                         verbatim embeds are stored as-is. No assets/ dir. The
                         satellite TLEs are NOT here - they're inline source
                         literals (ISS_TLE/HST_TLE in scenarios/iss_and_hubble.rs)
CLAUDE.md, MEMORY.md     the docs (this consolidation)
```

Note: `src/globe/atmosphere.rs` **no longer exists** — the bake source was
inlined into `build.rs` as `mod atmosphere` (2026-06-13). The whole `src/globe/`
tree is also gone: a 2026-06-19 refactor split it into the top-level
`application` / `simulation` / `renderer` / `ui` / `earth` modules.

---

## 2. Phase history (timeline)

- **Phase 1** (to 2026-06-11): the globe, embedded in an **iced 0.14** app
  via the `iced::widget::shader` widget. All the rendering/atmosphere math
  was designed here. iced created the device with `Features::empty()`,
  which blocked compressed textures; textures were decoded at runtime in
  `Pipeline::new` (lazy, first frame); the atmosphere LUTs were baked on
  the CPU at startup.
- **Phase 2** (2026-06-11 → 06-13): **removed iced entirely**, rebuilt the
  host on winit + raw wgpu + egui. Then three optimizations: build-time
  **BC7/KTX2 texture transcode**, build-time **atmosphere LUT bake**, and a
  **hidden-until-ready window**. Heavily iterated the **smoothed-zoom**
  controller. Later (06-13) inlined the LUT bake into `build.rs`, deleted
  `atmosphere.rs`, and **parallelized `GlobeRenderer::new` with rayon**.
  The rendering output stayed pixel-identical to phase 1.
- **Phase 3** (06-13 → 06-15, +06-17 update): **shader-only** change to
  `fs_main`. Dropped the photographic night map as *color*; the whole
  globe is day-mapped and darkened by sun geometry, with **procedural city
  lights** (luminance mask + fixed-grain 3D-noise dither-dissolve +
  additive yellow glow). 06-17 added `EMISSIVE_FADE_END` (lights bleed
  slightly onto the daylit side), converted all source to **ASCII-only**,
  and removed the per-file BC7 transcode cache guard.
- **Phase 4** (2026-06-18): **physical units.** World space moved from a
  unit sphere in "globe radii" to the **WGS84 reference ellipsoid in
  kilometers**, to host real-scale orbital simulation. New `earth` module
  holds the WGS84 + dynamics constants and the `surface_position` /
  `geodetic_normal` helpers. The mesh vertex gained an explicit geodetic
  **normal** (position is no longer `normalize(position)` on an ellipsoid);
  the camera, projection, and star/atmosphere shells all moved to km. The
  atmosphere scattering model stayed spherical (LUTs unchanged, no rebake).
  The look-tuning constants are untouched and the rendition is intended to
  be visually identical.
- **Phase 5** (2026-06-18): **satellite tracking.** Added the `satkit` crate
  and a `satellite` module that parses an embedded TLE (the ISS; originally
  `assets/TLE.txt` via `include_str!`, inlined as a source literal on
  2026-06-19, later the `ISS_TLE` const in Phase 11), propagates it with
  satkit's SGP4 to a fixed datetime (the TLE epoch),
  converts TEME→ITRF→geodetic, and reconstructs a world-space km point via the
  WGS84 helpers. A 4th render pass draws a constant-pixel marker circle at the
  station's projected position (hidden on the CPU when the globe occludes it),
  and the egui panel shows the datetime + sub-satellite lat/lon/altitude.
- **Phase 6** (2026-06-18): **simulation clock.** Added `clock.rs` (`Clock`):
  time starts at the TLE epoch and advances by the wall-clock delta between
  redraws x a multiplier (1x real time .. 100x, exponential), with play/pause. Each running
  frame propagates the satellite on demand (`Satellite::state_at`), so the
  marker moves. UI gained a play/pause button + speed slider (+ live multiplier);
  the displayed datetime now comes from the clock. A running clock is another
  "animating" redraw source; per the owner it **starts playing**, so the app
  renders continuously from launch and only idles when paused.
- **Phase 7** (2026-06-18): **ephemeris-driven Sun & celestial sphere.** Replaced the
  slider-driven `Sun` with `celestial_sphere.rs` (`CelestialSphere`): the Sun direction comes from the
  JPL DE440 ephemeris (`jplephem::geocentric_pos`) and the star map is oriented
  by Earth's real GCRF↔ITRF attitude (`q*_approx`, EOP-free), both for the
  clock's time and updated every running frame alongside the satellite. This
  **reverses** the earlier deliberately-non-physical "celestial sphere attached to the sun"
  rule (owner-requested). build.rs downloaded the DE440 file into `data/` and
  the app pointed satkit there via `set_datadir` (**superseded in Phase 9** -
  the ephemeris is now embedded). The Sun lat/lon sliders are
  gone; the panel shows the computed subsolar point read-only. The shader and
  its uniforms are unchanged - only the values of `sun_dir`/`star_rot_inv`.
- **Phase 8** (2026-06-18): **inertial camera.** The camera's orbital rig is
  now built in the celestial frame and rotated into the Earth-fixed world by
  `celestial_to_world = celestial_sphere.star_rot_inv.transpose()` (since the 2026-06-19
  refactor, in `ApplicationState::redraw` via `SimulationState::celestial_to_world`,
  applied with `camera.view_proj(aspect, c2w)`/`eye(c2w)`). So the camera holds still
  relative to the stars and the globe spins beneath it; `Camera`'s lon/lat are
  now an inertial look direction, not geography. Owner-requested.
- **Phase 9** (2026-06-19): **embedded ephemeris + offline EOP.** The DE440
  binary is no longer a runtime side file. build.rs downloads it straight into
  `OUT_DIR` and embeds it verbatim (see Phase 12 for the final layout);
  `celestial_sphere.rs` embeds it with `include_bytes!` and loads it into satkit's singleton
  via `jplephem::init_from_bytes` (satkit 0.18.1 entry point for embedded use).
  `set_datadir` and the `data/` dir are gone, so the binary is self-contained
  (~+98 MB) with no runtime data dependency. `init_data_dir` renamed
  `init_satkit`. **Also** (2026-06-19): running the binary was creating an empty
  `satkit-data` dir next to it — satkit's global EOP table lazily resolves a
  data dir on first read (and *creates* it), and that read happens on every
  frame transform, even the EOP-free `*_approx` ones (`gmst` → UT1 conversion)
  and `qteme2itrf` (polar motion). Fix: `init_satkit` now also pre-seeds the
  EOP singleton with an empty table (`earth_orientation_params::init_from_bytes`
  of a header-only CSV) + `disable_eop_time_warning()`, which consumes satkit's
  one-shot lazy load so the dir is never created; lookups still return zeros, so
  the result is numerically identical. (The **empty** seed is **superseded in
  Phase 10** — real EOP is now bundled.) Owner-requested.
- **Phase 10** (2026-06-19): **real EOP bundled** + project reframed as an
  astronomically-accurate, past-only satellite simulation tool. CelesTrak's
  `EOP-All.csv` is now downloaded in build.rs (the `EMBEDS` table, alongside the
  ephemeris) straight into `OUT_DIR`, embedded via `include_bytes!`
  as `EOP`, and loaded with `earth_orientation_params::init_from_bytes` (same
  call site as the old empty seed, so the no-`satkit-data`-dir guarantee
  stands). Effect: the satellite's `qteme2itrf` now applies **real** polar
  motion + UT1-UTC (sub-arcsec); the Sun/star backdrop still uses the
  `*_approx` transforms (which pick up real UT1-UTC via `gmst` but not polar
  motion, ~1"). Going full IERS-2010 for the celestial sphere would also need the IERS
  nutation tables (Tab5A/5B/5D) bundled — not done. Valid EOP range ≈ 1962 →
  build date; past-only keeps every scenario in range and the snapshot valid
  forever. Owner-requested.
- **Phase 11** (2026-06-19): **multiple satellites.** `SimulationState` now
  holds a `Vec<Satellite>` instead of one, and the tracked array is **assembled
  in `main`** (`vec![Satellite::from_tle(ISS_TLE), Satellite::from_tle(HST_TLE)]`)
  and passed to `SimulationState::new(satellites)` (clock starts at the first
  satellite's epoch; empty list panics). `Satellite::load`/`TLE_TEXT` became
  `Satellite::from_tle(text)` + `pub const ISS_TLE`/`HST_TLE` (HST flagged
  approximate: real orbit shape, made-up phase). `frame_state` loops the
  satellites, building a per-object `Vec` in both `RenderState.markers`
  (`SatelliteMarker { position_km, visible }`) and `TelemetryState.satellites`
  (`SatelliteTelemetry { name, lat, lon, alt }`); `RenderState` lost `Copy`. The
  renderer draws all markers in **one instanced draw** from a per-instance
  marker buffer (position + visible flag) that grows on demand; `sat_pos` left
  the uniform block. The UI panel lists one block per satellite under a shared
  datetime line. Owner-requested.
- **Phase 12** (2026-06-20): **no `assets/` dir; everything in `OUT_DIR`.**
  build.rs no longer creates or uses a gitignored `assets/` directory.
  `embed_verbatim` (ephemeris + EOP) downloads straight into `OUT_DIR` (those
  must be on disk for `include_bytes!`; `cargo::rerun-if-changed` points at the
  `OUT_DIR` copy, so deleting one re-downloads it) and no longer does a separate
  copy. **Textures** are now downloaded **into memory** and decoded+BC7-encoded
  in a single pass (`transcode(asset, out_dir)` via the shared `download()`
  helper + `image::load_from_memory`) - the source image **never hits disk**.
  The `.ktx2` output is the cache: `cargo::rerun-if-changed` points at it and an
  existing output short-circuits the download+transcode (the unconditional-
  re-encode guard from 2026-06-17 is back, of necessity - with no on-disk source
  there is nothing to re-encode from, so refreshing a texture or changing the
  encoder settings means deleting the stale `.ktx2`). `download_if_missing` is
  gone. `/assets` removed from `.gitignore`. Owner-requested.
- **Phase 13** (2026-06-20): **scenarios module + clap CLI.** A new top-level
  `scenarios` module (`src/scenarios/`) holds one module per past scenario, each
  with a `run()`. The first is `scenarios::iss_and_hubble`, which now owns the
  setup that used to live in `main`: `simulation::init()`, assembling the
  `Vec<Satellite>` from `ISS_TLE`/`HST_TLE`, building `SimulationState` +
  `ApplicationState`, and `application::run`. The `ISS_TLE`/`HST_TLE` inline TLE
  `const`s also **moved out of `satellite.rs` into this scenario** - the
  `satellite` module is now element-set agnostic (it propagates whatever a
  scenario hands it). `main.rs` is now a pure **clap**
  CLI: `Parser` is derived **directly on an enum** `Cli` (no wrapper struct /
  separate `Subcommand` enum - each variant is a top-level subcommand), with a
  `Scenario { name: Option<ScenarioName> }` variant and a `ScenarioName`
  `ValueEnum` (`#[value(name = "iss_and_hubble")]` keeps the token snake_case),
  dispatching to the matching `run`. So
  `globe-experiment scenario iss_and_hubble` runs the scene; an unknown name is a
  clap usage error. The name is **optional**: a bare `scenario` prints the
  available scenarios (`list_scenarios`, iterating `ScenarioName::value_variants`
  so it can't drift) instead of erroring. New dep: `clap` (derive). A second
  scenario `scenarios::iss`
  (CLI token `iss`, ISS only) was then added as a clone of `iss_and_hubble` minus
  HST; its `ISS_TLE` const is **deliberately duplicated** rather than shared
  (owner's call - each scenario owns its TLE data). Owner-requested.

---

## 3. Application shell (`application` / `simulation` / `renderer`)

`main()` is tiny: it parses the CLI with **clap** (a `scenario <name>`
subcommand backed by a `ScenarioName` `ValueEnum`) and dispatches to the matching
`scenarios::*::run`; it does no setup itself. Each scenario's `run` does the
setup that used to live in `main`: `simulation::init()` (seeds satkit's globals -
the embedded DE440 ephemeris and the real bundled EOP table - before any
ephemeris/frame-transform use; thin wrapper over `celestial_sphere::init_satkit`), then it
assembles the tracked `Vec<Satellite>` from the inline TLE consts
(`Satellite::from_tle(ISS_TLE)`, `HST_TLE`), then
`SimulationState::new(satellites)`, then
`application::run(ApplicationState::new(simulation))`. `run` builds the
`EventLoop`, sets `ControlFlow::Wait`, and runs the `ApplicationState` (a winit
`ApplicationHandler`).

The 2026-06-19 refactor split the old monolithic
`main.rs`/`globe` into modules:

- **`application`** (`src/application/mod.rs`) owns the window, the egui *logic*
  side (`Context` + `egui_winit::State`), the **camera + `Controller`** (all
  input/animation), the `SimulationState`, and the `Gfx` renderer:
  `ApplicationState { camera, simulation, controller, window: Option<Arc<Window>>,
  egui_ctx, egui_state: Option<...>, gfx: Option<Gfx>, shown: bool }`. Built via
  `ApplicationState::new(simulation)`; the window/egui_state/gfx are created on
  the first `resumed()`. **No message enum / no indirection** — input mutates the
  camera directly (the phase-1 iced `Interaction` enum is long gone). The camera
  rig + `Controller` live here so a different input scheme (e.g. touch) stays
  local to this module.
- **`simulation`** (`src/simulation/mod.rs`) owns `SimulationState { clock,
  satellite, celestial_sphere }` (composition) + the astronomical math. `new()` builds the
  `Satellite` (parses TLE, runs SGP4), `Clock::new(satellite.epoch())` (clock
  starts at the TLE epoch), and `CelestialSphere::at(clock.now())`. It has **no
  winit/wgpu/egui dependency and never names the `Camera` type** — it only ever
  takes a resolved camera `eye`/`view_proj` and returns a `RenderState`.
- **`renderer`** (`src/renderer/mod.rs`) owns `Gfx` (surface/device/queue/config
  + `egui_wgpu::Renderer` + a private `GlobeRenderer` scene). The camera is *not*
  here.

The dependency direction is **acyclic** (arrows = "depends on"; `()` =
external crates):

```
main        -> application, simulation, renderer
application -> simulation, renderer, ui, earth, (winit, egui, egui_winit, glam)
ui          -> simulation, (egui)
renderer    -> simulation (RenderState), earth, (wgpu, egui_wgpu, ktx2, glam)
simulation  -> earth, (satkit, glam)        # NO winit / wgpu / egui / Camera
earth       -> (glam)
```

Two purity rules the compiler enforces once imports are clean: (1)
**`simulation` imports neither winit/wgpu/egui nor the `Camera` type** - it
deals in `glam`/`satkit` values only and takes a resolved `Vec3`/`Mat4` for the
camera, which is what makes `frame_state` testable and the touch-controls swap
local to `application`; (2) the **`Camera` type lives in `application`** and
nothing outside it references the type (other modules only ever see a resolved
`eye`/`view_proj`). Note `RenderState` is *defined in* `simulation` but
*consumed by* `renderer`, so for that POD type the edge runs renderer ->
simulation.

### `Gfx::init` — device + surface setup (easy to get wrong)

1. `Instance::new(InstanceDescriptor::new_without_display_handle())`.
2. Surface from `window.clone()`; `request_adapter` (`HighPerformance`);
   `request_device` requesting `Features::TEXTURE_COMPRESSION_BC` and
   `experimental_features: ExperimentalFeatures::disabled()`.
3. `get_default_config(...)`, then **two deliberate overrides** (both are
   `get_default_config` traps — see `CLAUDE.md` "Surface / display"):
   - **Non-sRGB surface format** via `caps.formats.iter().find(|f|
     !f.is_srgb())`. The shader writes linear color; a non-sRGB surface
     stores it raw and the display reads it as sRGB. This is what iced's
     default `web-colors` feature did in phase 1, and every look-tuning
     constant is calibrated to that darker rendition. An sRGB surface
     (hardware encode) renders visibly brighter.
   - **`present_mode = AutoVsync`**. The default takes
     `present_modes.first()`, which is **Mailbox on DX12** — unpaced
     rendering that makes scroll-zoom and inertia judder (frames follow the
     bursty input cadence; the animation loop free-runs with jittery dt).
4. `GlobeRenderer::new(&device, &queue, config.format)` — the private scene
   builder, eager, before the first frame. After the BC7/LUT build work this is
   fast: GPU upload + pipeline creation only, no decode, no bake.
5. egui: only `egui_wgpu::Renderer::new(&device, config.format,
   RendererOptions::default())` is built here. The `egui::Context` +
   `egui_winit::State` (the logic/platform side) are created in
   `ApplicationState::resumed` and live in `application`.

### Event routing (`window_event` → `handle_input`, in `application`)

Replaces iced's `stack![]` overlay capture. `CloseRequested` exits;
`Resized` calls `gfx.resize` + `window.request_redraw`; `RedrawRequested`
calls `redraw()`. Everything else → `handle_input`:
1. Feed the event to `egui_state.on_window_event(&window, &event)`
   **first**. If `response.repaint`, request a redraw. If
   `response.consumed` (pointer over panel / slider drag), **return** — the
   globe controller never sees it.
2. Else `controller.handle_event(&event, &mut camera, gfx.viewport().1)`; if
   it returns `true` (camera changed or animation started), request redraw.

### Frame (`ApplicationState::redraw`)

1. `controller.tick(&mut camera, gfx.viewport().1)` advances flick inertia
   **and** the zoom glide; returns `animating`.
2. `simulation.advance()`: `clock.tick()` advances sim time by the wall-clock
   delta x multiplier; if it ran, `celestial_sphere = CelestialSphere::at(now)` (ephemeris) is recomputed
   for `now = clock.now()`. Satellite positions are **not** propagated here -
   that happens once in `frame_state` (step 3). Returns clock-running, so
   `animating |=` it and a playing clock keeps requesting frames. (Runs
   *before* the UI now, so this frame applies the **previous** frame's
   play/pause + speed edits - a one-frame, ~16 ms delay, imperceptible. This
   ordering is what lets each satellite's single propagation feed both its
   marker and its readout; see step 3.)
3. **Resolve the camera** (in `application`, since the camera lives here):
   `c2w = simulation.celestial_to_world()` (= `celestial_sphere.star_rot_inv.transpose()`),
   `eye = camera.eye(c2w)`, `view_proj = camera.view_proj(aspect, c2w)` with
   `aspect` from `gfx.viewport()`. Then `(render_state, telemetry) =
   simulation.frame_state(eye, view_proj)`: it loops `self.satellites`, calling
   `state_at(now)` **once** per object, and that single result feeds both sides
   - `render_state` packs `view_proj`/`camera_pos`/`sun_dir`/`star_rot_inv` + a
   `Vec<SatelliteMarker>` (world pos + CPU marker-occlusion flag via
   `marker_occluded`); `telemetry` packs the subsolar lat/lon, datetime label,
   and a `Vec<SatelliteTelemetry>` (name + sub-satellite lat/lon/alt) for the UI.
4. egui: `take_egui_input` → `run_ui(|ui| ui::control_panel(ui.ctx(),
   &telemetry, &mut simulation.clock))` → `handle_platform_output`. (0.34
   deprecated `Context::run` for `run_ui`, whose closure gets a transparent
   fullscreen `&mut Ui`; the Area panel hangs off `ui.ctx()`.) `control_panel`
   *reads* the `&TelemetryState` snapshot for its readout and mutates only the
   `&mut Clock` (play/pause + speed).
4b. Reassert the grab cursor unless `egui_ctx.is_pointer_over_egui()` (egui
   resets the cursor every frame; method renamed from `is_pointer_over_area`
   in 0.34).
5. Build `UiFrame { primitives: egui_ctx.tessellate(...), textures_delta,
   pixels_per_point }`.
6. `gfx.update(&window, &render_state, ui_frame)` does the GPU frame:
   `get_current_texture()` returns the wgpu-29 `CurrentSurfaceTexture` enum
   (not `Result`): `Success`/`Suboptimal` carry the frame; `Lost`/`Outdated` →
   reconfigure + return `FrameOutcome::Reconfigured`; `Timeout` →
   `Reconfigured`; `Occluded` → `Occluded`; `Validation` → panic. On a frame:
   `GlobeRenderer::prepare(device, queue, render, viewport)` writes the uniforms
   **and** the per-satellite marker instance buffer (growing it via `device` if
   needed; `viewport` from `config`; pixel size feeds the constant-size markers) →
   `update_texture` per `textures_delta.set` → `update_buffers` → one render
   pass (clear BLACK → `globe.render` → `egui_renderer.render`) → submit egui
   commands chained with the frame encoder → `window.pre_present_notify()` →
   `present` → `free_texture` per `textures_delta.free`. Returns
   `FrameOutcome::Presented`.
7. `application` reacts to the `FrameOutcome`: `Presented`/`Occluded` reveal
   the window on the first frame (`shown` flips true); `Occluded`/`Reconfigured`
   re-request a redraw and return. The renderer never touches the window
   (visibility/redraw stay in `application`); `update` borrows `&Window` only
   for the `pre_present_notify` latency hint.
8. Request another redraw iff `animating` or egui reported a zero
   `repaint_delay`.

The single render pass needs a `RenderPass<'static>` for egui-wgpu;
`render_pass.forget_lifetime()` is the documented pattern (the pass still
ends at scope close, before the encoder finishes).

### Hidden-until-ready window

Created `with_visible(false)`; revealed via `set_visible(true)` right after
the first `frame.present()`, so the window appears with the globe already
drawn instead of blank during load. Two non-obvious requirements (both
found by the owner on Windows):
- **`resumed()` calls `self.redraw()` directly** for the first frame, not
  `request_redraw()`. Windows only generates paint messages for *visible*
  windows; a requested redraw on a hidden window never delivers, so the
  reveal code (inside `redraw`) would never run. Presenting to a hidden
  window is fine — only paint-event *delivery* needs visibility.
- **`Occluded` first-frame guard**: some backends report a still-hidden
  window as occluded from `get_current_texture`; if that happens before
  `shown`, show the window and retry rather than deadlocking invisible.

### Redraw policy

`ControlFlow::Wait` + targeted `request_redraw()` on: camera change from
input, active flick inertia or zoom glide (each frame requests the next
until it settles), **the simulation clock running** (each frame requests the
next while playing), egui zero repaint delay (slider drag), resize, and
surface lost/timeout recovery. Idle requests nothing → zero frames — but
because the clock **starts playing**, the app is non-idle from launch until
the clock is paused. (A future sun animation would just be another such
"animating" flag.)

---

## 4. Camera (`src/application/camera.rs`)

Orbital model that lives in the **inertial (star-fixed) frame**: the rig math
is the usual look-at-a-point-near-the-origin, but the result is interpreted in
the celestial frame and rotated into the Earth-fixed world at render time, so
the camera holds still relative to the stars while the Earth rotates beneath
it. `longitude`/`latitude` therefore select an **inertial** viewing direction,
not a geographic point.

- Fields: `longitude`, `latitude` (inertial view direction, degrees),
  `distance` (eye→target, **kilometers**), `tilt` (degrees off nadir; 0 =
  straight down). Defaults: `0, 0`, distance `2.0 · MEAN_RADIUS_KM` (≈ 12742
  km), tilt `0`.
- Associated consts (km, written as `<radii> · earth::MEAN_RADIUS_KM` so the
  old tuned feel is preserved exactly): `FOV_Y = 45`,
  `MIN_DISTANCE = 0.01·R` (≈ 63.7 km), `MAX_DISTANCE = 10·R` (≈ 63710 km),
  `NEAR_PLANE = 0.01·R`, `FAR_PLANE = 50·R` (≈ 318550 km), `MAX_TILT = 80`.
  Latitude clamps to `±89°`; longitude wraps via `rem_euclid`.
- `frame()` builds the rig `(eye, target, up)` in km **in the inertial
  frame**: `target = earth::surface_position(lat, lon)` and `radial =
  earth::geodetic_normal(lat, lon)` (local up). Local tangent frame `east =
  normalize(Y × radial)`, `north = radial × east`. Tilt is a quaternion
  about `east`: `eye = target + tilt·radial·distance`, `up = tilt·north` —
  increasing tilt swings the eye off straight-down and reveals the horizon
  to the north.
- `world_frame(c2w)` rotates that rig into the world frame by
  `celestial_to_world` (a `Mat3`; the origin-centered rotation transforms eye,
  target, and up alike). `view_proj(aspect, c2w)` and `eye(c2w)` take that
  rotation; `ApplicationState::redraw` passes it `SimulationState::celestial_to_world()`
  = `celestial_sphere.star_rot_inv.transpose()` (the inverse of the world→celestial
  rotation), then hands the resolved `eye`/`view_proj` to
  `SimulationState::frame_state`. This is what makes the camera
  star-fixed: in the celestial frame the rig is constant (modulo user input),
  so `star_rot_inv · relative_world = relative_celestial` stays put while the
  ECEF globe spins.
- `view_proj(aspect, c2w) = perspective_rh(45°, aspect, NEAR_PLANE, FAR_PLANE)
  × look_at_rh(world rig)`. The `_rh` variants give wgpu's 0..1 depth
  (no depth buffer is used regardless; near/far only bound clipping, and the
  star shell must fit inside `FAR_PLANE`).
- `pan_degrees_per_pixel(viewport_height)` — cursor-stable panning:
  `km_per_pixel = 2·distance·tan(fov/2)/height`, then
  `(km_per_pixel / MEAN_RADIUS_KM).to_degrees()` (one radian of surface arc ≈
  one mean Earth radius of ground distance). Used by both live drags and
  inertia, so panning tracks the cursor at any altitude.
- `clamp_distance(d) -> f32` clamps to `MIN..MAX_DISTANCE`. (Phase 2
  replaced the old `zoom(factor)` with this — the smoothed-zoom controller
  owns the distance arithmetic and only needs the clamp.)

---

## 5. Input (`src/application/input.rs`) — `Controller`

Translates winit mouse events into camera motion, mutating `&mut Camera`
directly; `handle_event` returns `bool` (needs redraw). State: `cursor`,
`drag: Option<Drag>`, `inertia: Option<Inertia>`, `zoom: Option<Zoom>`,
plus wheel-cadence tracking (`last_wheel`, `wheel_gap`).

### Pan / tilt
- **Left drag pans** (globe follows cursor): `dlon = −dx·scale`,
  `dlat = +dy·scale`, `scale = camera.pan_degrees_per_pixel`.
- **Right drag tilts**: `−dy·0.25°/px`.
- **Cursor velocity EMA**: `alpha = 1 − e^(−20·dt)` (frame-rate
  independent), tracked on every `CursorMoved` for flick detection.
- Cursor icon `CursorIcon::Grab` / `Grabbing` (set on the window).

### Flick inertia
On left release, if `speed > FLICK_SPEED (50 px/s)` and the last move was
`< FLICK_TIMEOUT (0.1 s)` ago, start coasting. `tick_coast` integrates with
real `Instant`-based dt (capped 0.1 s), pans via the same
`pan_degrees_per_pixel` scale, decays with `HALF_LIFE (0.3 s)`, stops below
`STOP_SPEED (15 px/s)`. Pressing any button cancels inertia.

### Smoothed zoom (the heavily-iterated part — tune constants, do not restructure)

Phase 1 applied each wheel event directly (`distance *= 0.9^ticks`), which
stepped visibly on a trackpad (scroll + OS-synthesized momentum tail arrive
as sparse, quantized events). The current design is a **rate-adaptive glide
with velocity bridging**. Wheel events never move the camera directly — they
move a *target* distance; `tick_zoom` eases the camera toward it each frame.

- **Log space**: `delta = ticks · ln(0.9)`. Zoom is multiplicative, so
  working in log distance keeps perceived speed uniform at any altitude.
  Target clamped via `Camera::clamp_distance`.
- **Adaptive half-life**: `wheel_gap` is an EMA (0.5 blend) of the
  inter-event gap, each sample capped at `WHEEL_GAP_CAP (0.25 s)`. The glide
  half-life = `wheel_gap.clamp(ZOOM_HALF_LIFE_MIN 0.01, ZOOM_HALF_LIFE_MAX
  0.1)`. Dense events (~10 ms, active scroll) → near-instant; sparse events
  (momentum tail, single notches) → interpolate across exactly the gap that
  would otherwise step. **An in-flight glide keeps its `tick`** — resetting
  it per event stalls the glide between dense events.
- **Velocity bridging** (fixes a stall at finger-lift, where a ~10 ms
  half-life drained the target before the momentum tail's first event):
  `Zoom.velocity` is an EMA of the rate events move the target (log-dist/s).
  `tick_zoom` keeps advancing the target at that velocity, decaying with
  `ZOOM_COAST_HALF_LIFE (0.15 s)`, stopping below `ZOOM_STOP_RATE (0.1/s)`,
  and logging each advance in `Zoom.bridged`. The next wheel event
  **repays** `bridged` — only the un-bridged remainder moves the target;
  surpluses carry forward — so while events flow the total zoom equals
  exactly what the device sent; velocity only fills delivery gaps.
- Edge cases: reversing scroll direction zeroes the coast (`velocity` and
  `bridged` reset when `delta·velocity < 0`); a first event (no rate info)
  starts with zero velocity, so single mouse notches don't coast.
- `tick_zoom` eases the camera by exponential approach in log space:
  `blend = 1 − 0.5^(dt/half_life)`, `camera.distance *= (target/distance)^blend`.
  Settles (drops the `Zoom`) when within `1e-3` of the target *and*
  velocity is zero.
- **Rejected designs (do not reintroduce)**: fixed-half-life always-glide
  (laggy during active scroll); a fixed burst-gap split (instant if gap <
  0.05 s, glide otherwise — failed because a momentum tail starts dense and
  decays *through* the threshold, so its mid-tail got classified "active"
  and stepped); direct per-event zoom with no smoothing (briefly shipped,
  reverted).

`tick(camera, height)` runs both `tick_coast` and `tick_zoom`, returning
whether either still needs frames.

---

## 6. Celestial sphere (`src/simulation/celestial_sphere.rs`) — ephemeris-driven Sun + star orientation

Replaces the old slider-driven `Sun`. `CelestialSphere::at(time)` computes, for a satkit
`Instant`, the Sun direction and the star-map orientation in the renderer's
world frame; recomputed every running frame (cheap: ephemeris interp + a few
rotations). Geocentric model — Earth stays the globe at the origin. **For the
full pipeline, frames, and math see §16** (this is the module-level summary).

- `init_satkit()` seeds two satkit globals once at the start of a scenario's
  `run` (before the first ephemeris/frame-transform use):
  - `satkit::jplephem::init_from_bytes(EPHEMERIS)`, where `EPHEMERIS` is the
    DE440 binary embedded via `include_bytes!(concat!(env!("OUT_DIR"),
    "/linux_p1550p2650.440"))`. Sets satkit's `JPL_INSTANCE` `OnceLock`
    (settable once); **must** run before any position query, else satkit's lazy
    disk loader wins and this returns `AlreadyInitialized`.
  - `satkit::earth_orientation_params::init_from_bytes(EOP)` +
    `disable_eop_time_warning()`, where `EOP` is CelesTrak's `EOP-All.csv`
    embedded via `include_bytes!(concat!(env!("OUT_DIR"), "/EOP-All.csv"))`.
    Two reasons. (a) *Accuracy*: real EOP (polar motion + UT1-UTC) is what makes
    the satellite's ITRF transform sub-arcsec. (b) *No stray dir*: every frame
    transform reads satkit's global EOP table on first use - including the
    `*_approx` ones, because `gmst` does a UT1 conversion that consults it, and
    `qteme2itrf` reads polar motion - and satkit's default loader *resolves a
    data dir and creates an empty `satkit-data` dir next to the binary* as a
    side effect. Seeding up front consumes the one-shot lazy load
    (`DEFAULT_LOAD_ONCE`) so that dir is never created. **Don't remove the
    seed** - the dir comes back.
    - **What consumes it.** Satellite `qteme2itrf` is the **full** (non-approx)
      transform: real polar motion (via `qitrf2tirs`) + real UT1-UTC (via
      `gmst`) → sub-arcsec ground track. The Sun/star backdrop uses `*_approx`,
      which picks up real UT1-UTC via `gmst` but neglects polar motion (~0.3")
      and uses approximate nutation (~1"); fine for a backdrop.
    - **Full IERS-2010 for the celestial sphere is a bigger job** (not done): switching to
      `qgcrf2itrf`/`qitrf2gcrf` additionally needs satkit's IERS nutation tables
      (Tab5A/5B/5D), which `ierstable` `.unwrap()`s from the data dir — would
      `panic` + re-create `satkit-data` unless those are also bundled and seeded
      (`ierstable::init_from_bytes`).
    - **Valid range** ≈ 1962-01-01 → last `EOP-All.csv` row (≈ build date);
      out-of-range lookups return `None` → zeros. Past-only keeps scenarios in
      range and the snapshot permanently valid. See §16.9.
  - Net: no `set_datadir`/`data/` dir, no `satkit-data` dir - the app is
    fully offline and data-dir-free.
- **Sun**: `geocentric_pos(SolarSystem::Sun, time)` → GCRF position (meters);
  `qgcrf2itrf_approx(time) * sun_gcrf` → standard ITRF; then permute to the
  world frame and normalize → `sun_dir`.
- **Star map**: `star_rot_inv = P · R_itrf→gcrf · Pᵀ`, where `R_itrf→gcrf` is
  `qitrf2gcrf_approx(time)` as a `Mat3` (built by rotating the basis vectors)
  and `P` is the standard-ECEF→world permutation `(x,y,z)->(y,z,x)`. This maps
  a world view direction into the celestial (GCRF) frame for the equirect star
  lookup; as time advances it rotates the celestial sphere at the sidereal rate, consistent
  with the Sun. Uploaded to the shader **as-is** (no transpose).
- **Frame note**: satkit is standard ECEF/GCRF (Z = pole); the project world
  is Y = north. `P` bridges them — the same permutation `earth::
  surface_position`/`geodetic_normal` bake in. Verified: P·sun matches
  `ITRFCoord::to_geodetic` (Jan-1-2024 subsolar lat −23.02°, lon 0.84°), and
  `star_rot_inv` is a proper rotation (det 1, orthonormal to ~6e-8).
- **Accuracy**: the Sun/star backdrop uses the `*_approx` (IAU-76/FK5)
  transforms (~1 arcsec — sub-pixel here). With real EOP bundled (§16.8) they do
  pick up real UT1-UTC via `gmst`, but still neglect polar motion and use
  approximate nutation; that residual only affects the backdrop, so it's left
  as-is. (The satellite path is the full transform and *does* get sub-arcsec —
  see §6.5/§16.) Both read satkit's EOP global on first use — `init_satkit`
  seeds it so no `satkit-data` dir appears.
- `subsolar_lat_deg`/`subsolar_lon_deg` (from `sun_dir`) are shown read-only
  in the panel (the old sliders are gone).

---

## 6.5. Satellite (`src/simulation/satellite.rs`)

One `Satellite` = one tracked object. The simulation holds a `Vec<Satellite>`
(see §6.6/§3), **assembled by a scenario** and passed to `SimulationState::new`.
Each is one TLE propagated with the **satkit** crate's SGP4.
`Satellite::from_tle(text)` parses a 3-line TLE only - **no propagation at
load**. The position state is **not stored**; `state_at(time)` propagates on
demand and returns a `SatelliteState`. `SimulationState::frame_state` calls it
once per satellite per frame and feeds each result into **both** the renderer's
`RenderState.markers` and the UI's `TelemetryState.satellites`, so the two never
disagree and nothing goes stale as the `Clock` advances. The parsed `TLE` is
**retained** in the struct, and `state_at` takes `&mut self` because `sgp4`
needs `&mut TLE` (it caches its propagator init). **For the full pipeline,
frames, and math see §16** (this is the module-level summary).

- **TLEs**: `ISS_TLE` (real) and `HST_TLE` (orbit shape real, phase
  approximate - flagged in-source), each a 3-line (name + two element lines)
  inline source literal (`concat!`; was `include_str!("assets/TLE.txt")`). These
  `const`s live in the **scenario(s)** that use them, not in this module -
  `satellite.rs` is element-set agnostic. `iss_and_hubble.rs` has both;
  `iss.rs` has its own copy of `ISS_TLE` (duplicated on purpose, per owner).
  `Satellite::from_tle(text)` splits the lines and parses with
  `TLE::load_3line(line0, line1, line2)` (parses by column; trailing checksum
  **not** verified). `tle.epoch` is an `Instant`; `Satellite::epoch()` exposes
  it, and `SimulationState::new` starts the clock at the **first** satellite's
  epoch.
- **Propagate** (shared `propagate(&mut tle, &time)` helper): `sgp4(&mut tle,
  &[time]) -> SGP4State`. `state.pos` is a `DMatrix<f64>` shaped **3×N** (one
  column per time), in **meters**, **TEME** frame. Column 0 → a `Vector3`
  (`numeris`, ctor takes `[[f64;1];3]`).
- **TEME→ITRF**: `qteme2itrf(&time) * teme` — the **full** (non-approx)
  transform, so with the bundled real EOP it applies real polar motion (via
  `qitrf2tirs`) and real UT1-UTC (via `gmst`), giving sub-arcsec accuracy. (For
  in-range dates only; pre-1962 or beyond the EOP file → zeros. `init_satkit`
  seeds the EOP table so no `satkit-data` dir is created — see §15 / §16.8.) Then
  `ITRFCoord::from_vector(&itrf).to_geodetic_rad()` → (lat, lon,
  height-above-ellipsoid).
- **To world space**: rather than permute ECEF axes, reconstruct the point
  with our own helpers: `earth::surface_position(lat, lon) +
  earth::geodetic_normal(lat, lon) * altitude_km`. This guarantees the marker
  lands on the exact WGS84 ellipsoid the mesh uses. Units: meters→km.
- **Stored**: only `tle` (private) and `name`. The position state
  (`position_km` in the world frame, plus `latitude_deg`/`longitude_deg`/
  `altitude_km` for the UI) is returned by `state_at` as a `SatelliteState`,
  not stored. (The datetime lives in the `Clock`, not here.)
- **Verified** (2026-06-18, headless probe): ISS at epoch → lat 50.68°, lon
  176.9°, alt 421 km; +60 s → lon crosses the date line (~5.6°/min), +3600 s →
  opposite hemisphere, alt steady ~420 km (consistent with 51.6° inclination,
  ~92 min orbit).
- Malformed embedded data panics (`expect`), like the other baked-in assets.

## 6.6. Clock (`src/simulation/clock.rs`)

`Clock` is the simulation time source that drives the satellites.

- Fields: `epoch: Instant` (sim time zero = the first satellite's TLE epoch),
  `elapsed_seconds: f64`
  (sim seconds past epoch), `multiplier: f32` (pub, stored as the **linear**
  factor; `MIN_MULTIPLIER 1.0` .. `MAX_MULTIPLIER 100.0`), `paused: bool`
  (pub), `last: Option<std::time::Instant>` (wall-clock ref of the previous
  advance). The UI drives `multiplier` on an exponential (base e) slider; the
  clock itself just multiplies by the plain factor.
- `now()` = `epoch + Duration::from_seconds(elapsed_seconds)` (single source of
  truth; no per-frame Instant accumulation drift).
- `tick() -> bool`: if paused, drops `last` (so resuming doesn't jump by the
  paused interval) and returns false. Else advances `elapsed_seconds += dt *
  multiplier` where `dt` is the wall-clock delta since `last` (0 on the first
  tick after a resume), and returns true. Returned bool feeds `animating`.
- `multiplier`/`paused` are mutated **directly by the UI** (like `Sun`); `tick`
  handles the `last` reset based on `paused`, so direct mutation is safe.
- `datetime_label()` formats `now().as_datetime()` as `YYYY-MM-DD HH:MM:SS UTC`.

## 7. UI (`src/ui.rs`) — egui panel

`control_panel(ctx, &TelemetryState, &mut Clock)` draws an `egui::Area`
anchored top-left (`[10,10]`, width 260). Its readout comes entirely from the
`TelemetryState` snapshot `frame_state` built for this frame (so it matches the
rendered marker exactly); the only thing it mutates is the `&mut Clock`. The
**Sun sliders are gone** (the Sun is ephemeris-driven now); instead a read-only
line shows the computed **subsolar point** (`telemetry.subsolar_lat_deg`/
`lon_deg`). egui claims its own events first (see event routing), so interacting
with the panel never pans the globe. Below a separator it shows the **clock's**
datetime (UTC, `telemetry.datetime_label`, shared by all objects), then **one
block per tracked satellite** (looping `telemetry.satellites`): name + its
sub-satellite lat/lon + altitude. Then the **clock controls**: a `Play`/`Pause`
button (toggles
`clock.paused`; ASCII labels per the source rule) with a live `Speed: N.Nx`
label, and an **exponential (base e) speed slider**: it edits a local
`speed_exp` over `MIN_MULTIPLIER.ln()..=MAX_MULTIPLIER.ln()` (= `0..=ln100`),
writing `clock.multiplier = speed_exp.exp()` **only on `.changed()`** (so the
idle `multiplier->ln->exp` round trip never drifts). Linear slider travel thus
scales time geometrically: 1x at the left, 10x at the midpoint, 100x at the
right. `show_value(false)` (value is in the label).

---

## 8. Mesh (`src/renderer/mesh.rs`)

`wgs84_ellipsoid(stacks, slices)` — renderer calls it with `(64, 128)` (~8.4k
verts, u32 indices). `Vertex { position: [f32;3], normal: [f32;3], uv:
[f32;2] }` (32 bytes; attributes `0 => Float32x3, 1 => Float32x3, 2 =>
Float32x2` in all three pipelines).
- For stack/slice fractions `(u, v)`: `lat = 90° − 180°·v` (geodetic),
  `lon = 360°·u − 180°`. `position = earth::surface_position(lat, lon)` is the
  WGS84 ellipsoid point in km; `normal = earth::geodetic_normal(lat, lon)` is
  the outward geodetic unit normal `(cos lat·sin lon, sin lat, cos lat·cos
  lon)` (same direction a sphere would give); `uv = (u, v)`.
- The seam column at `u=0`/`u=1` is **duplicated** so the texture wraps.
- Indices: two triangles per quad, **CCW when viewed from outside** the
  ellipsoid (so back-face culling keeps the near side for the surface pass).

---

## 9. Renderer (`src/renderer/mod.rs`) — `Gfx` (+ private `GlobeRenderer`)

`Gfx` is the public renderer (`init` / `resize` / `viewport` / `update`, plus
the `FrameOutcome` and `UiFrame` types — see §3 for the per-frame `update`
flow). It owns the surface/device/queue/config and the `egui_wgpu::Renderer`,
and wraps a **private** `GlobeRenderer` scene struct (`new` / `prepare` /
`render`, no iced traits — the camera lives in `application`, not here).
`GlobeRenderer` owns the
**four** render pipelines (surface, atmosphere, stars, marker), the shared
vertex/index buffers, the uniform buffer, the bind group, and the **marker
instance buffer** (`markers` + `marker_capacity`/`marker_count`; grows on demand
in `prepare`, not in the bind group). `STACKS = 64`, `SLICES = 128`. The marker
pipeline shares the bind-group layout and takes **one instance vertex buffer**
(`MarkerInstance { position: vec3, visible: f32 }`, `step_mode = Instance`; its
quad corners still come from the vertex index) and uses alpha blending; it is
built in the `rayon::join` tree alongside the other three.

### `new(device, queue, format)` and its rayon parallelization

Cheap device-object creation (vertex/index/uniform buffers, two samplers,
bind-group layout, bind group) is sequential — it serializes on the
device's internal lock anyway. The expensive work is parallelized:
- One `rayon::join` runs `create_shader_module` (naga parse + validate) on
  one task while the **8 KTX2 textures upload in parallel** via
  `into_par_iter` + `upload_ktx2` on the rest of the pool. `par_iter`
  preserves input order, so the collected views line up with
  `texture_inputs` and the bind-group bindings.
- A nested `rayon::join` compiles the **3 render pipelines** concurrently
  (they share the module + layout, both `Sync`; each does independent
  backend PSO compilation).

This is **intentional** and was added on explicit request (do not confuse
it with the phase-1 reverted parallel decode; see `CLAUDE.md`).

### `upload_ktx2(device, queue, label, bytes) -> TextureView`

The single texture-upload path for both BC7 and LUT textures. Parses the
KTX2 (`ktx2::Reader`), maps the format (`BC7_SRGB_BLOCK` →
`Bc7RgbaUnormSrgb`, `BC7_UNORM_BLOCK` → `Bc7RgbaUnorm`,
`R16G16B16A16_SFLOAT` → `Rgba16Float`), takes level 0, and
`create_texture_with_data` with the raw bytes — a straight memcpy to the
GPU, no decode. All textures `mip_level_count 1`.

### Uniforms (must match Rust `Uniforms` and WGSL `Uniforms`)

```
view_proj:    mat4x4<f32>
camera_pos:   vec3<f32> + 1 f32 pad   (_pad0)   // km
sun_dir:      vec3<f32> + 1 f32 pad   (_pad1)
star_rot_inv: mat3x3<f32>   // Rust: 3 columns each padded to [f32;4]
marker:       vec4<f32>     // x,y = viewport px; z = radius px; w = unused
```

Per-marker position + visibility are **not** in the uniform - they live in the
marker instance buffer (one `MarkerInstance { position: vec3, visible: f32 }`
per satellite, drawn instanced; see the marker shader §). WGSL `mat3x3` columns
have vec4 stride, so the Rust struct pads each column. `star_rot_inv` =
`celestial_sphere.star_rot_inv` (world ECEF → celestial GCRF, uploaded as-is) and `sun_dir` =
`celestial_sphere.sun_dir`, both ephemeris-derived. The uniform and the marker instances are
written every frame in `prepare` (`queue.write_buffer`, ordered before the
frame's submit); `prepare` takes `&mut self` + `&Device` so it can grow the
marker buffer when the satellite count exceeds its capacity.

### Bind group 0 layout

| binding | resource | format / notes |
|---|---|---|
| 0 | uniforms | visibility VERTEX_FRAGMENT |
| 1 | day texture | `Bc7RgbaUnormSrgb`, 8192×4096 |
| 2 | `earth_sampler` | repeat U (dateline seam), clamp V (poles), linear, linear mipmap |
| 3 | night texture | `Bc7RgbaUnormSrgb` |
| 4 | normal map | **`Bc7RgbaUnorm`** (linear — data, not color) |
| 5 | specular mask | `Bc7RgbaUnorm` (linear), `.r` = water |
| 6 | transmittance LUT | `Rgba16Float` 256×64 |
| 7 | `lut_sampler` | clamp both, linear |
| 8 | inscatter Rayleigh LUT | `Rgba16Float` 256×128 |
| 9 | inscatter Mie LUT | `Rgba16Float` 256×128 |
| 10 | stars texture | `Bc7RgbaUnormSrgb` |

`earth_sampler` is shared by all image textures including stars;
`lut_sampler` by the three LUTs. The LUTs are read with
`textureSampleLevel(..., 0.0)` (used in non-uniform control flow; no mips
anyway). **BC7 is lossy** — the normal map is the one to eyeball, since
`NORMAL_STRENGTH` 4.5 amplifies block artifacts (`opaque_slow_settings` in
build.rs is the quality dial).

### `render(render_pass)`

Sets bind group + vertex/index buffers, then three `draw_indexed` calls in
order: **stars → surface → atmosphere**, followed (when `marker_count > 0`) by
the marker instance buffer bound to vertex slot 0 and one instanced
`draw(0..6, 0..marker_count)` for the **markers** (quad corners from the vertex
index, per-marker position/visibility from the instances). Draw order does the
occlusion (no depth buffer). The atmosphere is additive over the whole disc
(aerial perspective) and beyond the limb; the markers are alpha-blended screen
overlays on top.

### `prepare(device, queue, render, viewport)`

Packs the simulation's `RenderState` into the GPU `Uniforms` (and the marker
instance buffer) and writes them (`queue.write_buffer`). All the math is done
upstream (2026-06-19 refactor): the camera resolution (`view_proj`/`camera_pos`
via `celestial_to_world`) lives in `application`, and `sun_dir`/`star_rot_inv`
plus the per-satellite `markers` (`SatelliteMarker { position_km, visible }`,
visibility from `marker_occluded`, a ray-vs-mean-radius-sphere test) come from
`SimulationState::frame_state`. `prepare` only formats them (mat3 column
padding, vec3 pads, `marker = [w, h, MARKER_RADIUS_PX, 0]`) and maps each
`SatelliteMarker` to a `MarkerInstance`, growing the instance buffer (via
`device`) if the count exceeds capacity and recording `marker_count`. `viewport`
is the surface (width, height) px, used solely for the constant-pixel markers.
`MARKER_RADIUS_PX = 6`.

---

## 10. Shader (`shaders/globe.wgsl`) — four passes, one module

### Texture inventory (how each is used)

| Texture | Binding | Format | Used by | Role |
|---|---|---|---|---|
| day | 1 | BC7 sRGB | `fs_main` | Surface albedo, **whole globe**. sRGB ⇒ hardware linearizes on sample, so lighting math is linear. |
| night | 3 | BC7 sRGB | `fs_main` | **Luminance mask only** (find cities). Never displayed as color since phase 3. |
| normal | 4 | BC7 UNORM (linear) | `fs_main` | Tangent-space normal, unpacked `*2−1`. x=east, y=north (OpenGL green-up), z=up. Linear is load-bearing — sRGB decode would warp the vectors. |
| specular | 5 | BC7 UNORM (linear) | `fs_main` | Water mask, `.r` (1 = ocean). Blends BRDF params and gates the wave noise. |
| transmittance LUT | 6 | Rgba16Float 256×64 | `fs_main` | T(r, μ): fraction of sunlight surviving to the top of atmosphere. |
| inscatter Rayleigh | 8 | Rgba16Float 256×128 | `fs_atmosphere` | Σ_R: view-ray Rayleigh integral, phase factored out. |
| inscatter Mie | 9 | Rgba16Float 256×128 | `fs_atmosphere` | Σ_M: same for Mie. |
| stars | 10 | BC7 sRGB | `fs_stars` | Equirectangular star backdrop, sampled by direction. |

### Equirectangular mapping (both directions)

- Forward (mesh): `lat = 90° − 180°v`, `lon = 360°u − 180°`, position as
  above.
- Inverse (`fs_stars`, by direction `d`): `u = atan2(d.x, d.z)/2π + 0.5`,
  `v = acos(d.y)/π`. Runs **per fragment** — interpolating `u` across a
  triangle crossing the ±180° seam would smear the whole texture.

### Surface pass (`vs_main` / `fs_main`)

`vs_main`: `position = view_proj · vec4(pos, 1)`; passes `uv`, the per-vertex
geodetic `normal`, and `world_pos = pos` (the WGS84 surface point in km, used
for the view vector — `pos` is no longer the normal on an ellipsoid).

`fs_main`, step by step:

1. Sample day (albedo), night, normal (`*2−1`), specular (`.r` mask).
2. **Analytic tangent frame** at geometric normal `n_geo = normalize(in.normal)`:
   `east = normalize((n_geo.z, 0, −n_geo.x))` (the exact normalized
   ∂position/∂longitude), `north = cross(n_geo, east)`. No per-vertex
   tangents, no seam issues, exact except at the poles (textures are near-
   constant there so it's invisible).
3. **Perturbed normal**: `n = normalize(east·s.x·S + north·s.y·S +
   n_geo·s.z)`, `S = NORMAL_STRENGTH` (scales tangent components before
   renormalize, ≈ multiplying the slope by S; deliberately exaggerated).
4. **Cook-Torrance GGX specular** (`α = roughness²`):
   - D (GGX): `α² / (π · ((n·h)²(α²−1) + 1)²)`, `h = normalize(v + sun)`.
   - G (Smith, Schlick-GGX): `G₁(n·v)·G₁(n·l)`, `G₁(x) = x/(x(1−k)+k)`,
     `k = α/2`.
   - F (Schlick): `F0 + (1−F0)·(1 − v·h)⁵`.
   - `specular = D·G·F / max(4·(n·v)·(n·l), 1e-4) · (n·l)`. `n·v` clamped
     `≥ 1e-4`. Material from the water mask `m`:
     `roughness = mix(LAND_ROUGHNESS, OCEAN_ROUGHNESS, m)`,
     `F0 = mix(LAND_F0, OCEAN_F0, m)` — land rough/dim, ocean smooth/bright
     ⇒ the sun glint is water-only. Wider glint = raise OCEAN_ROUGHNESS;
     brighter = raise OCEAN_F0.
5. **Wave shimmer** (water only, mean-preserving): two-octave 2D value
   noise `wave_noise(uv)` (`hash2`/`value_noise`); `specular *= mix(1, 1 +
   WAVE_STRENGTH·(2·wave−1), m)`. Static by design.
6. **Atmosphere-filtered sunlight**: `cos_sun = dot(n_geo, sun)`;
   `sun_light = sun_transmittance(PLANET_RADIUS_KM + 0.1, cos_sun)` — the
   transmittance LUT color. This single lookup tints the whole lit surface
   and is what makes the terminator go orange. Applied to **both** diffuse
   and specular.
7. **Diffuse + composite**: `day_lit = albedo·(DAY_AMBIENT + (1−DAY_AMBIENT)
   ·(n·l)·sun_light) + specular·sun_light`.
8. **Night-side darkening** (geometric normal, see `CLAUDE.md`):
   `daylight = smoothstep(−0.12, 0.18, cos_sun)`,
   `night_factor = mix(NIGHT_DARKNESS, 1.0, daylight)`,
   `surface = day_lit · night_factor`.
9. **Procedural city lights (phase 3)** — see next section.

### City lights: dither-dissolve (phase 3, in `fs_main`)

The dark side is the day map scaled by `night_factor`; cities are
re-synthesized procedurally from the night map's *luminance*, never its
color.

```wgsl
let night_brightness = dot(night, vec3(0.2126, 0.7152, 0.0722));
let lit  = smoothstep(EMISSIVE_THRESHOLD,
                      EMISSIVE_THRESHOLD + EMISSIVE_SOFTNESS,
                      night_brightness);          // near-binary city mask
let fade = smoothstep(EMISSIVE_FADE_START, EMISSIVE_FADE_END, cos_sun);
let dither = value_noise_3d(n_geo * DITHER_SCALE);
let keep = step(fade, dither);                    // hard per-pixel dither
surface += lit * keep * EMISSIVE_COLOR * EMISSIVE_STRENGTH;
```

Why it works:
- `lit` is a near-binary mask from one **fixed luminance threshold**
  (`EMISSIVE_SOFTNESS` gives a soft edge that also absorbs BC7 softness).
- `fade` ramps `0 → 1` over `cos_sun ∈ [EMISSIVE_FADE_START,
  EMISSIVE_FADE_END]`. With the current `[−0.15, 0.15]`, the terminator
  (`cos_sun = 0`) is the **midpoint**, so ~half the city pixels survive
  *past* the terminator and finish dissolving by `cos_sun = 0.15` on the
  daylit side — the intended bleed. `EMISSIVE_FADE_END = 0` recovers the
  old "gone exactly at the terminator" behavior. The pair must satisfy
  `FADE_START < FADE_END` (smoothstep edges must be ordered).
- `dither = value_noise_3d(n_geo · DITHER_SCALE)` is **constant per surface
  point** (fixed scale, surface-anchored), so it never crawls or reshuffles
  under zoom/rotate/sun-motion.
- `keep = step(fade, dither)` is a hard per-pixel dither (uniform-brightness
  survivors, no dimming). Each pixel switches off exactly when `fade`
  crosses *its own* `dither` value, so as the terminator sweeps, pixels drop
  out in a **stable order** — a coherent wipe, **no fizz/boil**. (This is
  why the grain is fixed-scale; a frequency ramp makes the per-point value
  uncorrelated frame-to-frame and the band boils.)

**3D noise helpers** (next to the 2D `hash2`/`value_noise`/`wave_noise`):
- `hash3(p)` — an **integer-lattice bit-mixing hash** (cast i32→u32, three
  large-prime multiplies, XOR-folds, normalize to `[0,1]`). Deliberately
  *not* `fract(sin(...))`: `n_geo·DITHER_SCALE` (≈ ×400) pushes lattice
  indices into the hundreds where f32 `sin()` loses precision and bands.
  `p` arrives integer-valued (the floored cell corner).
- `value_noise_3d(p)` — trilinear interpolation of `hash3` over the 8 cube
  corners with a `f²(3−2f)` fade. Single octave (a second octave is an easy
  quality bump; keep it fixed-scale to preserve the coherent wipe).

> **Design note — both values below are intentional (owner-confirmed)**:
> two live constants deliberately depart from the original PHASE3 plan's
> starting values, and both are the shipped look — **do not "revert" them
> toward the plan's numbers.** (1) **`NIGHT_DARKNESS = 1.2`** (plan `~0.02`):
> since `night_factor = mix(NIGHT_DARKNESS, 1.0, daylight)`, a value > 1
> makes the unlit hemisphere ~20 % **brighter** than full daylight, so the
> globe reads bright all the way around with the city glow on top. (2)
> **`EMISSIVE_THRESHOLD = 0.05`** (plan `0.25`): a deliberately more
> permissive city mask, so more of the night map clears it. Mechanically,
> `DAY_AMBIENT` sets the floor of `day_lit` and `night_factor` scales it, so
> a `NIGHT_DARKNESS < 1` would darken the night side (`0` = black night).

A naming gotcha hit during implementation: the noise var cannot be `n`
(that's the perturbed normal) — it is `dither`.

### Atmosphere pass (`vs_atmosphere` / `fs_atmosphere`)

A **sphere** built from the per-vertex unit normal × `ATMOSPHERE_TOP_KM`
(= 6460 km; **not** the ellipsoid position — the scattering model is
spherical), rendered **front-face-culled** (far side of the shell, so it
spans the whole silhouette beyond the limb) with **additive blending**
(One/One). World space is already **km**, planet center at origin.

Per fragment:
1. `origin = camera_pos`, `dir = normalize(world_pos − origin)` (both km, no
   scaling). `ray_sphere(origin, dir, Ra)`; discard if it misses the shell.
2. **Impact parameter** `b = length(origin − dot(origin,dir)·dir)` (closest
   approach to the planet center).
3. **Split row mapping** (must match the bake): `ray_sphere(origin, dir,
   Rp)`; if it hits the ground, reference = ground hit and
   `v = 0.5·clamp(b/Rp)`; else reference = closest approach and
   `v = 0.5 + 0.5·clamp((b−Rp)/(Ra−Rp))`.
4. `μ_ref = dot(normalize(reference), sun)`; `uv = (μ_ref·0.5+0.5, v)`.
   Sample both inscatter LUTs.
5. **Phase functions** (constant along a ray): `μ = dot(dir, sun)`;
   `Φ_R = 3/(16π)(1+μ²)`; Cornette-Shanks
   `Φ_M = 3/(8π)·(1−g²)(1+μ²) / ((2+g²)(1+g²−2gμ)^1.5)`, `g = MIE_G`.
6. `L = Σ_R·Φ_R + Σ_M·Φ_M`; tone roll-off `color = 1 − exp(−L·SUN_INTENSITY)`
   (soft-clips the bright limb).

`ray_sphere(origin, dir, R)` returns the two roots `t = −b' ± √(b'²−c)`,
`b' = origin·dir`, `c = |origin|² − R²`, or `(−1,−1)` on a miss.

### Star + sun backdrop (`vs_stars` / `fs_stars`)

A **sphere** built from the per-vertex unit normal × `STARS_RADIUS_KM`
(= 222985 km ≈ 35 mean Earth radii; must enclose the camera — max ~70000 km
from center — but stay inside the ~318550 km `FAR_PLANE`), rendered
front-face (seen from inside), no blending, **before everything**.

`vs_stars` computes the **camera-relative** view direction
`relative = world − camera_pos` (linear in the vertex ⇒ exact under
interpolation), output twice: `dir = star_rot_inv · relative` (for the star
lookup) and `view = relative` (world frame, for the sun). Both normalized
per fragment. `star_rot_inv` is now the **ephemeris** world(ECEF)→celestial
(GCRF) rotation (`celestial_sphere.star_rot_inv`), so the celestial sphere tracks Earth's real attitude
and rotates at the sidereal rate as the clock advances; `sun_dir` is likewise
the ephemeris Sun direction. (The shader code is unchanged — only the uniform
values; the old sun-attached `star_rotation` is gone.)

`fs_stars`:
- Star color: equirectangular lookup from `normalize(dir)` (inverse mapping,
  per fragment so the seam doesn't smear).
- Sun: `angle = acos(dot(normalize(view), sun))`; AA disc
  `1 − smoothstep(0.85·SUN_ANGULAR_RADIUS, SUN_ANGULAR_RADIUS, angle)` plus
  cubic glow `SUN_GLOW_STRENGTH · max(1 − angle/SUN_GLOW_RADIUS, 0)³`.
- `color = stars·STARS_BRIGHTNESS + SUN_COLOR·(disc + glow)`.

Anchoring **both** lookups to the same camera-relative direction is what
keeps sun and stars locked under orbit/zoom (the backdrop is at infinity —
no parallax, no zoom dependence); the globe drawn afterward occludes it.
The 35-radius geometry is purely a screen-coverage device — nothing of it
shows.

### Satellite markers (`vs_marker` / `fs_marker`)

A flat, constant-pixel-size circle at each tracked object's projected screen
position, drawn last (over the finished scene) with alpha blending as **one
instanced draw** (one instance per satellite). `vs_marker` builds a two-triangle
`[-1,1]^2` quad from `@builtin(vertex_index)` (6 verts); the per-marker world
position + visibility arrive as the `MarkerInstance` vertex attributes
(`@location(0)` position, `@location(1)` visible).

- `vs_marker(vertex_index, inst)`: project `inst.position` to clip. If
  `inst.visible < 0.5` (CPU-decided occlusion) or `clip.w <= 0` (behind camera),
  emit an off-screen clipped vertex (`(0,0,2,1)`) so nothing rasterizes.
  Otherwise offset the center by `corner * radius_px * 2 / viewport` NDC
  (`radius_px` = `uniforms.marker.z`), **× clip.w** to pre-compensate the
  perspective divide (keeps the circle round and size-stable at any depth).
  Passes the corner as `uv`.
- `fs_marker`: `r = length(uv)`; `fwidth(r)` antialiases the outer edge;
  `discard` outside. A white ring (`smoothstep` at r≈0.6) around a red-orange
  fill (`MARKER_FILL`/`MARKER_RING`) so the dot reads on any background.

---

## 11. Atmosphere model (Hillaire 2020) — the math and the bake

Single-scattering atmosphere with Earth's standard medium (Rayleigh + Mie +
ozone). The per-pixel raymarch is replaced by two precomputed LUTs, baked
on the CPU in `build.rs`'s `mod atmosphere` and uploaded as f16 KTX2.

### Medium (defined in `build.rs mod atmosphere`; per-km coefficients)

At altitude `h` (km):
- **Rayleigh**: `σs_R(h) = (5.802, 13.558, 33.1)e-3 · exp(−h/8)` (scattering
  = extinction; no absorption). The 1 : 2.3 : 5.7 blue bias makes the
  atmosphere blue and the transmitted light orange.
- **Mie**: `σs_M(h) = 3.996e-3 · exp(−h/1.2)`, extinction
  `4.40e-3 · exp(−h/1.2)` (the difference is absorption). Phase asymmetry
  `g = 0.8` (Cornette-Shanks). Both `extinction()` and `scattering()` exist
  because Mie scatters less than it extinguishes.
- **Ozone**: absorption only, `(0.650, 1.881, 0.085)e-3 · max(0, 1 −
  |h−25|/15)` — a tent peaking at 25 km, ±15 km. The biggest contributor to
  sunset reds/purples.
- Total extinction `σt(h)` = sum of the three.
- Geometry: `PLANET_RADIUS_KM = 6360`, `ATMOSPHERE_TOP_KM = 6460`.

### Transmittance LUT (256×64; `bake_transmittance`, sampled by `sun_transmittance`)

Beer-Lambert `T = exp(−∫σt dt)` integrated to the top of the atmosphere (40
midpoint steps). **Bruneton (r, μ) parameterization** — resolution
concentrates near the horizon where T changes fastest:
- `ρ = √(r²−Rp²)`, `H_top = √(Ra²−Rp²)`. y axis: `x_r = ρ/H_top`.
- x axis: `d = −r·μ + √(r²(μ²−1) + Ra²)` (distance to atmosphere top),
  `x_mu = (d − d_min)/(d_max − d_min)`, `d_min = Ra − r`, `d_max = ρ + H_top`.
- The bake **inverts** this mapping per texel to recover `(r, μ)`, then
  integrates optical depth: at `t = (s+0.5)·dt`, `r(t) = √(r²+t²+2rμt)`
  (law of cosines), `h = r(t) − Rp` (clamped ≥ 0), accumulate `σt(h)·dt`.
  Store `exp(−depth)`. Kept in **full f32** for the inscatter bake (re-sampled
  thousands of times), converted to f16 only for the GPU copy.
- The WGSL `sun_transmittance(r, μ)` and the CPU `sample_transmittance`
  additionally return 0 when `μ` is below the geometric horizon cosine
  `−√(1 − (Rp/r)²)` — **the planet's own shadow**. The CPU twin does a
  hand-rolled bilinear fetch at texel centers, mirroring GPU filtering so
  bake-time and draw-time reads match.
- `fs_main` uses `T(Rp + 0.1, cos_sun)` directly as the **color of sunlight
  at the ground**.

### Inscatter LUTs (2 × 256×128; `bake_inscatter`)

Single-scattering luminance along a view ray
`L = ∫ T_view(t)·σs(h)·Φ(μ_view·sun)·T_sun(p(t)) dt`. Two exploits make it
precomputable for this scene:
1. **Phase functions factor out** — `dir·sun` is constant along a straight
   ray, so `L = Φ_R·Σ_R + Φ_M·Σ_M` with Σ the integral *without* phase.
2. **Spherical symmetry** — viewed from outside a sphere, a ray is fully
   described by its impact parameter `b` and one sun angle `μ_ref` (the sun
   cosine at a **reference point**: the ground hit for `b < Rp`, else the
   closest-approach point).

LUT axes: x = `μ_ref·0.5 + 0.5`; y = **split mapping** — `0.5·(b/Rp)` for
ground-hitting rays (lower half), `0.5 + 0.5·(b−Rp)/(Ra−Rp)` for limb rays
(upper half). The split gives the thin bright limb band half the table.
**This mapping is implemented independently in the bake and in
`fs_atmosphere` and must match.**

Bake geometry (canonical 2D frame): ray along +x, closest approach `(0, b)`;
entry `t = −√(Ra²−b²)`; exit `−√(Rp²−b²)` (ground) or `+√(Ra²−b²)` (limb).
Reference point normalized with a 1e-3 floor (b→0 head-on rays would
normalize a zero vector). Per step (32 midpoint), the **one approximation**:
the sun's tilt *along* the ray is treated as perpendicular —
`μ_sun(t) = μ_ref · (p̂(t) · r̂_ref) = μ_ref·(t·ref_x + b·ref_y)/r`. Then,
Hillaire eq. 11 (analytic inscatter across a constant-medium step, removes
step-count sensitivity):
```
step_trans = exp(−σt·dt)
integ      = T_view · t_sun · (1 − step_trans) / max(σt, 1e-6)
Σ_R += σs_R · integ ;  Σ_M += σs_M · integ ;  T_view *= step_trans
```
`t_sun = sample_transmittance(r, μ_sun)` (carries the planet shadow into the
LUT). rgb = Σ, alpha = 1.0, pushed as f16 row-major.

### Bake driver

`bake() -> Luts`: `bake_transmittance()` (f32) → `to_f16_texels` for the GPU
copy, and → `bake_inscatter(&transmittance)` → the two f16 inscatter
tables. The whole bake is sub-second.

### Gotchas when modifying
- Any change to the medium constants, the split mapping, the reference-point
  choice, or the Bruneton mapping must be made in **both** `build.rs mod
  atmosphere` and `globe.wgsl`.
- f16 max is 65504, min normal ~6e-5; keep any large scale factor (e.g.
  `SUN_INTENSITY`) **in the shader, not the bake**, or precision suffers.
- `μ_ref = ±1` is never baked exactly (texel centers); don't add math that
  depends on exact endpoint values.

---

## 12. Build pipeline (`build.rs`)

Jobs writing into `OUT_DIR` (which the runtime `include_bytes!`-es). There is
**no `assets/` dir**. The only files on disk are the outputs: the `.ktx2`
textures/LUTs and the two verbatim embeds (ephemeris + EOP). Texture *sources*
are downloaded into memory and transcoded in one pass - they never touch disk.
`OUT_DIR` persists across incremental builds, so an existing output is the cache
(see each step). `download(url, limit)` is the shared in-memory fetch helper
(ureq, body capped at `limit`).

### 0. Embed satkit data files
`embed_verbatim(embed, out_dir)`, run for each entry of the `EMBEDS` table,
`download()`s a file straight into `OUT_DIR` (per-entry size cap) unless already
present, and emits `cargo::rerun-if-changed=OUT_DIR/<name>` so deleting it
re-downloads. These embeds **must** land on disk (unlike the textures) because
`celestial_sphere.rs` `include_bytes!`-es them directly — no copy, no transcode (embedded
verbatim). The two entries:
- **JPL DE440 ephemeris** `linux_p1550p2650.440` (~98 MiB) from
  `ssd.jpl.nasa.gov` (256 MiB cap) — loaded via `jplephem::init_from_bytes`.
- **CelesTrak `EOP-All.csv`** (~2-3 MiB) from `celestrak.org/SpaceData/`
  (64 MiB cap) — Earth-orientation params, loaded via
  `earth_orientation_params::init_from_bytes`.

So there are no runtime data files. Adds ~100 MB to the network-dependent first
build and to the binary (the EOP file is small; the ephemeris dominates).

### 1+2. Download + BC7 → KTX2 transcode (one in-memory pass)
`ASSETS`: five solarsystemscope.com textures, each tagged `srgb: bool` —
`8k_earth_daymap.jpg`, `8k_earth_nightmap.jpg` (srgb), `8k_earth_normal_map.tif`,
`8k_earth_specular_map.tif` (linear/data), `8k_stars_milky_way.jpg` (srgb);
all 8192×4096. `transcode(asset, out_dir)` derives `<stem>.ktx2` from the URL
filename and emits `cargo::rerun-if-changed=OUT_DIR/<stem>.ktx2`. **If that
output already exists it returns immediately** (the cache hit); otherwise it
`download()`s the source into memory, decodes it with `image::load_from_memory`
(format guessed from magic bytes — no on-disk file name needed), asserts
multiple-of-4 dimensions (BC7 block size), BC7-compresses with
`intel_tex_2::bc7::compress_blocks(opaque_basic_settings(), surface)`, and
writes `<stem>.ktx2`. `srgb` → `BC7_SRGB_BLOCK` (day/night/stars), data →
`BC7_UNORM_BLOCK` (normal/specular — must stay linear). The source image is
**never written to disk**.
- **Skip-when-output-exists is now load-bearing**, not just an optimization:
  with no on-disk source cache, re-encoding requires re-downloading, so the
  guard is what stops every build-script rerun (e.g. editing `build.rs`) from
  re-fetching all five textures over the network. The flip side of the
  2026-06-17 "always re-encode" decision: **refreshing a texture or changing
  the encoder settings means deleting the stale `.ktx2`** so the next build
  re-downloads + re-encodes it. (The earlier on-disk-source cache made the
  unconditional re-encode cheap; the in-memory rework traded that for not
  spilling ~30 MB of sources onto disk.)

### 3. Atmosphere LUT bake
`bake_luts` runs the inline `atmosphere::bake()` and writes the three tables
as uncompressed `R16G16B16A16_SFLOAT` KTX2 (transmittance 256×64, two
inscatter 256×128). Runs **unconditionally** (sub-second), so the tables can
never go stale after a constants tweak. The WGSL twin constants still need
manual sync.

### `write_ktx2(format, width, height, blocks)` (shared by 2 and 3)
Hand-serializes a single-level 2D KTX2 using the `ktx2` crate's own types
(so writer and runtime `Reader` can't drift): 80-byte `Header`, one 24-byte
`LevelIndex`, a 4-byte DFD-total-length field + a basic DFD block
(`dfd::Basic::from_format` — the `Reader` *requires* a DFD; length 0 is
rejected), then the raw data 16-byte aligned (`next_multiple_of(16)` covers
BC7's 16 and RGBA16F's 8).

### Net startup effect
Runtime does **no image decode and no LUT bake** — both moved to build time.
Startup is GPU upload + pipeline creation + device init only. Trade-off:
embedded bytes grew ~21 MB (jpeg/tiff) → ~160 MB (5 × 32 MiB BC7) + ~0.6 MB
LUTs, so the binary is large and links slowly (runtime file loading is the
known follow-up). VRAM per color texture dropped 128 MB → 32 MB; uploads 4×
smaller.

---

## 13. Live constant snapshot (2026-06-18 — verify against source)

**`shaders/globe.wgsl`** (top look-tuning block + atmosphere/star consts):
```
DAY_AMBIENT 0.04   NORMAL_STRENGTH 4.5
LAND_ROUGHNESS 0.9   OCEAN_ROUGHNESS 0.45   LAND_F0 0.015   OCEAN_F0 0.15
WAVE_SCALE 2200.0   WAVE_STRENGTH 0.04
EMISSIVE_THRESHOLD 0.05   EMISSIVE_SOFTNESS 0.1
EMISSIVE_COLOR (1.0, 0.85, 0.3)   EMISSIVE_STRENGTH 1.5
EMISSIVE_FADE_START -0.15   EMISSIVE_FADE_END 0.15
DITHER_SCALE 400.0   NIGHT_DARKNESS 1.2      // >1 brightens night side, intentional (§10)
PLANET_RADIUS_KM 6360.0   ATMOSPHERE_TOP_KM 6460.0   MIE_G 0.8   SUN_INTENSITY 12.0
STARS_RADIUS_KM 222985.0   STARS_BRIGHTNESS 0.8
SUN_ANGULAR_RADIUS 0.012   SUN_GLOW_RADIUS 0.12   SUN_GLOW_STRENGTH 0.5
SUN_COLOR (1.0, 0.96, 0.9)
MARKER_FILL (1.0, 0.25, 0.2)   MARKER_RING (1.0, 1.0, 1.0)
day/night terminator smoothstep: smoothstep(-0.12, 0.18, cos_sun)
```
**`build.rs` mod atmosphere**:
```
RAYLEIGH_SCATTERING [5.802, 13.558, 33.1]e-3   RAYLEIGH_SCALE_HEIGHT 8.0
MIE_SCATTERING 3.996e-3   MIE_EXTINCTION 4.40e-3   MIE_SCALE_HEIGHT 1.2
OZONE_ABSORPTION [0.650, 1.881, 0.085]e-3       (tent peak 25 km, ±15)
TRANSMITTANCE 256×64 / 40 steps   INSCATTER 256×128 / 32 steps
```
**`src/application/input.rs`**:
```
FLICK_SPEED 50   STOP_SPEED 15   HALF_LIFE 0.3   FLICK_TIMEOUT 0.1
ZOOM_HALF_LIFE_MIN 0.01   ZOOM_HALF_LIFE_MAX 0.1   WHEEL_GAP_CAP 0.25
ZOOM_COAST_HALF_LIFE 0.15   ZOOM_STOP_RATE 0.1
```
**`src/earth.rs`** (WGS84 + dynamics): SEMI_MAJOR_AXIS_KM 6378.137,
INVERSE_FLATTENING 298.257223563, SEMI_MINOR_AXIS_KM ~6356.752314,
ECCENTRICITY_SQ ~0.00669438, MEAN_RADIUS_KM ~6371.0088,
GRAVITATIONAL_PARAMETER_KM3_S2 398600.4418, ANGULAR_VELOCITY_RAD_S 7.292115e-5.
**`src/application/camera.rs`** (km; constants = `<radii>·MEAN_RADIUS_KM`): FOV_Y
45, MIN_DISTANCE ~63.7, MAX_DISTANCE ~63710, NEAR_PLANE ~63.7, FAR_PLANE
~318550, MAX_TILT 80; defaults lon 0, lat 0, distance ~12742, tilt 0; lat
clamp ±89.
**`src/renderer/mod.rs`**: STACKS 64, SLICES 128, MARKER_RADIUS_PX 6.
**`src/simulation/celestial_sphere.rs`**: embedded ephemeris `linux_p1550p2650.440` (DE440) +
embedded `EOP-All.csv`, both loaded via `init_from_bytes` in `init_satkit`; Sun
via `jplephem::geocentric_pos(SolarSystem::Sun)`, backdrop Earth orientation via
`q*2*_approx` (~1 arcsec); re-evaluated each running frame.
**`src/simulation/satellite.rs`**: element-set agnostic - `from_tle` ctor parses
any 3-line TLE, propagated via `satkit` 0.18 SGP4; position computed on demand
(`state_at`), not stored. The TLE consts and the `Satellite` array both live in
the scenarios (`scenarios/iss_and_hubble.rs`: ISS+HST; `scenarios/iss.rs`: ISS
only, with its own duplicated `ISS_TLE`).
**`src/simulation/clock.rs`**: MIN_MULTIPLIER 1.0, MAX_MULTIPLIER 100.0 (UI slider
is exponential base e); starts at the TLE epoch, `paused = false` (runs at
launch).

---

## 14. Known issues & open follow-ups

Implemented startup optimizations (history): parallelize renderer setup
(rayon ✓), build-time LUT bake (✓), BC7/KTX2 transcode (✓). **Not done**:
- **Downsize textures to 4K** in `build.rs` (would quarter decode/upload;
  current zoom rarely samples past 4K density).
- **Placeholder-then-sharpen**: embed tiny textures for an instant first
  frame, load full-res async, swap the bind group. Fixes *perceived*
  startup; the largest effort.
- **Runtime file loading** instead of `include_bytes!` — shrinks the binary
  and link time, unblocks hot-swapping textures. The KTX2 files are
  self-describing already.

Other standing items:
- **No mipmaps** → far-zoom shimmer; the city-light dither can twinkle/alias
  sub-pixel at low zoom (no MSAA). Mitigations: lower `DITHER_SCALE`, or swap
  `step(fade, dither)` for a narrow `smoothstep`.
- **Expose emissive params (threshold/strength/color/fade) as uniforms +
  egui controls** for interactive tuning.
- **Second noise octave** in `value_noise_3d` if the grain reads too regular
  (keep it fixed-scale).
- **Real bloom post-process** for an actual glow halo (new pass, large) —
  explicitly **declined** for now.
- **No heading control, no fly-to animation, no tile streaming** (the "v2
  leap" from textured globe toward real Google Earth). The `Sun` model and
  the animation-capable redraw loop are already in place for a future
  time-of-day animation (just another "animating" flag).

### wgpu 27 → 29 churn (reference, in case of a future bump)
From the phase-2 migration: `Instance::new` takes `InstanceDescriptor` by
value, no `Default` (use `new_without_display_handle()`); `DeviceDescriptor`
gained `experimental_features`; `get_current_texture()` returns the
`CurrentSurfaceTexture` enum, not `Result`; `PipelineLayoutDescriptor` takes
`&[Option<&BindGroupLayout>]` + `immediate_size: 0` (replaced
`push_constant_ranges`); `multiview` → `multiview_mask` on pipeline and
render-pass descriptors; color attachments gained `depth_slice: None`;
`RenderPassDescriptor` gained `multiview_mask: None`; sampler `mipmap_filter`
is `MipmapFilterMode`. egui 0.34: `Context::run` → `run_ui`,
`is_pointer_over_area` → `is_pointer_over_egui`, `Renderer::new` takes
`RendererOptions`.

---

## 15. satkit crate reference (API notes, verified 2026-06-18)

Reference for the `satkit` ("satellite toolkit") crate, used for SGP4 (see §6.5
`satellite.rs`). Items marked **(verified)** were confirmed empirically with a
headless probe against **v0.18.1**; the rest is from the docs. Re-verify on a
version bump — this crate's API is still moving.

### Crate shape, features, dependencies
- **Linear algebra is `numeris`, NOT nalgebra.** satkit re-exports its types at
  the crate root: `satkit::Vector3`, `satkit::Quaternion`, `satkit::Instant`,
  and `DMatrix<f64>` (a numeris type alias). Watch the version of `numeris` if
  you ever depend on it directly, to avoid two incompatible copies.
- **Cargo features**: `download` (**default**, pulls `ureq` 3.1 for runtime data
  fetch), `omm-xml` (**default**, `quick-xml` for OMM XML), `chrono` (optional,
  adds a `TimeLike` impl for `chrono::DateTime<Utc>`). We use
  `default-features = false` — drops `ureq`+`quick-xml`; SGP4 + the TEME→ITRF
  rotation still work.
- **Data files**: many high-precision functions (full GCRF/IAU reductions,
  gravity fields, JPL ephemerides, space weather, Earth-orientation params)
  require downloaded data — fetched by `satkit::utils::update_datafiles(None,
  false)` (needs the `download` feature) into `satkit::utils::datadir()`, and
  they will fail/panic if it's missing. **(verified)** `sgp4`, `qteme2itrf`,
  `gmst`, and `ITRFCoord` produce correct results with **no** data files. But
  note `qteme2itrf` and `gmst` (and the `*_approx` transforms) still *read*
  satkit's global EOP table on first use, which lazily creates an empty
  `satkit-data` dir (see Frame transforms above); we suppress that by seeding
  the EOP table in `init_satkit`. We bundle real EOP (`EOP-All.csv`) and seed it
  via `init_from_bytes` rather than letting the lazy disk loader run; `qteme2itrf`
  then gets real polar motion + UT1-UTC.
- **EOP table — `satkit::earth_orientation_params`**: a global EOP singleton.
  `init_from_bytes(&[u8])` / `init_from_path(&Path)` seed it from CelesTrak
  `EOP-All.csv` text and consume the one-shot lazy default load. (`parse_csv`
  skips the header line; the real file has 12 columns per row from 1962-01-01
  on, and a header-only buffer would yield an empty/all-zeros table — we did
  that before bundling the real file.) `disable_eop_time_warning()` silences the
  out-of-range warning. `get(tm) -> Option<[f64;6]>` returns
  `[dut1, xp, yp, lod, dX, dY]` or `None` for out-of-range times (→ callers use
  zeros). `init_satkit` seeds it from the embedded real `EOP-All.csv` to stay
  fully offline (see §16.8).

### Time — `satkit::Instant`
- Microseconds since the Unix epoch; **`Copy`** **(verified)** (so `*instant`
  / `&[*t]` work). Construct a UTC datetime with `Instant::from_datetime(year:
  i32, month, day, hour, minute, second: f64) -> Result<Instant>`
  **(verified)** (seconds is fractional `f64`). Debug prints `Instant { year,
  month, day, hour, minute, second }`.
- **Unixtime**: `as_unixtime() -> f64`, `from_unixtime(f64) -> Instant`
  (leap seconds ignored).
- **Arithmetic**: `Instant + Duration -> Instant` **(verified)** (also `-`,
  `+=`, `-=`); `Instant - Instant -> Duration`.
- **Calendar out**: `as_datetime() -> (i32, i32, i32, i32, i32, f64)` =
  (year, month, day, hour, minute, second) UTC **(verified)**; also implements
  `Display` (`to_string()`).
- The `TimeLike` trait abstracts time across `Instant` and (with the `chrono`
  feature) `chrono::DateTime<Utc>`; the propagation/transform fns are generic
  over `T: TimeLike`.

### Duration — `satkit::Duration`
- Microsecond-backed. Constructors: `new(usec: i64)`, `from_microseconds(i64)`,
  `from_milliseconds(f64)`, `from_seconds(f64)` **(verified)**,
  `from_minutes(f64)`, `from_hours(f64)`, `from_days(f64)`, `zero()`.
- Use with `Instant` arithmetic above (e.g. advance a clock:
  `epoch + Duration::from_seconds(elapsed)`).

### TLE — `satkit::tle::TLE`
- Constructors (all but `new` return `Result`): `load_3line(line0, line1,
  line2)` **(verified)** (name + two element lines), `load_2line(line1,
  line2)`, `from_lines(&[String]) -> Result<Vec<TLE>>` (auto 2-/3-line),
  `from_url(url)` (download feature), `new()` (empty/invalid).
- Public fields include `name: String` **(verified)** and `epoch: Instant`
  **(verified)** (the element-set epoch), plus the parsed orbital elements
  (`inclination`, `raan`, `eccen`, `arg_of_perigee`, `mean_anomaly`,
  `mean_motion`, `bstar`, `sat_num`, ...).
- `sgp4` takes **`&mut TLE`** — it lazily builds and caches the SGP4 propagator
  inside the TLE on first use, so the binding must be mutable.

### SGP4 — `satkit::sgp4::sgp4`
- `sgp4(source: &mut impl SGP4Source, tm: &[T: TimeLike]) -> Result<SGP4State>`
  **(verified)**. `TLE` implements `SGP4Source`.
- `SGP4State { pos: DMatrix<f64>, vel: DMatrix<f64>, errcode: Vec<SGP4Error> }`
  **(verified)**. `pos`/`vel` are **3 × N** (3 rows = x,y,z; one **column** per
  input time), in **meters** / **m/s**, **TEME** frame. Index `state.pos[(row,
  col)]` or `state.pos.column(i)`. `errcode[i]` is `SGP4Success` on success.
- ISS TLE at its epoch → |pos| ≈ 6787 km (≈ 420 km altitude). **(verified)**

### Frame transforms — `satkit::frametransform`
- **EOP side effect (satkit 0.18.1 — important).** *Every* transform here reads
  satkit's global EOP table on first use, and that read triggers a lazy default
  load that **resolves a data dir and creates an empty `satkit-data` dir** next
  to the binary (`earth_orientation_params::get`/UT1 conversion →
  `ensure_default_loaded` → `datadir()`). Even the `*_approx` "EOP-free"
  transforms hit it, because `gmst` converts to UT1 (which reads `dut1` from the
  table), and `qteme2itrf` reads polar motion. Without data the lookups return
  zeros (so the math is genuinely EOP-free), but the **dir is still created** as
  a side effect. We suppress it by seeding the EOP table in `init_satkit` (see
  §3 / §16.8) — with the real bundled `EOP-All.csv`, so in-range lookups return
  real values.
- `qteme2itrf<T: TimeLike>(tm: &T) -> Quaternion` **(verified)**: TEME →
  ITRF (Earth-fixed/ECEF). The **full** transform: `qitrf2tirs(tm).conjugate() *
  rotz(-gmst)`, applying polar motion (`qitrf2tirs` reads `xp`,`yp` from EOP) and
  UT1-UTC (`gmst`'s UT1 conversion reads `dut1`). With real EOP bundled this is
  sub-arcsec; this is what the satellite uses. `earth_rotation_angle(...)` is
  pure math; `gmst` reads `dut1` from EOP.
- **GCRF ↔ ITRF**: full `qgcrf2itrf`/`qitrf2gcrf` (IERS 2010, needs EOP data —
  bundled — **and** the `ierstable` IERS nutation files Tab5A/5B/5D, which it
  `.unwrap()`s from the data dir → **not** bundled, so calling these would
  panic) and **`qgcrf2itrf_approx`/`qitrf2gcrf_approx`** (IAU-76/FK5, ~1 arcsec;
  read `dut1` via `gmst` but neglect polar motion + use approximate nutation —
  what the Sun/star backdrop uses). Each `<T: TimeLike>(tm: &T) -> Quaternion`.
  `*_approx` is the conjugate pair. State (pos+vel) variants:
  `gcrf_to_itrf_state[_approx]` etc.
- Apply a quaternion to a vector with `q * v` (numeris `Quaternion: Mul<Vector3>`);
  `Quaternion` is `Copy`, so one `q` can rotate several vectors **(verified)** —
  build a `Mat3` by rotating the three basis vectors.

### Ephemerides — `satkit::jplephem` (JPL DE)
- `geocentric_pos<T: TimeLike>(body: SolarSystem, tm: &T) -> Result<Vector3>`
  **(verified)**: body position **relative to Earth**, **meters**, GCRF
  (inertial). Also `barycentric_pos(...)`.
- `SolarSystem` (re-export it as **`satkit::SolarSystem`**, not
  `satkit::jplephem::SolarSystem` which is private) variants: Mercury, Venus,
  EMB, Mars, Jupiter, Saturn, Uranus, Neptune, Pluto, Moon, **Sun** (no Earth
  variant — it's the geocentric origin) **(verified)**.
- **Needs the JPL DE binary.** Two ways to provide it, both populating a
  module-global `OnceLock` singleton:
  - **`jplephem::init_from_bytes(&[u8]) -> Result<()>`** **(verified, what we
    use)**: parse the DE binary from an in-memory buffer — the entry point for
    embedded/bundled use. We `include_bytes!` the DE440 file and call this.
    There is also `init_from_path(&Path)`. Both must run **before** any
    position query, else the lazy disk loader has already initialized the
    singleton and they return `Error::AlreadyInitialized`.
  - **Lazy disk load** (if neither `init_*` is called): resolution order is env
    `SATKIT_JPLEPHEM_FILE`, else autodetect `linux_p*.4XX`/`lnxp*.4XX` in the
    data dir, else fallback `linux_p1550p2650.440` (DE440, ~98 MiB; JPL hosts it
    at `ssd.jpl.nasa.gov/ftp/eph/planets/Linux/de440/linux_p1550p2650.440`). We
    no longer use this path — we embed the file and `init_from_bytes` it.

### Data directory — `satkit::utils`
(We no longer use any of these — the ephemeris is embedded and loaded via
`jplephem::init_from_bytes`. Kept as reference for the lazy disk-load path.)
- `set_datadir(d: &Path) -> Result<()>` **(verified)**: sets a global
  `OnceCell` (settable **once**; later calls error), validates the dir exists.
- `datadir() -> Result<PathBuf>`: resolves the `SATKIT_DATA` env var (or the
  `set_datadir` singleton). **Side effect (root cause of the `satkit-data`
  dir):** if no populated/writeable candidate exists it `create_dir_all`s the
  first writeable candidate — `${binary_dir}/satkit-data` — and returns it. It's
  called transitively by the EOP lazy load, which is why any frame transform
  used to spawn that empty dir until we pre-seeded EOP (§16.8). `data_found() ->
  bool` checks the **full** bundle (EOP/space-weather), so it returns **false**
  even when the ephemeris alone is present **(verified)** — don't gate ephemeris
  use on it.
- `download_file`/`download_if_not_exist`/`update_datafiles` (the last gated on
  the `download` feature) fetch satkit's data bundle; we don't use them — the
  ephemeris is downloaded directly in build.rs.

### Earth-fixed coordinates — `satkit::itrfcoord::ITRFCoord`
- Build from an ECEF cartesian in **meters**: `ITRFCoord::from_vector(&Vector3)`
  **(verified)**, `ITRFCoord::from([x, y, z])`, or `from_slice(&[..]) ->
  Result`.
- Geodetic accessors (WGS84): `latitude_rad()/latitude_deg()`,
  `longitude_rad()/longitude_deg()`, `hae()` (height above ellipsoid, m),
  `to_geodetic_rad()/to_geodetic_deg() -> (lat, lon, hae)` **(verified)**.

### Gotchas
- **`numeris::Vector3::new` takes `[[f64; 1]; 3]`** (column-major), e.g.
  `Vector3::new([[x], [y], [z]])` **(verified)** — *not* three scalars. `.norm()`
  for magnitude; index `v[(row, 0)]`.
- **Axis convention differs from this project.** satkit ITRF/ECEF is X = prime
  meridian, Y = 90°E, Z = north; our world frame is X = 90°E, Y = north, Z =
  prime meridian (so project `(X,Y,Z)` = ECEF `(Y,Z,X)`). We avoid the
  permutation entirely by converting to geodetic lat/lon/alt and rebuilding the
  point with our own `earth::surface_position`/`geodetic_normal` (see §6.5).
- Units are **meters** out of SGP4/ITRFCoord; our world space is km (÷1000).

---

## 16. Orbital & astronomical computation — end-to-end reference

The single place that explains, in full, how the Sun (and any planet) and the
satellite are positioned/oriented every frame: the reference frames, the
transforms and their math, the exact satkit calls, units, data files, and
accuracy. Quick references elsewhere: module summaries in §6 (Celestial sphere), §6.5
(Satellite), §6.6 (Clock); the satkit API cheat-sheet in §15; the camera in
§4. **Source is authoritative** (`celestial_sphere.rs`, `satellite.rs`, `earth.rs`,
`camera.rs`, `renderer.rs`) — this section is the consolidated explanation.

### 16.1 Model, units, cadence

- **Geocentric.** The Earth is the rendered globe, fixed at the origin in the
  project world frame; it does **not** translate or rotate. Everything
  astronomical (Sun direction, Earth attitude, star orientation, satellite
  position) is computed *for the current simulation time* and expressed in that
  fixed world frame. The visible "Earth rotation" is actually the **camera**
  and the **celestial sphere/Sun** moving (the camera is star-fixed, §16.7).
- **Units.** World space is **kilometers**. satkit returns **meters**, so
  every satkit length is ÷1000 on the way in. Angles in code are radians;
  degrees only at the UI edge.
- **Time** is a satkit `Instant` (µs since the Unix epoch, UTC). The `Clock`
  (§6.6, §16.4) advances it by wall-dt × speed; **every running frame**
  re-evaluates the celestial sphere (`CelestialSphere::at`) at `clock.now()` and propagates the
  satellite on demand (`Satellite::state_at`) where its position is consumed.
  Paused ⇒ nothing recomputes, no frames.

### 16.2 Reference frames (the cast)

| Frame | Definition | Used for |
|---|---|---|
| **TEME** | True Equator Mean Equinox; SGP4's native quasi-inertial frame | SGP4 output (satellite) |
| **GCRF/ICRF** | Geocentric Celestial Reference Frame; inertial; Z = celestial pole, X ≈ vernal equinox | JPL ephemeris output; the star (celestial) frame |
| **ITRF** | "standard" Earth-fixed ECEF: X = equator∩prime-meridian, Y = 90°E, Z = north pole | output of the GCRF/TEME→Earth rotations |
| **world** | this project's frame: **Y = north**, **Z = prime meridian (lon0/lat0)**, **X = 90°E**; km; origin = Earth center | the whole renderer (mesh, camera, uniforms) |
| **celestial (star)** | GCRF re-permuted to Y-up so the equirect star lookup's pole = celestial pole | star map sampling; the camera's inertial rig |

The **world** frame is just **ITRF with axes permuted** so north is +Y (the
project's convention, baked into `earth.rs` and the mesh): `world (X,Y,Z) =
ITRF (Y,Z,X)`.

### 16.3 The axis permutation `P`

`P` maps a **standard ECEF/GCRF** vector (Z = pole) into the **world** frame
(Y = north): `P(x,y,z) = (y,z,x)`. As a `glam::Mat3` it is
`from_cols((0,0,1),(1,0,0),(0,1,0))`. It is a proper rotation/permutation, so
`Pᵀ = P⁻¹`.

Why this exact `P`: the project picked Y = north, +Z at lon0/lat0. For a point
at geodetic `(φ, λ)` the **standard** ECEF unit vector is `(cosφ cosλ, cosφ
sinλ, sinφ)`; applying `P` gives `(cosφ sinλ, sinφ, cosφ cosλ)`, which is
exactly `earth::geodetic_normal(φ, λ)`. So `P` is the bridge between every
satkit (standard-ECEF) result and the world frame, and it is consistent with
the WGS84 helpers (`earth::surface_position`/`geodetic_normal`) the mesh is
built from. (Verified: `P · sun_itrf` → asin/atan2 matches
`ITRFCoord::to_geodetic`.)

### 16.4 Time handling

`satkit::Instant` ↔ datetime via `from_datetime(y,mo,d,h,mi,s)` /
`as_datetime()`; arithmetic via `Instant + Duration::from_seconds(..)`. `Clock`
stores `epoch` (the TLE epoch) + `elapsed_seconds`; `now() = epoch +
Duration::from_seconds(elapsed_seconds)`. All satkit propagation/transform
functions are generic over `T: TimeLike` and take this `Instant` (UTC). satkit
performs the UTC→TT/TDB conversions it needs internally for the ephemeris; the
`*_approx` Earth-orientation transforms ignore UT1−UTC (≤0.9 s ⇒ ≤~13 arcsec of
Earth rotation, sub-pixel here — see §16.9).

### 16.5 Satellites — SGP4 (`satellite.rs`)

The simulation tracks a `Vec<Satellite>` (built by a scenario). Per-satellite
pipeline (TLE → world-space km point), run on demand by `state_at(time)`
(returns a `SatelliteState`; nothing is stored on the struct). `frame_state`
runs it once per satellite per frame:

1. **TLE** `ISS_TLE`/`HST_TLE` (3-line: name + 2 element lines), inline source
   literals (`concat!`; was `include_str!("assets/TLE.txt")`) that live in the
   scenarios (`scenarios/iss_and_hubble.rs`, `scenarios/iss.rs`), parsed by
   `Satellite::from_tle(text)` → `TLE::load_3line(l0,l1,l2) -> TLE` (by column;
   checksum not verified). The parsed `TLE` is **retained** (sgp4 needs
   `&mut TLE` — it caches its propagator) and `tle.epoch` (an `Instant`) of the
   **first** satellite seeds the `Clock`.
2. **Propagate**: `sgp4(&mut tle, &[time]) -> SGP4State { pos, vel:
   DMatrix<f64>, errcode }`. `pos` is **3×N** (one column per input time) in
   **meters**, **TEME** frame. We pass one time, take column 0 →
   `Vector3([[x],[y],[z]])`.
3. **TEME → ITRF**: `qteme2itrf(&time) * teme` — the **full** transform, so with
   the bundled real EOP it applies real polar motion + UT1-UTC → sub-arcsec (in
   range; pre-1962/post-file → zeros). Reads satkit's EOP global, seeded by
   `init_satkit` from the real `EOP-All.csv` (§15 / §16.8). Result is standard
   ECEF, meters.
4. **ITRF → geodetic**: `ITRFCoord::from_vector(&itrf).to_geodetic_rad()` →
   `(lat, lon, hae)` (WGS84; radians, meters).
5. **geodetic → world**: `position_km = earth::surface_position(lat,lon) +
   earth::geodetic_normal(lat,lon) * altitude_km`. Going through geodetic +
   the project's own WGS84 helpers (rather than `P·itrf/1000` directly)
   guarantees the marker sits on the **exact** ellipsoid the mesh uses; the two
   are equivalent.
6. **Returned** (in a `SatelliteState`, not stored): `position_km` (world, for
   the marker), `latitude_deg`, `longitude_deg`, `altitude_km` (UI). `frame_state`
   collects these into `RenderState.markers` + `TelemetryState.satellites`; the
   markers are drawn by the 4th render pass as one instanced call (§10) and each
   is CPU-occluded behind the globe (`marker_occluded`, §9).

Caveats: SGP4 is only accurate ~days around the TLE epoch (far-future clock
times still render, but drift physically); `sgp4`/`qteme2itrf`/`ITRFCoord` need
**no data files**.

### 16.6 Sun & planets — JPL ephemeris (`celestial_sphere.rs`)

Pipeline (Sun → world direction + Earth/celestial-sphere orientation), re-run every running
frame by `CelestialSphere::at(time)`:

1. **Body position**: `geocentric_pos(SolarSystem::Sun, &time) -> Result<
   Vector3>` — position **relative to Earth's center**, **meters**, **GCRF**
   (inertial). The same call serves any body: `SolarSystem` = {Mercury, Venus,
   EMB, Mars, Jupiter, Saturn, Uranus, Neptune, Pluto, Moon, **Sun**}. Only the
   Sun is currently consumed/rendered, but planet positions are one enum
   variant away.
2. **Earth orientation GCRF → ITRF**: `qgcrf2itrf_approx(&time)` (IAU-76/FK5,
   **EOP-free**, ~1 arcsec). `sun_itrf = q * sun_gcrf` (standard ECEF, meters).
3. **Sun in world**: `sun_dir = normalize(P · sun_itrf)`. Uploaded as the
   `sun_dir` uniform; drives day/night, the terminator, city-light fade, ocean
   glint, and the backdrop sun disc.
4. **Subsolar point** (UI, read-only): inverse of `geodetic_normal` —
   `lat = asin(sun_dir.y)`, `lon = atan2(sun_dir.x, sun_dir.z)`.
5. **Star-map orientation**: build `R_itrf2gcrf` as a `Mat3` from
   `qitrf2gcrf_approx(&time)` by rotating the three basis vectors (the
   `Quaternion` is `Copy`, so one `q` rotates all three). Then
   **`star_rot_inv = P · R_itrf2gcrf · Pᵀ`** — world→celestial. This is the
   `star_rot_inv` uniform: the shader maps each world view direction into the
   Y-up celestial frame for the equirectangular star lookup. As `time`
   advances `R_itrf2gcrf` rotates about the celestial pole, so the celestial sphere turns at
   the **sidereal rate**, consistent with the Sun's motion.

### 16.7 Star backdrop & the inertial camera

The shader's star/sun passes (§10) are **unchanged** — only the uniform
*values* (`sun_dir`, `star_rot_inv`) became ephemeris-driven; both are still
functions of the **camera-relative** view direction (the backdrop-anchoring
invariant in `CLAUDE.md`).

The camera (§4) is built in the **celestial frame** and rotated into the world
by **`celestial_to_world = star_rot_inv.transpose()`** (`= P · R_gcrf2itrf ·
Pᵀ`, the inverse of world→celestial), in `ApplicationState::redraw` (via
`SimulationState::celestial_to_world`), applied with
`camera.view_proj(aspect, c2w)` / `camera.eye(c2w)`. Because `star_rot_inv ·
celestial_to_world = I`, a rig held constant in the celestial frame yields a
constant star lookup direction — **the stars are locked to the camera while the
ECEF globe spins underneath**. So `Camera.longitude/latitude` are an inertial
look direction, not geography.

Limitation: the Milky-Way texture is not precisely registered to ICRF, so the
**absolute** celestial-sphere orientation is arbitrary; the **motion** (sidereal rotation +
Sun consistency) is physically correct.

### 16.8 satkit usage & data files (summary)

- **Crate**: `satkit = "0.18"`, `default-features = false` (drops `download` +
  `omm-xml`); linear algebra is `numeris` (`Vector3`/`Quaternion`/`DMatrix`).
- **Functions used**: `tle::TLE::load_3line`; `sgp4::sgp4`;
  `frametransform::{qteme2itrf, qgcrf2itrf_approx, qitrf2gcrf_approx}`;
  `jplephem::{geocentric_pos, init_from_bytes}`;
  `earth_orientation_params::{init_from_bytes, disable_eop_time_warning}`;
  `itrfcoord::ITRFCoord::{from_vector, to_geodetic_rad/deg}`; types `Instant`,
  `Duration`, `Vector3`, `SolarSystem`. Full signatures: §15.
- **Data files**: two, both **embedded** in the binary (no runtime data file):
  the **JPL DE440 ephemeris** `linux_p1550p2650.440` (~98 MiB) and CelesTrak's
  **`EOP-All.csv`** (~2-3 MiB Earth-orientation params). `build.rs` downloads
  both straight into `OUT_DIR`; `celestial_sphere.rs`
  `include_bytes!`-es each and `celestial_sphere::init_satkit()` (called once at the start of
  a scenario's `run`) loads them via `jplephem::init_from_bytes` /
  `earth_orientation_params::init_from_bytes`. The EOP seed also stops satkit
  from creating a `satkit-data` dir (§16.8). The satellite consumes the real EOP
  (sub-arcsec); the Sun/star backdrop uses the `*_approx` transforms (~1 arcsec).
  The binary is self-contained — nothing has to stay on disk between build and
  run.

### 16.9 Accuracy & limitations

- **Ephemeris**: DE440, sub-km Sun position; file spans years 1550–2650.
- **Earth orientation — satellite full, backdrop approximate.** Real EOP
  (`EOP-All.csv`) is bundled. The **satellite** uses the full `qteme2itrf`
  (real polar motion + UT1-UTC) → sub-arcsec, in-range. The **Sun/star
  backdrop** uses the `*_approx` (IAU-76/FK5) transforms: they pick up real
  UT1-UTC via `gmst` but neglect polar motion (~0.3") and use approximate
  nutation (~1"); fine for a backdrop. Full IERS-2010 for the celestial sphere additionally
  needs the IERS nutation tables (Tab5A/5B/5D) bundled — not done (§16.8).
- **Valid time range (load-bearing for scenarios).** The bundled EOP is only
  defined on a bounded interval, so every scenario's epoch window must lie
  inside it:
  - **Lower bound 1962-01-01** — the IERS EOP series starts here; nothing
    measured exists before. (The satellite era starts 1957, so a handful of the
    earliest objects predate EOP.) Earlier dates → satkit returns no EOP →
    silent fallback to zeros (`*_approx` accuracy).
  - **Upper bound = last entry of the bundled `EOP-All.csv`** (≈ build date).
    The tool is **past-only**, so scenarios are always below this; beyond it
    satkit does constant extrapolation (silent degradation).
  - **Discipline:** when a scenario is added, validate its `[start, end]`
    epochs against the bundled file's first/last MJD (and 1962). Out-of-range =
    does not meet the accuracy bar; flag it. See CLAUDE.md "Scenarios & valid
    time range." Because dates are past-only, a bundled EOP file stays correct
    forever (history doesn't change).
- **SGP4**: TLE valid ~days around epoch.
- **Atmosphere** is a *separate*, spherical scattering model (§11) — not part
  of this ephemeris pipeline; its planet radius (6360 km) is its own constant.
- **Star texture** not ICRF-registered (absolute orientation arbitrary).

### 16.10 Validation (headless probes, 2026-06-18)

- **ISS** at the TLE epoch: `|r| ≈ 6787 km` (~420 km altitude, matches the
  51.6° inclination / ~92 min orbit); +60 s the sub-point crosses the date line
  (~5.6°/min in longitude); +3600 s it is in the opposite hemisphere.
- **Sun**: 2024-01-01 12:00 UTC → subsolar `lat −23.02°, lon 0.84°`, distance
  `0.9833 AU` (near the Jan-3 perihelion); 2024-07-01 → `+23.05°`, `1.0167 AU`
  (aphelion); +6 h → `lon −89°` (Sun moved ~90° west).
- **star_rot_inv**: `det = 1.0`, orthonormal to ~6e-8 (a proper rotation).
- **Data**: `init_from_bytes` loads the embedded ephemeris and
  `geocentric_pos` succeeds offline.
