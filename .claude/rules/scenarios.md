---
paths:
  - "src/scenarios/**/*.rs"
  - "src/snapshot.rs"
---

# Scenarios & valid time range

## Adding a scenario

- One module per past scenario under `src/scenarios/`, each defining a
  `<Name>Simulation` struct that implements the `Simulation` trait, plus a
  `run()`. Add a module and a `ScenarioName` variant in `src/main.rs`.
- Each scenario struct holds a `SimulationState` (clock + celestial sphere)
  by composition, plus its own `Vec<Satellite>`. Name the struct
  `<Name>Simulation` (e.g. `IssSimulation`, `IssAndHubbleSimulation`).

### Empty (no-satellite) scenarios

Some scenarios track **no** objects and just wind the celestial sphere to an
event (e.g. `solar_eclipse`, `lunar_eclipse`). They omit the `Vec<Satellite>`
and `last_telemetry` fields entirely: `frame_state` returns `markers:
Vec::new()` and the celestial state from the shared core; `get_drawables`
returns only `self.simulation.get_drawables()` (no scenario panel). With no
TLE there is no epoch to borrow, so `new()` sets the clock start **directly**
from the event datetime via `satkit::Instant::from_datetime(...)` (still
range-check it against the EOP window below).

### Initial camera framing (optional)

A scenario's `run()` can frame its event on launch instead of the default
whole-Terra view: build the simulation, read its `celestial_sphere`, compute a
`Camera` with `Camera::looking_toward(target, star_rot_inv, world_look,
distance)` (orbits `target` with the look axis along a world-frame direction —
e.g. Terra target aimed at `-sol_dir` for the day side, or a Luna target aimed
at Luna's center (`celestial.luna().placement.pos_world`)), and pass it to
`ApplicationState::with_camera(sim, camera)`
instead of `::new`. The camera is fully interactive afterward. The solar eclipse
frames the Terra day side; the lunar eclipse launches **orbiting Luna** (a
Luna-target camera looking toward Luna's center sits on Luna's
Terra-facing side, so Terra is behind the camera — no limb offset needed).

### Camera-target selection (Terra or Luna)

A scenario that offers more than Terra holds a `simulation::TargetSelector`
and overrides `Simulation::camera_target()` to return `self.selector.resolve()`
(a `CameraTarget` identity - the center is resolved from the sphere downstream,
no longer passed in).
The selector's Terra / Luna radio panel is appended in `get_drawables` (after the
shared-core panel; the two panels borrow disjoint fields). A key press only sets
a disjoint `request_*` flag; the scenario's `advance()` calls
`self.selector.apply_requests()` *before* the frame's `camera_target` is read, so
the two radio callbacks never need a shared `&mut` (same disjoint-field rule as
the clock's Run toggle vs speed slider). Satellite scenarios skip all this and
inherit the Terra-only default.

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
identity (the center is resolved from the sphere downstream). `frame_state` just
fills the reduced
`RenderState` (the frame's time + camera rig + `camera_target` (= the resolved
target) + empty markers); the renderer derives every body's geometry from the
time and takes the render origin from the `camera_target`. The panel is appended
in `get_drawables` like the eclipse selector.
- The `Simulation` impl's `frame_state` propagates `self.satellites` using
  `self.simulation.clock.now()`, calls `marker_occluded` from
  `crate::simulation` for visibility, clones each satellite's TLE into its
  `SatelliteMarker` (the renderer propagates it ahead for the predicted orbit
  path), and reads Sol/star values from
  `self.simulation.celestial_sphere`. The near-identical propagation loop
  across scenarios is **intentional** — each may diverge (marker style,
  visibility logic, non-satellite objects); premature factoring adds
  indirection before any variant exists.
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

## Render / snapshot mode

**`snapshot` deliberately does NOT enforce the EOP range.** The render mode
(`src/snapshot.rs`) accepts any datetime and degrades silently outside range.
Do not add a range check there — the caller owns the time and the behavior
is documented.
