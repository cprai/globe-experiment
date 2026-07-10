//! The manual-control scene with its UI panels produced by **Python**: a
//! clone of `manual_control` (same physics, camera, and burn semantics) whose
//! `UIDrawable::get_drawables` delegates to a Python script whose path is
//! the scene's required `--script` argument (the repo ships
//! `scenes/manual_control_py.py`). The two scenes live side by side so the
//! Rust and Python panel APIs can be compared line for line (CLI:
//! `globe-experiment scene manual_control_py --script
//! scenes/manual_control_py.py`).
//!
//! Structure: the scene state lives in [`ManualControlSceneInner`], itself a
//! `#[pyclass]` - the script receives the *live* scene object and reads/
//! drives it through its curated Python surface (the shared `clock` handle,
//! the telemetry/orbit-shape readouts, the six burn-request methods). Rust
//! owns it as [`ManualControlPyScene`], a thin wrapper holding the `Py<..>`
//! handle plus the script's `get_drawables` function; each trait method
//! attaches to the interpreter and borrows the inner pyclass cell for its own
//! duration only - no borrow is ever held across a call into Python, which
//! is what keeps the script's re-entrant property/method borrows safe.
//!
//! Error policy: a failing script (load, compile, or a per-frame
//! `get_drawables` exception) panics with the traceback printed - it would
//! recur every frame, so fail-fast beats limping. A failing *callback* only
//! prints (see `ui::py`).

use std::path::PathBuf;

use glam::DVec3;
use pyo3::prelude::*;
use satkit::Instant;

use crate::engine::application::{self, ApplicationState};
use crate::engine::camera::{
    CameraControl, CameraView, CursorHint, PointerButton, PtzCamera, ScrollDelta,
};
use crate::engine::py;
use crate::engine::scene::celestial_sphere::CelestialSphere;
use crate::engine::scene::satellite::{self, OrbitShape, OrbitState, Propagation, Satellite};
use crate::engine::scene::{
    self, CameraTarget, Clock, RenderState, SatelliteMarker, SatelliteTelemetry, Scene,
    marker_occluded,
};
use crate::engine::ui::{self, UIDrawable, UIDrawablePanel};

// This scene's seed TLE, inlined as a source literal - see `iss.rs` for the
// format notes. (Deliberately duplicated per scene.) Used ONCE, to bootstrap
// the initial GCRF state vector; after that the orbit belongs to the user.

/// The International Space Station (ISS / ZARYA), epoch 2024-001.5. Real
/// element set - the starting orbit only.
const ISS_TLE: &str = concat!(
    "ISS (ZARYA)\n",
    "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9003\n",
    "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299357\n",
);

/// Thrust acceleration while a burn key is held, m/s^2 - the same game-like
/// ~1 g as the Rust sibling (see `manual_control.rs` for the rationale).
const BURN_ACCEL_M_S2: f64 = 10.0;

/// The live scene state, as a pyclass: the Python side of the scene. The
/// script sees `scene.clock` (the shared [`Clock`] handle - its Run/speed
/// callbacks mutate the same clock Rust ticks), `scene.name`, the readout
/// methods, and the burn-request methods; everything else (orbit state,
/// camera) stays Rust-private. `clock` is held as `Py<Clock>` (not a plain
/// field) precisely so both sides mutate one object; the cost is a
/// `borrow(py)` on the Rust side's accesses.
#[pyclass(name = "ManualControlScene", module = "globe")]
pub struct ManualControlSceneInner {
    /// Simulation clock (datetime + play/paused + speed), shared with the
    /// script. The getter hands Python the same live object.
    #[pyo3(get)]
    clock: Py<Clock>,
    /// Object name from the seed TLE, for the panel header.
    #[pyo3(get)]
    name: String,
    /// The satellite's GCRF state vector, valid at `orbit_epoch`. THE orbit -
    /// burns mutate its velocity, and each frame's numerical propagation
    /// carries the result forward. No TLE behind it after seeding.
    orbit: OrbitState,
    /// The instant `orbit` is valid at; advanced to the clock each frame.
    orbit_epoch: Instant,
    /// Burn request flags, one per key. The script's held keys call the
    /// `request_*` methods during the egui pass; `advance` folds the flags
    /// into a velocity change next frame, then clears them. (The Rust sibling
    /// needs six disjoint fields for its closure captures; the Python
    /// callbacks all go through `&mut self` method calls on the pyclass cell,
    /// which cannot overlap by construction, but the fields are kept
    /// one-per-key for the same panel semantics.)
    burn_prograde: bool,
    burn_retrograde: bool,
    burn_normal: bool,
    burn_anti_normal: bool,
    burn_radial_out: bool,
    burn_radial_in: bool,
    /// The scene's interactive orbital camera (pan/tilt/zoom rig plus its
    /// animations); the default whole-Terra view.
    camera: PtzCamera,
    /// The body the camera orbits - fixed at Terra here (no selector).
    camera_target: CameraTarget,
}

