# Simulation Trait Refactor Plan

## Goal

Introduce a `Simulation` trait so each scenario can own its satellites directly,
and `ApplicationState` depends only on the trait — not on `SimulationState`
concretely. The clock and celestial sphere stay in `SimulationState` (shared
infrastructure); each scenario struct holds one by composition and adds its own
`Vec<Satellite>`.

---

## Architecture overview

### New `Simulation` trait

Lives in `src/simulation/mod.rs` alongside `RenderState`, `TelemetryState`, and
`SimulationState`.

```rust
pub trait Simulation {
    /// Advance the clock and re-evaluate the celestial sphere. Returns whether
    /// the clock is running (i.e. the app should keep requesting frames).
    fn advance(&mut self) -> bool;

    /// Rotation from the inertial camera rig frame to the Earth-fixed world frame.
    /// Used by the application to resolve the camera before each frame.
    fn celestial_to_world(&self) -> Mat3;

    /// Produce this frame's RenderState (for the renderer) and TelemetryState
    /// (for the UI), given the application-resolved eye position and view-projection
    /// matrix. Satellite propagation happens here (once per frame per satellite).
    fn frame_state(&mut self, eye: Vec3, view_proj: Mat4) -> (RenderState, TelemetryState);

    /// Mutable access to the clock so the egui panel can read play/pause state
    /// and speed, and write them on user interaction. The UI mutates the clock
    /// directly (not via a message queue) — same pattern as today, just routed
    /// through the trait.
    fn clock_mut(&mut self) -> &mut Clock;
}
```

### `SimulationState` changes

Loses `satellites: Vec<Satellite>`. Becomes the shared core: clock + celestial
sphere. Its `frame_state` method is removed (satellite propagation moves into
each scenario impl). Its other methods (`advance`, `celestial_to_world`) stay as
concrete helpers that scenario impls delegate to.

```rust
pub struct SimulationState {
    pub clock: Clock,
    pub celestial_sphere: CelestialSphere,
}

impl SimulationState {
    // start_epoch is satkit::Instant (from the first satellite's .epoch())
    pub fn new(start_epoch: satkit::Instant) -> Self { ... }

    // Same logic as today — clock tick + celestial sphere re-eval
    pub fn advance(&mut self) -> bool { ... }

    // Same as today
    pub fn celestial_to_world(&self) -> Mat3 { ... }
}
```

### Per-scenario structs

One per scenario file. Each holds a `SimulationState` (by composition) plus its
own `Vec<Satellite>`, and implements `Simulation`. The satellite propagation code
that currently lives in `SimulationState::frame_state` moves into each scenario's
`frame_state` impl unchanged — it's a straight lift.

The naming convention is `<ScenarioName>Simulation` (e.g. `IssSimulation`,
`IssAndHubbleSimulation`).

```rust
// src/scenarios/iss.rs
pub struct IssSimulation {
    simulation: SimulationState,
    satellites: Vec<Satellite>,
}

impl IssSimulation {
    fn new() -> Self {
        let satellites = vec![Satellite::from_tle(ISS_TLE)];
        let epoch = satellites.first().expect("TLE present").epoch();
        Self {
            simulation: SimulationState::new(epoch),
            satellites,
        }
    }
}

impl Simulation for IssSimulation {
    fn advance(&mut self) -> bool       { self.simulation.advance() }
    fn celestial_to_world(&self) -> Mat3 { self.simulation.celestial_to_world() }
    fn clock_mut(&mut self) -> &mut Clock { &mut self.simulation.clock }

    fn frame_state(&mut self, eye: Vec3, view_proj: Mat4) -> (RenderState, TelemetryState) {
        // Satellite propagation lifted from SimulationState::frame_state verbatim.
        let now = self.simulation.clock.now();
        let mut markers = Vec::with_capacity(self.satellites.len());
        let mut sat_telemetry = Vec::with_capacity(self.satellites.len());
        for sat in &mut self.satellites {
            let state = sat.state_at(&now);
            markers.push(SatelliteMarker {
                position_km: state.position_km,
                visible: !marker_occluded(eye, state.position_km),
            });
            sat_telemetry.push(SatelliteTelemetry {
                name: sat.name.clone(),
                latitude_deg: state.latitude_deg,
                longitude_deg: state.longitude_deg,
                altitude_km: state.altitude_km,
            });
        }
        let render = RenderState {
            view_proj,
            camera_pos: eye,
            sun_dir: self.simulation.celestial_sphere.sun_dir,
            star_rot_inv: self.simulation.celestial_sphere.star_rot_inv,
            markers,
        };
        let telemetry = TelemetryState {
            subsolar_lat_deg: self.simulation.celestial_sphere.subsolar_lat_deg,
            subsolar_lon_deg: self.simulation.celestial_sphere.subsolar_lon_deg,
            datetime_label: self.simulation.clock.datetime_label(),
            satellites: sat_telemetry,
        };
        (render, telemetry)
    }
}
```

The `IssAndHubbleSimulation` impl is identical in shape, just with two satellites.

### `ApplicationState` becomes generic

```rust
pub struct ApplicationState<S: Simulation> {
    camera: Camera,
    simulation: S,
    controller: Controller,
    window: Option<Arc<Window>>,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    gfx: Option<Gfx>,
    shown: bool,
}

impl<S: Simulation> ApplicationState<S> {
    pub fn new(simulation: S) -> Self { ... }
}

impl<S: Simulation> ApplicationHandler for ApplicationState<S> { ... }
```

