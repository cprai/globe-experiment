---
paths:
  - "src/scenes/**/*.rs"
  - "src/headless.rs"
---

# Scenes & valid time range

## Adding a scene

- One module per past scene under `src/scenes/`, each defining a
  `<Name>Scene` struct that implements the **four traits** `Scene`
  + `camera::CameraControl` + `camera::CameraView` + `ui::UIDrawable`
  (`CameraControl` normally arrives via the `camera::ScenePtzCamera`
  blanket impl - see below), plus a
  `run(args)`. Add a module (with its own `#[derive(clap::Args)] Args`
  struct - empty until the scene needs flags) and a `SceneCommand` variant
  wrapping it in `src/main.rs`.
- Each scene struct holds `clock: Clock` + `camera: PtzCamera` +
  `camera_target: CameraTarget` as direct fields (there is no shared core
  struct), plus its own `Vec<Satellite>`. **No scene stores a
  `CelestialSphere`**: `CelestialSphere::at` is a pure function of time, so
  `frame_state` evaluates it fresh at the frame's clock instant (the same
  pattern as the renderer's per-frame `at`). The scene — not the camera —
  owns the orbit target and passes `&self.camera_target` into every camera
  call that depends on it.
  `new()` builds `Clock::new(epoch)` +
  `PtzCamera::default()` (or a `looking_toward` framing, below — the only
  case where `new()` evaluates a throwaway local sphere) +
  `CameraTarget::terra()` (or the body matching the seeded framing, so the
  first frame does not reframe). **The clock tick is NOT the scene's job**:
  the application calls `Scene::tick_scene` (a default method provided
  `where Self: SceneClock`, i.e. for every scene), which ticks the clock and
  then calls `advance(running)`; the scene's `advance(&mut self, running:
  bool)` holds only scene-specific per-frame work (empty for most scenes;
  `manual_control`'s orbit re-anchor is the exception) - any Time-panel
  pause/speed edit already landed directly during the previous egui pass.
  **All clock access goes through the `SceneClock` trait** in
  `src/engine/scene/clock.rs` (beside `Clock` itself): the scene implements
  only `clock_mut()` and calls the trait's default API (`tick_clock`/
  `clock_now`/`clock_datetime_label`/`clock_paused`/`clock_multiplier`/...) -
  all the clock logic lives in those defaults, `Clock` is plain data with
  private fields only same-module code (= the trait defaults) can reach, so
  the compiler enforces it (the only direct `Clock` use in a scene is
  `Clock::new`; a framing sphere in `new()` evaluates at the epoch
  `Instant` directly, before `self` exists).
  `get_drawables` builds the top-left Time panel itself (UTC readout, speed
  readout, Run toggle, speed slider - the toggle/slider callbacks receive
  the scene as `&mut Self` at fire time and call the `SceneClock` setters
  directly: `move |scene| scene.set_clock_paused(running)` /
  `|scene, exp| scene.set_clock_multiplier(exp.exp())`. Keep callbacks
  idempotent - write build-time snapshots like the pre-click `running`,
  never flip live state they re-read - because egui's discard pass can fire
  one twice per frame; the panels are owned/`'static` and built once per
  frame, before `run_ui`).
  The Time-panel code is **deliberately duplicated across scenes** (like
  the propagation loop) so each can diverge in what it exposes. Name the
  struct `<Name>Scene` (e.g. `IssScene`, `IssAndHubbleScene`).
- `CameraControl` comes for free: the scene implements
  `camera::ScenePtzCamera` (three accessors - `camera()`/`camera_mut()`/
  `camera_target()` - saying where the embedded camera and the scene-owned
  target live; the `SceneClock` pattern) and the blanket
  `impl<S: ScenePtzCamera> CameraControl for S` in `camera/ptz.rs` forwards
  every input event into the `PtzCamera`, passing `camera_target()` into the
  calls that scale by the orbited body. **Every** scene implements it, the
  `*_py` wrappers included (their camera is a plain wrapper field, outside
  the pyclass - see below); a scene that must diverge (gate input, fly a
  different camera type) skips `ScenePtzCamera` and implements
  `CameraControl` directly.
  The `impl CameraView` block is `frame_state`, the frame
  recipe: evaluate this frame's sphere
  (`let sphere = CelestialSphere::at(&now)`), derive
  `celestial_to_world = sphere.star_rot_inv.transpose()`, take the frame's
  target (`self.camera_target` - already current: a multi-target scene's
  Camera Target key callbacks wrote it, and reframed, at fire time; fixed at
  Terra for satellite scenes), `world_rig(&target, &sphere, c2w)` ->
  (eye, look_at, up), then pack `RenderState` (packing the SAME `target`
  the rig was built for).

