---
name: add-scene
description: Add a new past scene under src/scenes/ with a run() and clap CLI wiring, including the mandatory EOP valid-time-range check (1962 to build date). Use when adding a tracked satellite event/time window.
---

# Add a new past scene

Add one module per past scene under `src/scenes/`, each with a `run()`
that pins the simulation to a specific **past** event (a satellite/TLE + a
time window) and wires it into the clap CLI.

## Tools
- `cargo` (build/run), plus the `format-rust` and `clippy-lint` skills

## Steps
1. **Add a module** `src/scenes/<name>.rs` that:
   - owns its **inline TLE `const`s** (e.g. `ISS_TLE`), assembled with
     `concat!` of the three TLE lines (name + two element lines). TLE data is
     **deliberately duplicated** per scene — do not factor it into a
     shared const.
   - defines a `pub struct <Name>Scene { clock: Clock, celestial_sphere:
     CelestialSphere, satellites: Vec<Satellite>, last_telemetry:
     Vec<SatelliteTelemetry> }` (the clock + celestial sphere are direct
     fields; there is no shared core struct).
   - implements `Scene` for it (`advance`, `celestial`, `frame_state`).
     `advance` ticks the clock and, while it is running, re-evaluates
     `CelestialSphere::at(&self.clock.now())`. The `frame_state` impl
     propagates `self.satellites` using `self.clock.now()`, fills in
     `RenderState`, and stashes the per-satellite readout into
     `self.last_telemetry`. Use `marker_occluded` from `crate::scene`
     for visibility testing.
   - implements `crate::ui::UIDrawable` for it (import `UIDrawable`,
     `UIDrawablePanel`, `Instrument`, `PanelAnchor`, and the instrument structs
     you use, from `crate::ui`): build the top-left **Time panel** first (copy
     it from an existing scene: UTC readout, speed readout, Run toggle +
     speed slider whose callbacks assign the disjoint `self.clock.paused` /
     `self.clock.multiplier` fields directly — the panel code is deliberately
     duplicated per scene), then push **one** scene `UIDrawablePanel`
     (e.g. `anchor: PanelAnchor::TopRight`) whose rows are per-satellite
     readouts from `self.last_telemetry` (e.g. `Box::new(Header { .. })`).
   - has a `run()` function that: calls `scene::init()` (seeds satkit's
     globals — ephemeris + real EOP — before any ephemeris/frame-transform
     use), then calls
     `application::run(ApplicationState::new(<Name>Scene::new()))`.
   - In `<Name>Scene::new()`: build `satellites` first (for the epoch),
     take `satellites.first().expect(...).epoch()` as the clock start, then
     `let clock = Clock::new(epoch);` and `celestial_sphere:
     CelestialSphere::at(&clock.now())`.
2. **Wire the CLI** in `src/main.rs`: add a `SceneName` `ValueEnum`
   variant (use `#[value(name = "<token>")]` to keep the token snake_case)
   and dispatch to the new `run`. `list_scenes` iterates
   `SceneName::value_variants`, so it can't drift.
3. **Format + lint:** run the `format-rust` and `clippy-lint` skills.
4. **Run it:** `cargo run --release -- scene <token>`.

## Valid time range — check this or the accuracy goal silently breaks
Every scene's time window must fall inside the bundled EOP range:
- **Lower bound: 1962-01-01** (MJD 37665) — measured EOP does not exist
  before then; earlier dates silently fall back to zeros (`*_approx`-level).
  Satellite era starts in 1957, so verify.
- **Upper bound: the build date** — the last data row of the bundled
  `OUT_DIR/EOP-All.csv`. Past dates only, never live/future; beyond the last
  entry satkit does constant extrapolation (silently degrades).

Check the scene's start and end epochs against the bundled EOP file's
`[first_entry .. last_entry]` MJD range (and against 1962 for the lower
bound). If a scene can't be brought in-range, it does not meet the
accuracy bar — **flag it** rather than shipping a silently-degraded result.

## Docs
Update `.claude/rules/scenes.md` and `.claude/rules/architecture.md`
(file map) in the same change.