`application::run` becomes:

```rust
pub fn run<S: Simulation>(mut app: ApplicationState<S>) { ... }
```

The `redraw` method changes only where it previously accessed `self.simulation`
directly — four call sites, all now going through the trait:

| Before | After |
|---|---|
| `self.simulation.advance()` | `self.simulation.advance()` — no change, same method name |
| `self.simulation.celestial_to_world()` | `self.simulation.celestial_to_world()` — no change |
| `self.simulation.frame_state(eye, view_proj)` | `self.simulation.frame_state(eye, view_proj)` — no change |
| `&mut self.simulation.clock` | `self.simulation.clock_mut()` |

Only the last line changes at the call site (field access -> method call).

### Scenario `run()` functions

The `run()` functions simplify: they no longer construct `Vec<Satellite>` and
`SimulationState` separately; they just construct the scenario struct and hand it
to the application. The `simulation::init()` call remains in `run()` (before any
satkit use, per the existing convention).

```rust
// before
pub fn run() {
    simulation::init();
    let satellites = vec![Satellite::from_tle(ISS_TLE)];
    let simulation = SimulationState::new(satellites);
    application::run(ApplicationState::new(simulation));
}

// after
pub fn run() {
    simulation::init();
    application::run(ApplicationState::new(IssSimulation::new()));
}
```

---

## File-by-file change summary

### `src/simulation/mod.rs`

- Add `pub trait Simulation { ... }` with the four methods above.
- Remove `satellites: Vec<Satellite>` from `SimulationState`.
- Change `SimulationState::new(satellites: Vec<Satellite>)` to
  `SimulationState::new(start_epoch: satkit::Instant)`.
- Remove `SimulationState::frame_state` entirely.
- Keep `SimulationState::advance` and `SimulationState::celestial_to_world` as
  `pub` helpers (scenario impls delegate to them).
- Change `marker_occluded` from private to `pub(crate)` so scenario impls in
  `crate::scenarios` can call it.
- Update the module doc comment and `SimulationState` doc comment.
- `pub use clock::Clock` so the trait signature's `&mut Clock` is importable by
  users of `Simulation` without knowing the submodule path.

### `src/scenarios/iss.rs`

- Add `pub struct IssSimulation { simulation: SimulationState, satellites: Vec<Satellite> }`.
- Add `impl IssSimulation { fn new() -> Self { ... } }`.
- Add `impl Simulation for IssSimulation { ... }`.
- Simplify `run()` as shown above.
- Update imports.

### `src/scenarios/iss_and_hubble.rs`

- Same pattern: `IssAndHubbleSimulation` struct + `Simulation` impl + simplified `run()`.

### `src/application/mod.rs`

- Make `ApplicationState` generic over `S: Simulation`.
- Change the `simulation` field type from `SimulationState` to `S`.
- Make `run` and all `impl` blocks generic over `S: Simulation`.
- Change `&mut self.simulation.clock` to `self.simulation.clock_mut()` in the
  `redraw` method (the one line that accesses the field directly).
- Update imports: replace `use crate::simulation::SimulationState` with
  `use crate::simulation::Simulation`.
- Update module and struct doc comments.

### `src/ui.rs`

No changes. It already takes `&TelemetryState` and `&mut Clock` — the clock is
now delivered via `clock_mut()` but the function signature is unaffected.

### `src/simulation/satellite.rs`

No changes. `Satellite` and `SatelliteState` are unchanged.

### `src/simulation/clock.rs`

No changes.

### `src/simulation/celestial_sphere.rs`

No changes.

### `src/snapshot.rs`

No changes. The headless render mode constructs `RenderState` directly and does
not use `SimulationState` (it never did).

### `src/main.rs`

No changes to the dispatch logic. The scenario `run()` signatures are unchanged.

---

## What does NOT change

- `RenderState`, `TelemetryState`, `SatelliteMarker`, `SatelliteTelemetry` —
  unchanged; they are the currency the trait passes.
- `Clock`, `CelestialSphere`, `Satellite` — unchanged internally.
- The satellite propagation logic itself — lifted verbatim from
  `SimulationState::frame_state` into each scenario impl.
- The TLE `const`s — stay in their scenario files, deliberately duplicated (per
  the existing convention).
- `simulation::init()` call convention — stays in each scenario's `run()`.
- The `application -> simulation` dependency direction — scenarios still import
  from `simulation`; `simulation` does not import from `scenarios`.

---

## Open questions / decisions already made

| Topic | Decision |
|---|---|
| Storage in `ApplicationState` | Generic `S: Simulation` (no boxing, zero vtable cost) |
| Clock access from UI | `fn clock_mut(&mut self) -> &mut Clock` on the trait |
| `SimulationState` name | Keep as-is; it is still "state of the simulation core" |
| `simulation::init()` call site | Stay in `run()`, before the scenario struct's `new()` |
| `marker_occluded` visibility | Widen to `pub(crate)` so scenario impls can call it |
| Scenario struct naming | `<Name>Simulation` (e.g. `IssSimulation`) |

---

## Duplication note

Each scenario's `frame_state` impl will contain a nearly identical satellite
propagation loop. This is intentional: the code is short (~20 lines), and each
scenario may in future need to deviate (different marker colours, custom
visibility logic, non-satellite objects). Premature factoring would just add
indirection before any variant exists. If the loop becomes non-trivial or a third
scenario arrives with the same body, consider a free helper in `simulation/mod.rs`
at that point.
