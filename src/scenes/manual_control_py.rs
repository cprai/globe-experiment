//! The manual-control scene with its UI panels produced by **Python**: a
//! clone of `manual_control` (same physics, camera, and burn semantics) whose
//! `UIDrawable::get_drawables` delegates to a Python script whose path is
//! the scene's required `--script` argument (the repo ships
//! `scenes/manual_control_py.py`). The two scenes live side by side so the
//! Rust and Python panel APIs can be compared line for line (CLI:
//! `globe-experiment scene manual_control_py --script
//! scenes/manual_control_py.py`).
//!
//! Structure: the simulation state lives in [`ManualControlSceneInner`],
//! itself a `#[pyclass]` - the script receives the *live* scene object and
//! reads/drives it through its curated Python surface (the clock properties
//! `paused`/`multiplier`/`datetime_label()`, the Python face of the
//! `SceneClock` trait API, plus the telemetry/orbit-shape readouts and the
//! six burn-request methods). Rust
//! owns it as [`ManualControlPyScene`], a thin wrapper holding the `Py<..>`
//! handle, the script's `get_drawables` function, and the camera + its
//! target as plain wrapper fields - deliberately OUTSIDE the pyclass (the
//! script has no camera surface), which lets the wrapper implement
//! `ScenePtzCamera` like every Rust scene and take the blanket
//! `CameraControl` impl. The remaining trait methods
//! attach to the interpreter and borrow the inner pyclass cell for their own
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
use crate::engine::camera::{CameraView, PtzCamera, ScenePtzCamera};
use crate::engine::py;
use crate::engine::scene::celestial_sphere::CelestialSphere;
use crate::engine::scene::satellite::{self, OrbitShape, OrbitState, Propagation, Satellite};
use crate::engine::scene::{
    self, CameraTarget, Clock, RenderState, SatelliteMarker, SatelliteTelemetry, Scene, SceneClock,
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

/// The live simulation state, as a pyclass: the Python side of the scene.
/// The script sees the clock properties (`scene.paused`/`scene.multiplier`/
/// `scene.datetime_label()` - the `SceneClock` API re-exposed as pymethods,
/// so its Run/speed callbacks drive the same clock Rust ticks), `scene.name`,
/// the readout methods, and the burn-request methods; everything else (the
/// clock object itself, the orbit state) stays Rust-private. The camera is
/// not even in here: it lives on the wrapper, outside the pyclass, so a
/// script cannot reach it at all.
#[pyclass(name = "ManualControlScene", module = "globe")]
pub struct ManualControlSceneInner {
    /// Simulation clock (datetime + play/paused + speed), reached only via
    /// the `SceneClock` API (and, from the script, its pymethod face).
    clock: Clock,
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
    /// into a velocity change next frame, then clears them. The flags stay
    /// (in both siblings) because the burn is dt-scaled: only `advance`
    /// knows the frame's simulation dt.
    burn_prograde: bool,
    burn_retrograde: bool,
    burn_normal: bool,
    burn_anti_normal: bool,
    burn_radial_out: bool,
    burn_radial_in: bool,
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
    fn telemetry(&mut self) -> SatelliteTelemetry {
        let now = self.clock_now();
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

    // The clock properties - the `SceneClock` API's Python face. The script's
    // Run/speed callbacks assign these; no `Clock` instance crosses into
    // Python (`&mut self` getters are fine: each pyclass access takes its own
    // transient runtime borrow, and no Rust borrow is live during the script).

    #[getter]
    fn get_paused(&mut self) -> bool {
        self.clock_paused()
    }

    #[setter]
    fn set_paused(&mut self, paused: bool) {
        self.set_clock_paused(paused);
    }

    #[getter]
    fn get_multiplier(&mut self) -> f32 {
        self.clock_multiplier()
    }

    #[setter]
    fn set_multiplier(&mut self, multiplier: f32) {
        self.set_clock_multiplier(multiplier);
    }

    /// The current simulation datetime formatted for display (UTC).
    fn datetime_label(&mut self) -> String {
        self.clock_datetime_label()
    }
}

impl SceneClock for ManualControlSceneInner {
    fn clock_mut(&mut self) -> &mut Clock {
        &mut self.clock
    }
}

/// The Rust-only half: construction and the per-frame simulation work,
/// mirroring the Rust sibling body for body (only the Time-panel plumbing
/// differs - the script's callbacks apply through the pymethod face above,
/// so no `request_*` clock fields are needed here). The frame recipe
/// (`frame_state`) lives on the wrapper, beside the camera it rigs.
impl ManualControlSceneInner {
    fn new() -> Self {
        // The TLE lives exactly long enough to produce the initial conditions
        // (see the Rust sibling).
        let mut seed = Satellite::from_tle(ISS_TLE);
        let epoch = seed.epoch();
        let orbit = seed.state_at(&epoch).orbit;
        Self {
            clock: Clock::new(epoch),
            name: seed.name,
            orbit,
            orbit_epoch: epoch,
            burn_prograde: false,
            burn_retrograde: false,
            burn_normal: false,
            burn_anti_normal: false,
            burn_radial_out: false,
            burn_radial_in: false,
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

    fn advance(&mut self) -> bool {
        // Tick the clock through the SceneClock API (any script-requested
        // pause/speed change already landed via the pymethod setters during
        // the previous egui pass).
        let running = self.tick_clock();
        let now = self.clock_now();

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
}

/// What the application owns: the `Py` handle to the scene pyclass, the
/// script's `get_drawables` function (loaded once at startup), and the
/// camera. The pyclass-touching trait methods open their own interpreter
/// attach + pyclass borrow; the camera path never attaches at all.
pub struct ManualControlPyScene {
    inner: Py<ManualControlSceneInner>,
    get_drawables_fn: Py<PyAny>,
    /// The script's CLI-given path, kept only so per-frame failures name the
    /// file that raised (the function itself is already loaded).
    script: PathBuf,
    /// The scene's interactive orbital camera (pan/tilt/zoom rig plus its
    /// animations); the default whole-Terra view. A plain wrapper field,
    /// deliberately OUTSIDE the pyclass: the script has no camera surface,
    /// and keeping it here lets the wrapper hand out the real borrows
    /// `ScenePtzCamera` needs (a pyclass cell's borrow guard could not),
    /// taking the same blanket `CameraControl` impl as every Rust scene.
    camera: PtzCamera,
    /// The body the camera orbits - fixed at Terra here (no selector).
    camera_target: CameraTarget,
}

impl ManualControlPyScene {
    fn new(script: PathBuf) -> Self {
        Python::attach(|py| Self {
            inner: Py::new(py, ManualControlSceneInner::new()).expect("scene pyclass"),
            get_drawables_fn: py::load_get_drawables(py, &script),
            script,
            camera: PtzCamera::default(),
            camera_target: CameraTarget::terra(),
        })
    }
}

impl Scene for ManualControlPyScene {
    fn advance(&mut self) -> bool {
        Python::attach(|py| self.inner.borrow_mut(py).advance())
    }
}

impl ScenePtzCamera for ManualControlPyScene {
    // The accessors behind the blanket `CameraControl` impl, same as every
    // Rust scene - possible because the camera lives on the wrapper, not
    // behind the pyclass cell's borrow guard.
    fn camera(&self) -> &PtzCamera {
        &self.camera
    }

    fn camera_mut(&mut self) -> &mut PtzCamera {
        &mut self.camera
    }

    fn camera_target(&self) -> &CameraTarget {
        &self.camera_target
    }
}

impl CameraView for ManualControlPyScene {
    fn frame_state(&mut self) -> RenderState {
        // The Rust sibling's frame recipe, split across the wrapper/pyclass
        // boundary: the simulation state (clock, orbit) is borrowed from the
        // cell for the whole body - safe, nothing here calls into Python -
        // while the camera rig lives on the wrapper itself.
        Python::attach(|py| {
            let mut inner = self.inner.borrow_mut(py);
            let now = inner.clock_now();
            // This frame's celestial sphere, evaluated on the spot (pure
            // function of time, same as the Rust sibling).
            let sphere = CelestialSphere::at(&now);

            let celestial_to_world = sphere.star_rot_inv.transpose();
            let target = self.camera_target;
            let (eye, look_at, up) = self.camera.world_rig(&target, &sphere, celestial_to_world);

            // `advance` just re-anchored the state to `now`: pure frame
            // change.
            let state = satellite::resolve_orbit(&inner.orbit, &now);
            let markers = vec![SatelliteMarker {
                position_km: state.position_km,
                visible: !marker_occluded(eye, state.position_km),
                propagation: Propagation::Numerical(inner.orbit),
            }];

            RenderState {
                time: now,
                camera_target: target,
                camera_pos: eye,
                camera_look_at: look_at,
                camera_up: up,
                markers,
            }
        })
    }
}

impl UIDrawable for ManualControlPyScene {
    /// The Python delegation this scene exists for: call the script's
    /// `get_drawables(scene)` with the live pyclass handle and convert its
    /// panels. No Rust borrow of the inner cell is held while the script
    /// runs - its property/method accesses each take their own transient
    /// borrow.
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<Self>> {
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
    /// egui render, callbacks fired by egui's hit test). Panels are built
    /// once, before `run_ui`, like the live `redraw` path.
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
        let mut panels = scene.get_drawables();
        let _ = ctx.run_ui(input, |ui| ui::control_panel(ui.ctx(), &mut panels, scene));
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
        Python::attach(|py| scene.inner.borrow_mut(py).clock_paused())
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
        // The clock starts running; the script's on_toggle flips the scene's
        // `paused` property (the SceneClock API's Python face), which is the
        // assertion target.
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
