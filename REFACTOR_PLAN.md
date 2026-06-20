# Refactor Plan — module reorganization

A structural refactor of `globe-experiment` into four clear top-level
modules — **application**, **simulation**, **renderer**, **ui** — plus a
shared **earth** geometry module, with a thin `main.rs`. This is a
**behavior-preserving** refactor: no rendering, math, input-feel, or
accuracy changes. Every golden rule in `CLAUDE.md` must survive intact (see
[Invariants](#invariants-that-must-survive)).

> **Status: COMPLETE (2026-06-19).** All milestones landed; the target layout
> below is the current layout. Verified end-to-end (`cargo clippy --release`
> warning-free, smoke run with no panic/validation error, no stray
> `satkit-data` dir). See [Outcome & deviations](#outcome--deviations) for what
> changed relative to this plan. This document is kept as the design record.

---

## 1. Goals

Move from the current `main.rs` + flat `src/globe/*` layout to:

- **`application`** — owns windowing, the winit event loop, per-frame redraw
  orchestration, **and the camera (rig + all input/animation)**. Holds an
  `ApplicationState` that *contains* the `SimulationState`. Translates window
  input into camera/simulation updates, then drives the render.
- **`simulation`** — owns all simulation/astronomical state and math. A
  `SimulationState` holds datetime/clock, play-paused, speed, and
  satellite/TLE info (by composition). It **does not own the camera**; it
  only ever *takes a current camera position/view* and produces a
  `RenderState` (the finished positions/matrices of everything drawn).
- **`renderer`** — owns *only* rendering. A `Gfx` struct with an `init` (GPU
  setup: device/surface/pipelines/bind groups) and an `update(&RenderState,
  …)` that renders one frame.
- **`ui`** — all egui panel *logic*, called by the application.
- **`earth`** — shared WGS84 constants + `surface_position`/`geodetic_normal`
  helpers (used by both simulation and renderer; promoted out of `globe`).
- **`main.rs`** — only: seed satkit, build `SimulationState`, build
  `ApplicationState`, hand off to the application to run the event loop.

### Confirmed design decisions

1. **Camera lives in `application` (rig + animation); everything else takes
   only a current camera position.** This is the load-bearing decision. The
   `Camera` rig type, its `view_proj`/`eye` geometry, and the winit
   `Controller` (pan/tilt/zoom inertia) all live in `application`.
   `SimulationState` never sees a `Camera`; `render_state` receives the
   already-resolved world-frame `eye` (`Vec3`) and `view_proj` (`Mat4`).
   Rationale: adding touch controls (or any other input scheme) is then a
   change *only* inside `application` — `simulation` and `renderer` are
   untouched because they consume a plain camera position.
2. **(2a) egui rendering.** `application` runs the egui *logic* via the `ui`
   module (owning `egui::Context` + `egui_winit::State`), then hands egui's
   tessellated paint output to `Gfx.update(...)` alongside the `RenderState`.
   `Gfx` owns `egui_wgpu::Renderer` and draws the UI in the same render pass.
   All GPU submission stays in `renderer`.
3. **`SimulationState` is composition**, not flattened: it holds named
   `clock` / `satellite` / `celestial_sphere` sub-structs (preserving each subsystem's
   tuned logic), not loose `paused`/`datetime`/`speed` fields.
4. **`earth`** is a top-level shared module (`src/earth.rs`), not buried in
   simulation or renderer.
5. **`RenderState`** is *defined in* `simulation` (a `SimulationState` method
   produces it) and *consumed by* `renderer`. For that POD type the
   dependency runs renderer → simulation.

#### How the inertial-frame camera is resolved (the one subtlety)

The camera rig lives in the **inertial (star-fixed) frame**; the scene is
drawn in the **Earth-fixed world frame**. The bridge is `celestial_to_world =
celestial_sphere.star_rot_inv.transpose()`, which comes from `celestial_sphere` (simulation). So each
frame `application`:

1. reads the rotation from simulation: `let c2w = simulation.celestial_to_world();`
2. resolves the camera using its own viewport aspect:
   `let eye = camera.eye(c2w);` and `let view_proj = camera.view_proj(aspect, c2w);`
3. hands the resolved position/view to simulation:
   `let rs = simulation.render_state(eye, view_proj);`

All camera math stays in `application`; `simulation` only consumes a `Vec3` +
`Mat4` and fills in the astronomical fields. This is a refinement of the
earlier "render_state computes view_proj" idea — the camera ownership moved to
`application`, so the matrix is built there and passed in.

---

## 2. Target layout

```
src/
  main.rs              # thin: seed satkit, build state, run application
  earth.rs             # shared WGS84 constants + surface_position/geodetic_normal
  application/
    mod.rs             # ApplicationState + winit ApplicationHandler + run()
    camera.rs          # Camera rig + view_proj/eye geometry (moved from globe/)
    input.rs           # Controller (winit -> camera, inertia/zoom) (moved from globe/)
  simulation/
    mod.rs             # SimulationState, RenderState, new()/advance()/render_state()/celestial_to_world()
    clock.rs           # moved from globe/clock.rs
    satellite.rs       # moved from globe/satellite.rs
    celestial_sphere.rs             # moved from globe/celestial_sphere.rs (incl. init_satkit)
  renderer/
    mod.rs             # Gfx: device/queue/surface/config + pipelines + egui_wgpu
    mesh.rs            # moved from globe/mesh.rs
  ui.rs                # control_panel(ctx, &mut SimulationState)
shaders/globe.wgsl     # unchanged
```

`src/globe/` and `src/globe/mod.rs` are deleted at the end.

### Dependency direction (acyclic)

```
main      -> application, simulation, renderer
application -> simulation, renderer, ui, earth, (winit, egui, egui_winit, glam)
ui        -> simulation, (egui)
renderer  -> simulation (RenderState), earth, (wgpu, egui_wgpu, ktx2, glam)
simulation-> earth, (satkit, glam)        # NO winit / wgpu / egui / Camera
earth     -> (glam)
```

Two purity rules the compiler will enforce once imports are cleaned up:
- **`simulation` depends on neither winit/wgpu/egui nor the `Camera` type.**
  It deals in `glam`/`satkit` values only (and takes `Vec3`/`Mat4` for the
  camera). This is what makes `render_state` testable and the touch-controls
  swap local.
- The `Camera` type lives in `application`; nothing outside `application`
  references it.

---

## 3. Key types

### `simulation::SimulationState` (composition; no camera)

```rust
pub struct SimulationState {
    pub clock: Clock,          // datetime + play/paused + speed (multiplier)
    pub satellite: Satellite,  // TLE + last propagated state
    pub celestial_sphere: CelestialSphere, // ephemeris-driven sun_dir + star rotation
}

impl SimulationState {
    pub fn new() -> Self;                         // was App::default() minus the camera
    pub fn advance(&mut self) -> bool;            // tick clock; if running, re-propagate sat + recompute celestial sphere; returns "animating"
    pub fn celestial_to_world(&self) -> Mat3;     // celestial_sphere.star_rot_inv.transpose(); the inertial->world bridge for the camera
    pub fn render_state(&self, eye: Vec3, view_proj: Mat4) -> RenderState;   // fills astronomical fields + marker visibility
}
```

- `advance()` absorbs the `clock.tick()` + `satellite.update_to()` +
  `CelestialSphere::at()` block currently inline in `main::App::redraw`.
- `render_state(eye, view_proj)` takes the application-resolved camera and
  returns a finished `RenderState`. It computes `marker_visible` from `eye` +
  `sat_pos` (the `marker_occluded` test, **moved into simulation**), and
  copies through `eye`/`view_proj` plus `sun_dir`/`star_rot_inv`/`sat_pos`.
- No camera animation here — that is entirely in `application`.

### `simulation::RenderState` (plain data; `glam` types)

```rust
pub struct RenderState {
    pub view_proj: Mat4,
    pub camera_pos: Vec3,    // eye in world frame (passed in by application)
    pub sun_dir: Vec3,
    pub star_rot_inv: Mat3,
    pub sat_pos: Vec3,
    pub marker_visible: bool,
}
```

The renderer packs these into its `#[repr(C)] bytemuck` `Uniforms`. The
`MARKER_RADIUS_PX` constant and the `viewport`→marker-uniform packing stay in
the renderer (a pure render concern; renderer knows its viewport via `config`).

### `application::Camera` + `application::Controller`

Moved verbatim from `globe/camera.rs` and `globe/input.rs`. `Camera` keeps its
inertial-frame rig (`longitude`/`latitude`/`distance`/`tilt`) and its
`frame`/`eye`/`view_proj`/`pan`/`tilt_by`/`clamp_distance`/
`pan_degrees_per_pixel` geometry. `Controller` keeps its glide/coast/inertia
logic and named constants **unchanged**. `application` owns both instances.

### `renderer::Gfx`

Absorbs today's `main::Gfx` (device/queue/surface/config + egui_wgpu) **plus**
today's `globe::renderer::GlobeRenderer` (pipelines, buffers, bind group). The
**window is not stored here** — only borrowed (as `Arc<Window>`) during `init`
to create the surface.

```rust
pub struct Gfx { /* surface, device, queue, config, scene pipelines+buffers+bind_group, egui_wgpu::Renderer */ }

pub enum FrameOutcome { Presented, Occluded, Reconfigured }

impl Gfx {
    pub fn init(window: Arc<Window>) -> Self;            // was Gfx::new + GlobeRenderer::new
    pub fn resize(&mut self, size: PhysicalSize<u32>);   // reconfigure surface (owns config)
    pub fn update(&mut self, render: &RenderState, ui: UiFrame) -> FrameOutcome;
    pub fn viewport(&self) -> (f32, f32);                // config size, for the aspect application needs
}
```

- `update` does the whole GPU frame: acquire surface texture (handle
  Lost/Outdated/Timeout/Occluded → `FrameOutcome`), `write_buffer` the
  uniforms from `RenderState`, encode the single render pass (stars → surface
  → atmosphere → marker → egui), submit, `pre_present_notify`, `present`. It
  returns a `FrameOutcome` so the **application** can drive window visibility
  (first-frame reveal) and any retry `request_redraw` — the renderer never
  touches the window.

### `renderer::UiFrame` (POD egui paint output)

```rust
pub struct UiFrame {
    pub primitives: Vec<egui::ClippedPrimitive>,
    pub textures_delta: egui::TexturesDelta,
    pub pixels_per_point: f32,
}
```

`application` produces this (it owns `egui::Context`, runs `ui::control_panel`,
and calls `ctx.tessellate(...)`); `Gfx.update` consumes it via its
`egui_wgpu::Renderer`.

### `application::ApplicationState`

```rust
pub struct ApplicationState {
    simulation: SimulationState,
    camera: Camera,            // owned here (rig + animation)
    controller: Controller,
    egui_ctx: egui::Context,
    // created on `resumed` (need a Window):
    window: Option<Arc<Window>>,
    egui_state: Option<egui_winit::State>,
    gfx: Option<Gfx>,
    shown: bool,
}
```

Implements `winit::ApplicationHandler`:
- `resumed`: create hidden window, `Gfx::init(window.clone())`,
  `egui_winit::State::new(...)`, set cursor, then **direct `redraw()`** (not
  `request_redraw`) for the first frame.
- `window_event`: Close → exit; Resized → `gfx.resize` + redraw; Redraw →
  `redraw()`; else route to egui (`egui_state.on_window_event`); if not
  consumed, feed the `Controller`, which mutates `self.camera`.
- `redraw()`:
  1. `animating = controller.tick(&mut self.camera, h)` (camera inertia/zoom).
  2. run egui: `take_egui_input` → `ctx.run(ui::control_panel(ctx, &mut
     simulation))` → `handle_platform_output`.
  3. `animating |= simulation.advance()`.
  4. resolve camera → view: `let c2w = simulation.celestial_to_world(); let
     aspect = gfx.viewport()…; let eye = camera.eye(c2w); let view_proj =
     camera.view_proj(aspect, c2w);`
  5. `let rs = simulation.render_state(eye, view_proj);`
  6. tessellate → `UiFrame`; `let outcome = gfx.update(&rs, ui_frame);`
  7. react to `outcome` (reveal window on first `Presented`/`Occluded`,
     request redraw on `Reconfigured`); then `if animating || egui_repaint {
     window.request_redraw() }`.

A free `pub fn run(app: ApplicationState)` creates the `EventLoop`, sets
`ControlFlow::Wait`, and calls `run_app`.

### `main.rs` (target)

```rust
fn main() {
    simulation::init();                 // thin wrapper over celestial_sphere::init_satkit() (ephemeris + EOP)
    let simulation = SimulationState::new();
    application::run(ApplicationState::new(simulation));
}
```

`simulation::init()` keeps `main` from knowing about the `celestial_sphere` submodule, and
must run **before** `SimulationState::new()` (which builds `CelestialSphere`).

---

## 4. Milestones

**Process:** one milestone at a time. After each, I stop and report; the owner
reviews and handles the commit, then tells me to proceed. I do not commit.

Each milestone ends with the app **compiling and running** (smoke test:
`timeout 20 cargo run --release 2>&1 | head`), plus `cargo fmt` and
`cargo clippy` clean. The sequence is ordered smallest-blast-radius first;
each is a mostly-mechanical move so regressions are easy to localize.

### M0 — Baseline
- Run the smoke test on the current tree; note startup time, that the clock
  runs from launch, idle-when-paused renders zero frames, no stray
  `satkit-data` dir appears. This is the behavior the refactor must preserve.
- (This plan file is the M0 artifact.)

### M1 — Promote `earth` to shared top-level module
- Move `src/globe/earth.rs` → `src/earth.rs`; add `mod earth;` to `main.rs`;
  remove `pub mod earth;` from `globe/mod.rs`.
- Update imports: `globe::earth` / `super::earth` → `crate::earth`.
- **Done when:** builds, runs, clippy clean. Pure move.

### M2 — Carve out `simulation` (clock / satellite / celestial_sphere only — NOT camera)
- **M2a (move):** Move `clock.rs`, `satellite.rs`, `celestial_sphere.rs` from `src/globe/`
  to `src/simulation/`; create `simulation/mod.rs` with `pub mod`
  declarations + a `pub fn init()` wrapper over `celestial_sphere::init_satkit()`. Update
  paths (`globe::x` → `simulation::x`, `super::earth` → `crate::earth`).
  `App` still uses the pieces individually; `camera.rs`/`input.rs` stay in
  `globe/` for now. Builds & runs identically.
- **M2b (state):** Add `SimulationState { clock, satellite, celestial_sphere }` (composition)
  + `new()` (the clock/satellite/celestial_sphere part of `App::default`) + `advance()`
  (the clock/satellite/celestial_sphere block from `redraw`). Replace those three `App`
  fields with one `simulation: SimulationState`; `App` keeps `camera` +
  `controller`. `redraw` calls `simulation.advance()`. Builds & runs.
- **M2c (render_state):** Move `marker_occluded` into `simulation`. Add
  `RenderState`, `celestial_to_world()`, and `render_state(eye, view_proj)`.
  `App::redraw` now resolves the camera (`c2w = simulation.celestial_to_world()`,
  `eye`/`view_proj` from its camera + aspect), calls
  `simulation.render_state(eye, view_proj)`, then `globe.prepare(&queue, &rs)`.
  `GlobeRenderer::prepare` changes to take `&RenderState` and only pack
  `Uniforms`. Builds & runs. **Checkpoint:** all astronomical math lives in
  `simulation`; the camera math is still in `App` (moves to `application` in
  M4).

### M3 — Reshape the renderer into `Gfx`
- Move `src/globe/renderer.rs` → `src/renderer/mod.rs` and
  `src/globe/mesh.rs` → `src/renderer/mesh.rs`.
- Merge today's `main::Gfx` (device/queue/surface/config + egui_wgpu) and
  `GlobeRenderer` into a single `renderer::Gfx`. Define `Gfx::init(window)`
  (old `Gfx::new` + `GlobeRenderer::new`), `Gfx::resize`, `Gfx::viewport`,
  and `Gfx::update(&RenderState, UiFrame) -> FrameOutcome` (the GPU portion
  of `redraw`: acquire/encode/draw/present). Define `FrameOutcome` and
  `UiFrame`.
- Preserve exactly: non-sRGB surface format pick, `AutoVsync`, the
  stars→surface→atmosphere→marker→egui draw order in one pass, no depth
  attachment, the rayon parallelism in `init`, surface-recovery branches.
- `main::App` shrinks: it now holds `Gfx`, calls `gfx.update(...)`, and reacts
  to `FrameOutcome` for window reveal/retry.
- **Done when:** builds, runs, visuals identical, window still reveals after
  first present.

### M4 — Create the `application` module (camera moves here)
- Move the `App` struct + `ApplicationHandler` impl from `main.rs` into
  `src/application/mod.rs`, renamed `ApplicationState`. Move
  `src/globe/camera.rs` → `src/application/camera.rs` and
  `src/globe/input.rs` → `src/application/input.rs`.
- `ApplicationState` owns `simulation`, `camera`, `controller`, `egui_ctx`,
  and the on-resume `window` / `egui_state` / `gfx` / `shown`.
- Move egui *logic invocation* here: `take_egui_input` → `ctx.run(
  ui::control_panel(ctx, &mut simulation))` → `handle_platform_output` →
  `ctx.tessellate` → `UiFrame`.
- Add `application::run(app)` that builds the `EventLoop` (`ControlFlow::Wait`)
  and runs it.
- Preserve exactly: first-frame **direct `redraw()`** from `resumed`, hidden
  window + reveal-after-present, the `Occluded` first-frame guard, the
  `animating || egui_repaint` redraw gating, egui-first input routing, cursor
  restore.
- **Done when:** builds, runs, all interaction (pan/flick/zoom/tilt/clock)
  behaves as before; idle-when-paused = zero frames.

### M5 — Finish the `ui` module
- Change `ui::control_panel` signature from `(ctx, &CelestialSphere, &Satellite, &mut
  Clock)` to `(ctx, &mut SimulationState)` (read celestial_sphere/satellite, mutate clock).
  Confirm all egui panel logic lives here and `application` only *invokes* it.
- **Done when:** builds, panel identical.

### M6 — Slim `main.rs` + delete `globe`
- Reduce `main.rs` to the target (seed satkit, build `SimulationState`, build
  `ApplicationState`, `application::run`). Delete the now-empty
  `src/globe/mod.rs` and the `globe` namespace.
- **Done when:** builds, runs, `main.rs` is the small target shown above.

### M7 — Docs + final verification
- Update **in the same change** (golden rule): `CLAUDE.md` "Where things
  live" file map + every path reference (`src/globe/*` → new homes,
  `init_satkit` location, camera-now-in-application, "egui in main" notes);
  `MEMORY.md` file map / § references; `README.md`; and the moved files'
  module doc-comments.
- `cargo fmt`, `cargo clippy` (warning-free), smoke test. Owner does the
  native-Windows feel/color pass (WSLg can't validate those). Re-verify the
  [invariants](#invariants-that-must-survive).

---

## 4.5. Outcome & deviations

All milestones landed and the target layout above is the current tree. A few
things diverged from the plan as written — recorded here so the plan matches
reality:

- **M6 folded into M4.** Deleting the `globe` module and slimming `main.rs`
  were listed as M6, but moving `camera.rs` + `input.rs` out in M4 left `globe`
  empty, so it was removed then, and `main.rs` reached its 18-line target in the
  same step. M5 (the `ui` signature change) ran next, and M6 had nothing left to
  do. Net milestone order executed: M1, M2a/b/c, M3, M4 (incl. globe removal +
  main slimming), M5, M7.

- **`GlobeRenderer` kept as a private struct, not flattened into `Gfx`.** The
  plan's "single `renderer::Gfx`" is satisfied at the public API
  (`init`/`resize`/`viewport`/`update`), but the ~480-line scene setup stayed in
  a private `GlobeRenderer` that `Gfx` owns as a field — lower transcription
  risk, same external contract. (`Gfx` holds
  surface/device/queue/config + `egui_wgpu::Renderer` + `globe: GlobeRenderer`.)

- **`Gfx::update` borrows `&Window`.** The plan said the renderer never touches
  the window, but `window.pre_present_notify()` must sit immediately before
  `present()` (which happens inside `update`), so `update(&mut self, window:
  &Window, render, ui)` takes a transient `&Window` *only* for that latency
  hint. `Gfx` still does not store the window, and all visibility/redraw
  decisions remain in `application`, driven by the returned `FrameOutcome`.

- **`SimulationState::render_state` takes `(eye, view_proj)`, not the viewport.**
  Per the camera-in-`application` decision, the application resolves the camera
  to a world-frame `eye` + `view_proj` (reading `simulation.celestial_to_world()`
  for the inertial→world rotation) and passes those in; `render_state` adds the
  astronomical fields + marker visibility. The viewport/aspect never enters
  `simulation` — it's owned by `application`/`renderer`. (This refined the
  original "render_state(viewport)" sketch once the camera moved out of
  `SimulationState`.)

- **`App` → `ApplicationState::new(simulation)` (no derived `Default`).** Built
  explicitly to avoid constructing a throwaway `SimulationState` (which loads the
  TLE + runs SGP4) via `..Default::default()`.

---

## 5. Invariants that must survive

Behavior-preserving means **none** of these change:

- **satkit seeding** (`init_satkit`: ephemeris + EOP from embedded bytes,
  `disable_eop_time_warning`) runs once **before** any `CelestialSphere` is built. No
  stray `satkit-data` dir appears (run from a clean dir to verify).
- **Astronomical accuracy** unchanged: satellite path `qteme2itrf` (full EOP),
  celestial-sphere path `*_approx`. No frame/transform changes.
- **Inertial-frame camera unchanged**: the camera rig stays in the inertial
  frame and is rotated to world by `celestial_to_world =
  celestial_sphere.star_rot_inv.transpose()` via `Camera::view_proj(aspect, c2w)`. The math
  is identical; it just executes in `application` now instead of inside
  `prepare`.
- **First frame** rendered via direct `redraw()` (never `request_redraw`);
  window starts hidden, revealed after first `present`; `Occluded`
  first-frame guard intact.
- **Idle = zero GPU work**: `ControlFlow::Wait`; redraw only when
  `animating || egui_repaint`. `animating` = `controller.tick(...)` (camera) OR
  `simulation.advance()` (clock running). Clock runs from launch; idle only
  when paused.
- **Single render pass, fixed draw order** stars → surface → atmosphere →
  marker → egui; **no depth buffer**; marker occlusion decided on the CPU
  (`marker_occluded`, now in simulation) and passed as a flag.
- **Non-sRGB surface format** + **`AutoVsync`** overrides kept in `Gfx::init`;
  every shader look-tuning constant stays calibrated to this surface.
- **Input feel** untouched: the `Controller` glide/coast/inertia code and its
  named constants are moved verbatim, not restructured.
- **Source format**: all `.rs` stays pure ASCII; `cargo fmt` after every `.rs`
  edit; clippy clean. `shaders/globe.wgsl` is **not touched** by this refactor.
- **No new dependencies**, no reintroduction of iced.

---

## 6. Risks & notes

- **`simulation` purity:** the compiler enforces it once `simulation` stops
  importing winit/wgpu/egui *and* never references the `Camera` type. If a
  signature tempts a `Camera` import, pass `Vec3`/`Mat4` instead — as
  `render_state` does.
- **Camera/celestial-sphere coupling lives in `application`:** `application` must read
  `simulation.celestial_to_world()` to resolve the inertial rig into the world
  frame before calling `render_state`. That is a one-way read
  (application → simulation), not a cycle.
- **`Uniforms` packing stays in renderer:** `RenderState` uses `glam` types;
  the `#[repr(C)] bytemuck` `Uniforms` (vec3 padding, mat3→vec4 columns,
  marker `[w,h,radius,visible]`) is a render detail and stays in
  `renderer/mod.rs`. Keep the field-by-field mapping faithful.
- **Window/`Gfx` split:** the surface is `'static` via `Arc<Window>`; `Gfx`
  borrows the window only in `init`. The window lives in `ApplicationState`
  (needs `request_redraw`/`set_cursor`/`set_visible`). Both hold an `Arc`
  clone.
- **`FrameOutcome` drives visibility:** the reveal-after-present and
  Occluded-retry logic moves from inside the old `redraw` into the
  application's reaction to `Gfx.update`'s return value. Get this mapping
  exactly right or the window can stay invisible (the documented Windows
  hidden-window failure mode).
- **No test suite:** verification is the smoke test + manual interaction;
  the milestone-by-milestone run bisects any regression to one small move.
- **Untracked `package.json` / `package-lock.json`** in the worktree are
  unrelated to this refactor; leave them alone.