/// The script-facing surface. Bound methods double as panel callbacks: the
/// script passes e.g. `scene.request_prograde` straight to an
/// `InteractiveHoldButton`.
#[pymethods]
impl ManualControlSceneInner {
    /// The satellite's lat/lon/alt readout at the frame's clock instant - a
    /// pure frame change (`advance` re-anchored the orbit to the same
    /// instant), so it matches the rendered marker. f64 -> f32 at this
    /// egui-facing boundary, same as the Rust sibling.
    fn telemetry(&self, py: Python<'_>) -> SatelliteTelemetry {
        let now = self.clock.borrow(py).now();
        let state = satellite::resolve_orbit(&self.orbit, &now);
        SatelliteTelemetry {
            name: self.name.clone(),
            latitude_deg: state.latitude_deg as f32,
            longitude_deg: state.longitude_deg as f32,
            altitude_km: state.altitude_km as f32,
        }
    }

    /// Osculating apo/peri/speed of the live orbit; `None` after a burn to
    /// escape (e >= 1: no apsides - the script shows dashes).
    fn orbit_shape(&self) -> Option<OrbitShape> {
        satellite::orbit_shape(&self.orbit)
    }

    /// Current inertial speed, m/s - the escape-case fallback readout (the
    /// elliptic case reads it off `orbit_shape`).
    fn speed_m_s(&self) -> f64 {
        self.orbit.vel_gcrf_m_s.length()
    }

    fn request_prograde(&mut self) {
        self.burn_prograde = true;
    }

    fn request_retrograde(&mut self) {
        self.burn_retrograde = true;
    }

    fn request_normal(&mut self) {
        self.burn_normal = true;
    }

    fn request_anti_normal(&mut self) {
        self.burn_anti_normal = true;
    }

    fn request_radial_out(&mut self) {
        self.burn_radial_out = true;
    }

    fn request_radial_in(&mut self) {
        self.burn_radial_in = true;
    }
}

/// The Rust-only half: construction and the per-frame work, mirroring the
/// Rust sibling body for body (only the clock accesses differ - `borrow`
/// through the shared `Py<Clock>` cell).
impl ManualControlSceneInner {
    fn new(py: Python<'_>) -> Self {
        // The TLE lives exactly long enough to produce the initial conditions
        // (see the Rust sibling).
        let mut seed = Satellite::from_tle(ISS_TLE);
        let epoch = seed.epoch();
        let orbit = seed.state_at(&epoch).orbit;
        Self {
            clock: Py::new(py, Clock::new(epoch)).expect("clock pyclass"),
            name: seed.name,
            orbit,
            orbit_epoch: epoch,
            burn_prograde: false,
            burn_retrograde: false,
            burn_normal: false,
            burn_anti_normal: false,
            burn_radial_out: false,
            burn_radial_in: false,
            camera: PtzCamera::default(),
            camera_target: CameraTarget::terra(),
        }
    }

    /// The unit thrust direction for the currently-requested burns, in GCRF
    /// (identical to the Rust sibling).
    fn burn_direction(&self) -> Option<DVec3> {
        let radial = self.orbit.pos_gcrf_m.normalize();
        let prograde = self.orbit.vel_gcrf_m_s.normalize();
        let normal = self
            .orbit
            .pos_gcrf_m
            .cross(self.orbit.vel_gcrf_m_s)
            .normalize();

        let mut sum = DVec3::ZERO;
        if self.burn_prograde {
            sum += prograde;
        }
        if self.burn_retrograde {
            sum -= prograde;
        }
        if self.burn_normal {
            sum += normal;
        }
        if self.burn_anti_normal {
            sum -= normal;
        }
        if self.burn_radial_out {
            sum += radial;
        }
        if self.burn_radial_in {
            sum -= radial;
        }
        sum.try_normalize()
    }

