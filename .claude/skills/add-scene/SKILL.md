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
   - defines a `pub struct <Name>Scene { clock: Clock, request_toggle_run:
     bool, request_multiplier: Option<f32>, satellites: Vec<Satellite>,
     camera: PtzCamera, camera_target: CameraTarget }` (direct fields; there
     is no shared core struct, and no stored `CelestialSphere`).
   - implements `crate::engine::scene::SceneClock` for it: just
     `clock_mut() -> &mut Clock`. **All clock access goes through the
     trait's default API** (`tick_clock`/`clock_now`/
     `clock_datetime_label`/...) - the logic lives in those defaults beside
     `Clock` (plain data, private fields), so the compiler enforces it.
   - implements `Scene` (`advance`: fold the Time panel's requests -
     `std::mem::take` the two `request_*` fields into
     `apply_clock_requests` - then `self.tick_clock()`), `CameraControl`
     (forward to the embedded `PtzCamera`), and `CameraView`
     (`frame_state`: propagate `self.satellites` at `self.clock_now()` and
     fill `RenderState`; use `marker_occluded` from `crate::scene` for
     visibility testing).
   - implements `crate::ui::UIDrawable` for it (import `UIDrawable`,
     `UIDrawablePanel`, `Instrument`, `PanelAnchor`, and the instrument structs
     you use, from `crate::ui`): build the top-left **Time panel** first (copy
     it from an existing scene: UTC readout, speed readout, Run toggle +
     speed slider whose callbacks assign the disjoint
     `self.request_toggle_run` / `self.request_multiplier` fields directly
     (never a `SceneClock` call - it would borrow the whole scene and
     collide) — the panel code is deliberately
     duplicated per scene), then push **one** scene `UIDrawablePanel`
     (e.g. `anchor: PanelAnchor::TopRight`) whose rows are per-satellite
     readouts (e.g. `Box::new(Header { .. })`) built into an owned
     `Vec<SatelliteTelemetry>` at the top of `get_drawables` by
     re-propagating `self.satellites` at `self.clock_now()` — the same
     instant `frame_state` used, so the values match the rendered markers.
   - has a `run()` function that: calls `scene::init()` (seeds satkit's
     globals — ephemeris + real EOP — before any ephemeris/frame-transform
     use), then calls
     `application::run(ApplicationState::new(<Name>Scene::new()))`.
   - In `<Name>Scene::new()`: build `satellites` first (for the epoch),
     take `satellites.first().expect(...).epoch()` as the clock start, then
     `clock: Clock::new(epoch)` with both `request_*` fields `false`/`None`.
2. **Wire the CLI**: give the scene module its own `#[derive(clap::Args)]
   pub struct Args {}` (empty until the scene needs flags; `run` takes it)
   and add a `SceneCommand` variant in `src/main.rs` wrapping it (use
   `#[command(name = "<token>")]` to keep the token snake_case) plus a
   dispatch arm to the new `run`. Bare `scene` lists the subcommands via
   clap's help, so the listing can't drift.
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