### Empty (no-satellite) scenes

Some scenes track **no** objects and just wind the celestial sphere to an
event (e.g. `solar_eclipse`, `lunar_eclipse`). They omit the `Vec<Satellite>`
field entirely: `CameraView::frame_state` returns `markers:
Vec::new()` and the time from the scene's own clock; `get_drawables`
returns the Time panel (plus a Camera Target panel if the scene offers
more than one orbit body). With
no TLE there is no epoch to borrow, so `new()` sets the clock start
**directly** from the event datetime via `satkit::Instant::from_datetime(...)`
(still range-check it against the EOP window below).

### Initial camera framing (optional)

A scene's `new()` can frame its event on launch instead of the default
whole-Terra view: evaluate a throwaway local sphere at the clock's start
instant (`CelestialSphere::at(&epoch)` - used for the framing only,
never stored; the clock has not ticked yet) and seed the
`camera` field with `PtzCamera::looking_toward(&target, star_rot_inv,
world_look, distance)` (orbits `target` with the look axis along a world-frame
direction — e.g. Terra target aimed at `-sol_dir` for the day side, or a Luna
target aimed at Luna's center (`celestial.luna().placement.pos_world`))
instead of `PtzCamera::default()` — and seed `camera_target` with that same
target, so the framing starts on the body the camera orbits. The camera is fully interactive afterward
(`ApplicationState::with_camera` no longer exists - the scene owns its
camera and its target). The solar eclipse frames the Terra day side; the lunar eclipse
launches **orbiting Luna** (a Luna-target camera looking toward Luna's center
sits on Luna's Terra-facing side, so Terra is behind the camera — no limb
offset needed).

### Camera-target selection (Terra or Luna)

There are **no selector types**: a scene that offers more than Terra holds
no state beyond its `camera_target` and builds its Terra / Luna Camera
Target panel **inline in `get_drawables`** (a header plus two latching keys
splitting one row, the orbited body lit via
`self.camera_target.same_kind(..)`). Each key's callback receives the scene
at fire time and writes the camera target **directly** through the scene's
private `set_camera_target(&mut self, target)` helper: a `same_kind` guard
(re-pressing the lit key is a no-op, which also keeps the callback
idempotent under egui's discard-pass double fire), then
`self.camera.reframe(&target, &sphere, c2w)` against a sphere evaluated at
the current clock instant, then the `camera_target` write. The switch (and
reframe) lands during the egui pass; the next frame's `frame_state` simply
rigs on `self.camera_target`. The helper is deliberately duplicated per
scene, like the Time panel. Satellite scenes skip all this and keep their
`camera_target` fixed at `CameraTarget::terra()` (never reframed).

### Multi-body selection (solar_system)

The `solar_system` scene (empty, no satellites; clock from 2025-06-01)
offers **nine** orbit bodies - same inline pattern as the eclipse scenes,
scaled up. `CelestialBody::ALL` (in `scene/body.rs`) lists them ordered by
distance from Sol with Luna right after Terra (Mercury, Venus, Terra, Luna,
Mars, Jupiter, Saturn, Uranus, Neptune); the scene starts on
`CameraTarget::terra()` (matching the default whole-Terra camera). The
Camera Target panel shows **one always-visible latching key per body** (a
single column, the orbited one lit), built by one loop over
`CelestialBody::ALL` in `get_drawables`: each key's callback writes its own
fixed body into the camera target directly through the same
`set_camera_target` helper as the eclipse scenes (same_kind-guarded reframe
+ write; idempotent). `frame_state` rigs on `self.camera_target` and fills
the reduced `RenderState` (the frame's time + camera rig + `camera_target` +
empty markers); the renderer derives every body's geometry from the time and
takes the render origin from the `camera_target`.

### Manually-controlled satellite (manual_control)

The `manual_control` scene tracks **one** satellite the user can thrust.
The ISS TLE (deliberately duplicated, epoch 2024-001.5) is used **once**, in
`new()`, to bootstrap a GCRF state vector (`Satellite::state_at(epoch).orbit`);
after that there is no TLE — the scene owns `orbit: OrbitState` +
`orbit_epoch: Instant` and every frame's `advance()` **re-anchors** the state
to the clock with `satellite::propagate_numerical` (one `orbitprop` step over
the frame's simulation dt), so a burn's velocity change compounds forward.
Burns: six **hold-to-fire** keys in a **bottom-center** "Burns" panel
(`PanelAnchor::BottomCenter`), one `ui::InteractiveHoldButton` per orbital-
frame direction (prograde/retrograde, normal/anti-normal, radial out/in).
A held key's callback (`|scene: &mut Self| scene.burn_* = true`) sets its
`burn_*` request flag every egui pass - the flags stay (unlike the removed
clock/selector request flags) because the burn is dt-scaled and only
`advance()` knows the frame's simulation dt; `advance()` folds the flags into a
unit GCRF direction (prograde = v-hat, radial = r-hat, normal = (r x v)-hat,
opposing keys cancel) and applies `dv = BURN_ACCEL_M_S2 * dt * dir`
(10 m/s^2, deliberately game-like ~1 g; dt-scaled so a paused clock burns
nothing), then clears them. `CameraView::frame_state` resolves the marker with
`satellite::resolve_orbit` (pure frame change — the state is already at
`now`) and fills `Propagation::Numerical(self.orbit)` so the predicted path
reshapes live. `get_drawables` re-derives lat/lon/alt with the same
`resolve_orbit` at the same clock instant, plus `satellite::orbit_shape`
(apo/peri/speed; `None` on an escape orbit, shown as dashes — the path
renderer likewise draws nothing for e >= 1).

### Python-paneled scenes (manual_control_py, solar_system_py)

Clones of their Rust siblings whose `UIDrawable::get_drawables` delegates to a
Python script — kept **side by side** with the Rust originals so the two panel
APIs can be compared; keep the pairs in sync when either side's panels change.
The pattern:

- **Split struct**: the scene state lives in a `<Name>SceneInner` that is
  itself a `#[pyclass]` (Python-side name `<Name>Scene`, module `globe`, NOT
  registered in the `globe` module — it reaches Python only as the instance
  handed into the script). The Inner carries a **snapshot/request mirror of
  the wrapper-owned clock** as pyclass properties (`paused`/`multiplier`
  getter+setter pairs + `datetime_label()`): the getters return snapshot
  fields the wrapper refreshes after every advance (so both discard-pass
  fires of a callback like `scene.paused = not scene.paused` read the same
  value - idempotent), and the setters record `requested_*` options the
  wrapper folds into the clock before the next tick - the same one-frame
  timing as the Rust siblings' direct setter callbacks. The camera target
  gets the same treatment (solar_system_py): a plain `selected_body: usize`
  Inner field (index into `CelestialBody::ALL`) re-exposed as
  `selected_body`/`body_names()`/`request_body(i)` pymethods - no shared
  `Py<..>` state at all. Rust-private simulation
  state (orbit) stays in plain Inner fields. The Rust side of the
  Inner mirrors the sibling's `new`/`advance` body for body (minus the
  clock: the wrapper ticks it and passes the frame instant into the Inner's
  `advance(&now)`).
- **Thin wrapper**: `<Name>PyScene { inner: Py<Inner>, get_drawables_fn:
  Py<PyAny>, clock: Clock, camera: PtzCamera, camera_target: CameraTarget }`
  implements the four engine traits. The clock, camera, and target are
  **plain wrapper fields, deliberately outside the pyclass** — a script has
  no direct clock/camera surface (only the mirror setters and
  `request_body`), and only a plain field can hand out the `&mut Clock` /
  `&mut PtzCamera` that `SceneClock::clock_mut` / `ScenePtzCamera` require
  (a pyclass cell's borrow guard cannot) — so the wrapper implements
  `SceneClock` (granting `Scene`'s provided `tick_scene`) and
  `ScenePtzCamera` (taking the blanket `CameraControl`) exactly like the
  Rust scenes. The wrapper **overrides `tick_scene`** to first drain the
  Inner's `requested_*` clock edits into the wrapper clock (the clock twin
  of the `selected_body` fold), then run the default's body (tick +
  `advance(running)`); its `advance` does the sibling's scene-specific work
  through the Inner (passing the ticked `clock_now()` in) and pushes fresh
  clock snapshots for the coming `get_drawables`. `frame_state` also lives
  on the **wrapper** (`CameraView`): it reads the frame instant off the
  wrapper clock, borrows the Inner for the orbit / script-requested
  `selected_body` (folding the latter into the wrapper-owned
  `camera_target`, reframing on a genuine switch - the Rust sibling
  retargets directly in its key callbacks, but the script can only reach
  the Inner, so the fold lands here one frame later) and rigs the wrapper's
  own camera; the sibling's recipe,
  split across the boundary. The remaining pyclass-touching methods are
  `Python::attach` + `inner.borrow_mut(py)` + delegate. **The one borrow
  rule: never hold a pyclass borrow across a call into Python.** The trait
  bodies call no Python; `get_drawables` holds NO Rust borrow at all — it
  passes `self.inner.bind(py)` into the script, whose property/method
  accesses each take their own transient borrow. Script callbacks fire later,
  during `control_panel`'s render, when no borrow is live.
- **Burn keys / Camera Target keys**: the script passes the Inner's bound
  `&mut self` methods (`scene.request_prograde`, `scene.request_body(i)`) as
  callbacks - the converted `Interactive*<S>` twins ignore their `&mut S`
  fire-time argument and call the Python callable, which reaches the scene
  through its own transient pyclass borrow. `request_body(i)` writes
  `selected_body` directly (the pymethod twin of a Rust key's camera-target
  write); the burn `request_*`
  methods set the per-key flags both siblings keep (dt-scaled in `advance`);
  the clock setters record the `requested_*` mirror values (folded by the
  wrapper's `tick_scene`).
- **run() order**: `run(args: Args)` carries the script path (the `*_py`
  scenes' required `--script` flag, declared on their own `Args` struct so
  clap rejects it on every other scene; the repo ships the reference
  scripts under the repo-root `scenes/`), then `scene::init()` then
  `engine::py::init()` (inittab before interpreter init, Once-guarded) then
  construct the scene — construction loads the script at **runtime** (no
  path resolution; edit + relaunch, no rebuild).
- **Script contract**: module-level `get_drawables(scene) -> list[Panel]`,
  importing from the embedded `globe` module; the clock is read/driven
  through the scene's own properties (`scene.paused`, `scene.multiplier`,
  `scene.datetime_label()` — no `scene.clock` object); the ln/exp
  speed-slider mapping
  is done in Python (`math.log`/`math.exp` against `Clock.MIN_MULTIPLIER`/
  `MAX_MULTIPLIER` — the classattrs are all `Clock` is still imported for).
  The 9-key Camera Target loop needs the `lambda i=i:` capture
  (Python's late binding).
- **Error policy**: script load/compile failure and a per-frame
  `get_drawables` exception print the traceback and **panic** (they would
  recur every frame — fail fast); a **callback** exception prints and
  continues (one missed mutation; panicking mid-egui-pass would unwind
  through the presenter).

- The `CameraView` impl's `frame_state` resolves the rig first (its eye feeds the
  occlusion test), then propagates `self.satellites` using
  `self.clock_now()`, calls `marker_occluded` from
  `crate::scene` for visibility, fills each `SatelliteMarker`'s
  `propagation` (the renderer propagates it ahead for the predicted orbit
  path) — `Propagation::Sgp4` with a cloned TLE, or `Propagation::Numerical`
  with the `SatelliteState.orbit` GCRF state vector from the same propagation
  (`iss_and_hubble` deliberately mixes both, ISS SGP4 + Hubble numerical, to
  exercise the mixed-scene capability) — and reads Sol/star values from
  the sphere it evaluated at the top of the frame
  (`CelestialSphere::at(&now)`). The near-identical propagation loop (and Time-panel
  builder) across scenes is **intentional** — each may diverge (marker
  style, visibility logic, non-satellite objects, panel content); premature
  factoring adds indirection before any variant exists.
- Each satellite scene owns its inline TLE `const`s. The `ISS_TLE` literal
  is **deliberately duplicated** across scenes that need it — do not factor
  into a shared const. (Empty scenes have no TLE.)
- Each scene's `run()` must call `scene::init()` (= `init_satkit`)
  before any satkit use. This seeds the embedded DE440 ephemeris and the real
  EOP table. See `simulation.md`.

## EOP valid time range (load-bearing accuracy constraint)

Every scene's epoch window **must** fall inside `[1962-01-01, last
EOP-All.csv entry]` (approximately the build date):

- **Below 1962**: EOP doesn't exist; satkit silently returns zeros for all EOP
  lookups. The satellite transform falls back to EOP-free accuracy.
- **Above last entry**: satkit does constant extrapolation, silently degrading
  accuracy. Past-only keeps all scenes below this by construction.
- **Out-of-range = does not meet the accuracy bar.** Flag it rather than
  shipping a silently-degraded result.

When adding a scene, validate its `[start, end]` epochs against the
bundled file's first/last MJD (and 1962 as a hard lower bound).

## The `headless` binary (single-frame render)

**The `headless` binary deliberately does NOT enforce the EOP range.** It
(`src/headless.rs`, its own bin over the shared `engine` — no
`scenes`) accepts any datetime and degrades silently outside range.
Do not add a range check there — the caller owns the time and the behavior
is documented.