    fn advance(&mut self, py: Python<'_>) -> bool {
        // Tick the shared clock; the borrow is scoped so it ends before any
        // other work (nothing below re-enters Python, but keeping borrows
        // tight is the rule that makes that always true).
        let (running, now) = {
            let mut clock = self.clock.borrow_mut(py);
            let running = clock.tick();
            (running, clock.now())
        };

        // Re-anchor the state vector to the clock (see the Rust sibling).
        let dt = (now - self.orbit_epoch).as_seconds();
        if dt > 0.0 {
            self.orbit = satellite::propagate_numerical(&self.orbit, &self.orbit_epoch, &now);
            self.orbit_epoch = now;

            if let Some(direction) = self.burn_direction() {
                self.orbit.vel_gcrf_m_s += direction * (BURN_ACCEL_M_S2 * dt);
            }
        }

        // Held keys re-request during the coming egui pass; clearing here is
        // what makes "held" mean "burning".
        self.burn_prograde = false;
        self.burn_retrograde = false;
        self.burn_normal = false;
        self.burn_anti_normal = false;
        self.burn_radial_out = false;
        self.burn_radial_in = false;

        running
    }

    fn frame_state(&mut self, py: Python<'_>) -> RenderState {
        let now = self.clock.borrow(py).now();
        // This frame's celestial sphere, evaluated on the spot (pure function
        // of time, same as the Rust sibling).
        let sphere = CelestialSphere::at(&now);

        let celestial_to_world = sphere.star_rot_inv.transpose();
        let target = self.camera_target;
        let (eye, look_at, up) = self.camera.world_rig(&target, &sphere, celestial_to_world);

        // `advance` just re-anchored the state to `now`: pure frame change.
        let state = satellite::resolve_orbit(&self.orbit, &now);
        let markers = vec![SatelliteMarker {
            position_km: state.position_km,
            visible: !marker_occluded(eye, state.position_km),
            propagation: Propagation::Numerical(self.orbit),
        }];

        RenderState {
            time: now,
            camera_target: target,
            camera_pos: eye,
            camera_look_at: look_at,
            camera_up: up,
            markers,
        }
    }
}

/// What the application owns: the `Py` handle to the scene pyclass plus the
/// script's `get_drawables` function (loaded once at startup). Every trait
/// method opens its own interpreter attach + pyclass borrow.
pub struct ManualControlPyScene {
    inner: Py<ManualControlSceneInner>,
    get_drawables_fn: Py<PyAny>,
    /// The script's CLI-given path, kept only so per-frame failures name the
    /// file that raised (the function itself is already loaded).
    script: PathBuf,
}

impl ManualControlPyScene {
    fn new(script: PathBuf) -> Self {
        Python::attach(|py| Self {
            inner: Py::new(py, ManualControlSceneInner::new(py)).expect("scene pyclass"),
            get_drawables_fn: py::load_get_drawables(py, &script),
            script,
        })
    }
}

impl Scene for ManualControlPyScene {
    fn advance(&mut self) -> bool {
        Python::attach(|py| self.inner.borrow_mut(py).advance(py))
    }
}

impl CameraControl for ManualControlPyScene {
    // The forwarding block every scene duplicates, with one extra hop: attach
    // + borrow the pyclass cell, then forward to the embedded PtzCamera.
    fn pointer_press(&mut self, button: PointerButton) -> bool {
        Python::attach(|py| self.inner.borrow_mut(py).camera.pointer_press(button))
    }

    fn pointer_release(&mut self, button: PointerButton) -> bool {
        Python::attach(|py| self.inner.borrow_mut(py).camera.pointer_release(button))
    }

    fn pointer_move(&mut self, position: (f64, f64), viewport_height: f64) -> bool {
        Python::attach(|py| {
            let mut inner = self.inner.borrow_mut(py);
            let inner = &mut *inner;
            inner
                .camera
                .pointer_move(&inner.camera_target, position, viewport_height)
        })
    }

    fn scroll(&mut self, delta: ScrollDelta) -> bool {
        Python::attach(|py| {
            let mut inner = self.inner.borrow_mut(py);
            let inner = &mut *inner;
            inner.camera.scroll(&inner.camera_target, delta)
        })
    }

    fn tick(&mut self, viewport_height: f64) -> bool {
        Python::attach(|py| {
            let mut inner = self.inner.borrow_mut(py);
            let inner = &mut *inner;
            inner.camera.tick(&inner.camera_target, viewport_height)
        })
    }

    fn cursor_hint(&self) -> CursorHint {
        Python::attach(|py| self.inner.borrow(py).camera.cursor_hint())
    }
}

impl CameraView for ManualControlPyScene {
    fn frame_state(&mut self) -> RenderState {
        Python::attach(|py| self.inner.borrow_mut(py).frame_state(py))
    }
}

