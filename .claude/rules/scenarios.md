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
full-globe view: build the simulation, read its `celestial_sphere`, compute a
`Camera` with `Camera::looking_toward(star_rot_inv, world_look, distance)`
(aims the look axis along a world-frame direction — e.g. `-sun_dir` for the
day side, or `moon_pos_world` for the Moon), and pass it to
`ApplicationState::with_camera(sim, camera)` instead of `::new`. The camera is
fully interactive afterward. The eclipse scenarios do this; the Moon also needs
a small latitude offset so the Earth's limb doesn't occlude it.
- The `Simulation` impl's `frame_state` propagates `self.satellites` using
  `self.simulation.clock.now()`, calls `marker_occluded` from
  `crate::simulation` for visibility, and reads sun/star values from
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
