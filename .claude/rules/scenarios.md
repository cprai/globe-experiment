---
paths:
  - "src/scenarios/**/*.rs"
  - "src/headless.rs"
---

# Scenarios & valid time range

## Adding a scenario

- One module per past scenario under `src/scenarios/`, each defining a
  `<Name>Simulation` struct that implements the **four traits** `Simulation`
  + `camera::CameraControl` + `camera::CameraView` + `ui::UIDrawable`, plus a
  `run()`. Add a module and a `ScenarioName` variant in `src/main.rs`.
- Each scenario struct holds `clock: Clock` + `celestial_sphere:
  CelestialSphere` + `camera: PtzCamera` as direct fields (there is no shared
  core struct), plus its own `Vec<Satellite>`. `new()` builds
  `Clock::new(epoch)` + `CelestialSphere::at(&clock.now())` +
  `PtzCamera::default()` (or a `looking_toward` framing, below); `advance()`
  ticks the clock and, while running, re-evaluates the sphere.
  `get_drawables` builds the top-left Time panel itself (UTC readout, speed
  readout, Run toggle, speed slider - the toggle/slider callbacks mutate the
  disjoint `clock.paused` / `clock.multiplier` fields by direct assignment).
  The Time-panel code is **deliberately duplicated across scenarios** (like
  the propagation loop) so each can diverge in what it exposes. Name the
  struct `<Name>Simulation` (e.g. `IssSimulation`, `IssAndHubbleSimulation`).
- The `impl CameraControl` block is six one-line forwarders to `self.camera`
  (`pointer_press`/`pointer_release`/`pointer_move`/`scroll`/`tick`/
  `cursor_hint`); the `impl CameraView` block is `frame_state`, the frame
  recipe: derive
  `celestial_to_world = self.celestial_sphere.star_rot_inv.transpose()`,
  resolve the frame's target (`CameraTarget::terra()` or the selector),
  `self.camera.retarget(target, &sphere, c2w)`, `world_rig` -> (eye, look_at,
  up), then pack `RenderState` (packing the SAME resolved `target`). The
  forwarding block is **deliberately duplicated per scenario** (like the Time
  panel) - a scenario may gate input or fly a different camera type.

### Empty (no-satellite) scenarios

Some scenarios track **no** objects and just wind the celestial sphere to an
event (e.g. `solar_eclipse`, `lunar_eclipse`). They omit the `Vec<Satellite>`
and `last_telemetry` fields entirely: `CameraView::frame_state` returns `markers:
Vec::new()` and the time from the scenario's own clock; `get_drawables`
returns the Time panel (plus a selector panel if the scenario has one). With
no TLE there is no epoch to borrow, so `new()` sets the clock start
**directly** from the event datetime via `satkit::Instant::from_datetime(...)`
(still range-check it against the EOP window below).

### Initial camera framing (optional)