impl UIDrawable for ManualControlPyScene {
    /// The Python delegation this scene exists for: call the script's
    /// `get_drawables(scene)` with the live pyclass handle and convert its
    /// panels. No Rust borrow of the inner cell is held while the script
    /// runs - its property/method accesses each take their own transient
    /// borrow.
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        Python::attach(|py| {
            self.get_drawables_fn
                .bind(py)
                .call1((self.inner.bind(py),))
                .and_then(|panels| ui::py::panels_from_python(py, &panels))
                .unwrap_or_else(|err| {
                    err.print(py);
                    panic!("{}: get_drawables failed", self.script.display());
                })
        })
    }
}

/// The `scene manual_control_py` CLI arguments. Only the Python-paneled
/// scenes have a script: declaring it here (not on a shared scene arg set)
/// is what lets clap itself require it for this scene and reject it for the
/// others.
#[derive(clap::Args)]
pub struct Args {
    /// Path to the scene's Python panel script, e.g.
    /// `scenes/manual_control_py.py` (read at runtime: edit + relaunch, no
    /// rebuild).
    #[arg(long)]
    pub script: PathBuf,
}

/// Builds the Python-paneled manual-control scene around the `--script`
/// panel script and hands off to the winit event loop. Blocks until the
/// window closes.
pub fn run(args: Args) {
    // satkit globals first (TLE parse + propagation), then the embedded
    // interpreter (inittab before init - see `engine::py`), then the scene,
    // whose construction loads the script.
    scene::init();
    py::init();

    application::run(ApplicationState::new(ManualControlPyScene::new(
        args.script,
    )));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::scene::celestial_sphere;
    use crate::engine::ui::{self, PanelAnchor};

    const SCREEN: egui::Vec2 = egui::vec2(900.0, 700.0);

    /// One CPU-only egui pass over the scene's panels (the real Python
    /// round trip: script -> Panel pyclasses -> converted instruments ->
    /// egui render, callbacks fired by egui's hit test).
    fn run_frame(
        ctx: &egui::Context,
        scene: &mut ManualControlPyScene,
        time: f64,
        events: Vec<egui::Event>,
    ) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
            time: Some(time),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| ui::control_panel(ui.ctx(), scene));
    }

    fn click_events(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            },
        ]
    }

    fn clock_paused(scene: &ManualControlPyScene) -> bool {
        Python::attach(|py| scene.inner.borrow(py).clock.borrow(py).paused)
    }

    /// The repo's reference script, resolved against the manifest dir so the
    /// test runs from any cwd - the same file the CLI example passes.
    fn repo_script() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("scenes")
            .join("manual_control_py.py")
    }

    /// The full Python round trip, render-free of the GPU but driving the
    /// real script: three panels come back converted, and a synthetic click
    /// on the top-left panel's Run key flips the live clock through the
    /// script's Python callback (proving state flows Python -> Rust).
    #[test]
    fn python_scene_round_trip() {
        celestial_sphere::init_satkit_for_tests();
        py::init();
        let mut scene = ManualControlPyScene::new(repo_script());

        // Shape: the script returns the same three panels as the Rust
        // sibling (Time top-left, telemetry top-right, Burns bottom-center).
        let panels = scene.get_drawables();
        assert_eq!(panels.len(), 3, "script must return three panels");
        assert!(matches!(panels[0].anchor, PanelAnchor::TopLeft));
        assert!(matches!(panels[1].anchor, PanelAnchor::TopRight));
        assert!(matches!(panels[2].anchor, PanelAnchor::BottomCenter));
        drop(panels);

        // Interactivity: probe click positions down the top-left panel until
        // one lands on the Run key (taffy sizes the panel to content, so the
        // rect is not known a priori - the hold-button test's probe pattern).
        // The clock starts running; the script's on_toggle flips `paused`
        // through the shared Py<Clock>, which is the assertion target.
        assert!(!clock_paused(&scene), "clock must start running");
        let mut y = 8.0;
        while y < 300.0 {
            let mut x = 8.0;
            while x < 350.0 {
                let pos = egui::pos2(x, y);
                let ctx = egui::Context::default();
                ui::install_theme(&ctx);
                run_frame(&ctx, &mut scene, 0.0, Vec::new());
                run_frame(&ctx, &mut scene, 0.05, click_events(pos, true));
                run_frame(&ctx, &mut scene, 0.10, click_events(pos, false));
                if clock_paused(&scene) {
                    return;
                }
                x += 12.0;
            }
            y += 10.0;
        }
        panic!("no probed click reached the Run key");
    }
}
