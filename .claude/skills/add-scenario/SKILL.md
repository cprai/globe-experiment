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
1. **Add a module** `src/scenarios/<name>.rs` with a `run()` that:
   - calls `simulation::init()` (seeds satkit's globals — ephemeris + real
     EOP — before any ephemeris/frame-transform use),
   - owns its **inline TLE `const`s** (e.g. `ISS_TLE`), assembled with
     `concat!` of the three TLE lines (name + two element lines). TLE data is
     **deliberately duplicated** per scenario — do not factor it into a
     shared const.
   - assembles the tracked array `vec![Satellite::from_tle(...), ...]`,
   - builds `SimulationState::new(satellites)` (clock starts at the **first**
     satellite's epoch; an empty list panics),
   - builds `ApplicationState::new(simulation)` and calls `application::run`.
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