A scenario's `new()` can frame its event on launch instead of the default
whole-Terra view: after building the clock + celestial sphere, seed the
`camera` field with `PtzCamera::looking_toward(target, star_rot_inv,
world_look, distance)` (orbits `target` with the look axis along a world-frame
direction — e.g. Terra target aimed at `-sol_dir` for the day side, or a Luna
target aimed at Luna's center (`celestial.luna().placement.pos_world`))
instead of `PtzCamera::default()`. The camera is fully interactive afterward
(`ApplicationState::with_camera` no longer exists - the scenario owns its
camera). The solar eclipse frames the Terra day side; the lunar eclipse
launches **orbiting Luna** (a Luna-target camera looking toward Luna's center
sits on Luna's Terra-facing side, so Terra is behind the camera — no limb
offset needed).

### Camera-target selection (Terra or Luna)

A scenario that offers more than Terra holds a `simulation::TargetSelector`;
its `CameraView::frame_state` resolves it (`self.selector.resolve()`, a
`CameraTarget` identity - the center is resolved from the sphere downstream)
into `self.camera.retarget(..)` each frame.
The selector's Terra / Luna radio panel is appended in `get_drawables` (after the
shared-core panel; the two panels borrow disjoint fields). A key press only sets
a disjoint `request_*` flag; the scenario's `advance()` calls
`self.selector.apply_requests()` *before* the frame's target is resolved
(`frame_state` runs after `advance`), so
the two radio callbacks never need a shared `&mut` (same disjoint-field rule as
the clock's Run toggle vs speed slider). Satellite scenarios skip all this and
retarget `CameraTarget::terra()` every frame (a same-body refresh no-op).

### Multi-body selection (solar_system)

The `solar_system` scenario (empty, no satellites; clock from 2025-06-01) offers
**nine** orbit bodies, so it holds a `simulation::BodySelector` instead of
`TargetSelector`. `SELECTABLE_BODIES` lists them ordered by distance from the
Sol with Luna right after Terra (Mercury, Venus, Terra, Luna, Mars, Jupiter,
Saturn, Uranus, Neptune); `selected` defaults to `TERRA_INDEX` so the scenario
starts on Terra (matching the default whole-Terra camera). The panel shows
**one always-visible latching key per body** (a single column, the chosen one
lit), so each key callback needs a **disjoint** `request_*` field — hence nine
named flags (not an array, whose elements can't be captured disjointly), in
`SELECTABLE_BODIES` order (now a `[CelestialBody; 9]`), that `apply_requests`
folds into `selected`. Its `resolve()` returns the chosen body's `CameraTarget`
identity (the center is resolved from the sphere downstream).
`CameraView::frame_state` retargets/rigs on it and fills the reduced
`RenderState` (the frame's time + camera rig + `camera_target` (= the resolved
target) + empty markers); the renderer derives every body's geometry from the
time and takes the render origin from the `camera_target`. The panel is appended
in `get_drawables` like the eclipse selector.

### Manually-controlled satellite (manual_control)

The `manual_control` scenario tracks **one** satellite the user can thrust.
The ISS TLE (deliberately duplicated, epoch 2024-001.5) is used **once**, in
`new()`, to bootstrap a GCRF state vector (`Satellite::state_at(epoch).orbit`);
after that there is no TLE — the scenario owns `orbit: OrbitState` +
`orbit_epoch: Instant` and every frame's `advance()` **re-anchors** the state
to the clock with `satellite::propagate_numerical` (one `orbitprop` step over
the frame's simulation dt), so a burn's velocity change compounds forward.
Burns: six **hold-to-fire** keys in a **bottom-center** "Burns" panel
(`PanelAnchor::BottomCenter`), one `ui::InteractiveHoldButton` per orbital-
frame direction (prograde/retrograde, normal/anti-normal, radial out/in).
A held key sets a disjoint `burn_*` request flag every egui pass (the selector
request-flag pattern, six named bools); `advance()` folds the flags into a
unit GCRF direction (prograde = v-hat, radial = r-hat, normal = (r x v)-hat,
opposing keys cancel) and applies `dv = BURN_ACCEL_M_S2 * dt * dir`
(10 m/s^2, deliberately game-like ~1 g; dt-scaled so a paused clock burns
nothing), then clears them. `CameraView::frame_state` resolves the marker with
`satellite::resolve_orbit` (pure frame change — the state is already at
`now`), fills `Propagation::Numerical(self.orbit)` so the predicted path
reshapes live, and stashes lat/lon/alt + `satellite::orbit_shape`
(apo/peri/speed; `None` on an escape orbit, shown as dashes — the path
renderer likewise draws nothing for e >= 1).

- The `CameraView` impl's `frame_state` resolves the rig first (its eye feeds the
  occlusion test), then propagates `self.satellites` using
  `self.clock.now()`, calls `marker_occluded` from
  `crate::simulation` for visibility, fills each `SatelliteMarker`'s
  `propagation` (the renderer propagates it ahead for the predicted orbit
  path) — `Propagation::Sgp4` with a cloned TLE, or `Propagation::Numerical`
  with the `SatelliteState.orbit` GCRF state vector from the same propagation
  (`iss_and_hubble` deliberately mixes both, ISS SGP4 + Hubble numerical, to
  exercise the mixed-scene capability) — and reads Sol/star values from
  `self.celestial_sphere`. The near-identical propagation loop (and Time-panel
  builder) across scenarios is **intentional** — each may diverge (marker
  style, visibility logic, non-satellite objects, panel content); premature
  factoring adds indirection before any variant exists.
- Each satellite scenario owns its inline TLE `const`s. The `ISS_TLE` literal
  is **deliberately duplicated** across scenarios that need it — do not factor
  into a shared const. (Empty scenarios have no TLE.)
- Each scenario's `run()` must call `simulation::init()` (= `init_satkit`)
  before any satkit use. This seeds the embedded DE440 ephemeris and the real
  EOP table. See `simulation.md`.

## EOP valid time range (load-bearing accuracy constraint)

Every scenario's epoch window **must** fall inside `[1962-01-01, last
EOP-All.csv entry]` (approximately the build date):

- **Below 1962**: EOP doesn't exist; satkit silently returns zeros for all EOP
  lookups. The satellite transform falls back to EOP-free accuracy.
- **Above last entry**: satkit does constant extrapolation, silently degrading
  accuracy. Past-only keeps all scenarios below this by construction.
- **Out-of-range = does not meet the accuracy bar.** Flag it rather than
  shipping a silently-degraded result.

When adding a scenario, validate its `[start, end]` epochs against the
bundled file's first/last MJD (and 1962 as a hard lower bound).

## The `headless` binary (single-frame render)

**The `headless` binary deliberately does NOT enforce the EOP range.** It
(`src/headless.rs`, its own bin over the shared `engine` — no
`scenarios`) accepts any datetime and degrades silently outside range.
Do not add a range check there — the caller owns the time and the behavior
is documented.
