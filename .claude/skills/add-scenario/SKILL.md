---
name: add-scenario
description: Add a new past scenario under src/scenarios/ with a run() and clap CLI wiring, including the mandatory EOP valid-time-range check (1962 to build date). Use when adding a tracked satellite event/time window.
---

# Add a new past scenario

Add one module per past scenario under `src/scenarios/`, each with a `run()`
that pins the simulation to a specific **past** event (a satellite/TLE + a
time window) and wires it into the clap CLI.

## Tools
- `cargo` (build/run), plus the `format-rust` and `clippy-lint` skills

## Steps
1. **Add a module** `src/scenarios/<name>.rs` that:
   - owns its **inline TLE `const`s** (e.g. `ISS_TLE`), assembled with
     `concat!` of the three TLE lines (name + two element lines). TLE data is
     **deliberately duplicated** per scenario — do not factor it into a
     shared const.
   - defines a `pub struct <Name>Simulation { simulation: SimulationState, satellites: Vec<Satellite> }`.
   - implements `Simulation` for it (`advance`, `celestial_to_world`,
     `frame_state`, `clock_mut`). The `frame_state` impl propagates
     `self.satellites` using `self.simulation.clock.now()` and fills in
     `RenderState`/`TelemetryState` from `self.simulation.celestial_sphere`.
     Use `marker_occluded` from `crate::simulation` for visibility testing.
   - has a `run()` function that: calls `simulation::init()` (seeds satkit's
     globals — ephemeris + real EOP — before any ephemeris/frame-transform
     use), then calls
     `application::run(ApplicationState::new(<Name>Simulation::new()))`.
   - In `<Name>Simulation::new()`: build `satellites` first (for the epoch),
     take `satellites.first().expect(...).epoch()` as the clock start, and
     construct `SimulationState::new(epoch)`.
2. **Wire the CLI** in `src/main.rs`: add a `ScenarioName` `ValueEnum`
   variant (use `#[value(name = "<token>")]` to keep the token snake_case)
   and dispatch to the new `run`. `list_scenarios` iterates
   `ScenarioName::value_variants`, so it can't drift.
3. **Format + lint:** run the `format-rust` and `clippy-lint` skills.
4. **Run it:** `cargo run --release -- scenario <token>`.

## Valid time range — check this or the accuracy goal silently breaks
Every scenario's time window must fall inside the bundled EOP range:
- **Lower bound: 1962-01-01** (MJD 37665) — measured EOP does not exist
  before then; earlier dates silently fall back to zeros (`*_approx`-level).
  Satellite era starts in 1957, so verify.
- **Upper bound: the build date** — the last data row of the bundled
  `OUT_DIR/EOP-All.csv`. Past dates only, never live/future; beyond the last
  entry satkit does constant extrapolation (silently degrades).

Check the scenario's start and end epochs against the bundled EOP file's
`[first_entry .. last_entry]` MJD range (and against 1962 for the lower
bound). If a scenario can't be brought in-range, it does not meet the
accuracy bar — **flag it** rather than shipping a silently-degraded result.

## Docs
Update `.claude/rules/scenarios.md` and `.claude/rules/architecture.md`
(file map) in the same change.
