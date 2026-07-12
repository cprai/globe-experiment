//! `manual_control` with its UI panels produced by the `--script` Python
//! file (reference: `scenes/manual_control_py.py`; kept side by side with
//! the Rust sibling for API comparison). Simulation state lives in the
//! [`ManualControlSceneInner`] pyclass handed live to the script; the
//! [`ManualControlPyScene`] wrapper owns the clock/camera/target as plain
//! fields and implements the engine traits. Never hold a pyclass borrow
//! across a call into Python.

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

// Seed TLE - see `iss.rs` for the column-sensitive format notes.
// Deliberately duplicated per scene - do not factor into a shared const.
// Used ONCE to bootstrap the initial GCRF state.

/// The International Space Station (ISS / ZARYA), epoch 2024-001.5. Real
/// element set - the starting orbit only.
const ISS_TLE: &str = concat!(
    "ISS (ZARYA)\n",
    "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9003\n",
    "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299357\n",
);

/// Thrust while a burn key is held, m/s^2 - the same game-like ~1 g as the
/// Rust sibling (see `manual_control.rs` for the rationale).
const BURN_ACCEL_M_S2: f64 = 10.0;

/// The live simulation state, as a pyclass: the script's whole surface.
/// The clock and camera are deliberately NOT in here - they live on the
/// wrapper, out of the script's reach.
#[pyclass(name = "ManualControlScene", module = "globe")]
pub struct ManualControlSceneInner {
    /// Object name from the seed TLE, for the panel header.
    #[pyo3(get)]
    name: String,
    /// THE orbit - burns mutate its velocity, and each frame's numerical
    /// propagation carries the result forward. No TLE behind it after seeding.
    orbit: OrbitState,
    /// The instant `orbit` is valid at; advanced to the clock each frame.
    orbit_epoch: Instant,
    /// Burn request flags, set by the script's `request_*` calls during the
    /// egui pass. Flags (not direct edits) because the burn is dt-scaled:
    /// only `advance` knows the frame's simulation dt.
    burn_prograde: bool,
    burn_retrograde: bool,
    burn_normal: bool,
    burn_anti_normal: bool,
    burn_radial_out: bool,
    burn_radial_in: bool,
    /// Script-facing clock mirror: snapshots the wrapper refreshes each
    /// frame, plus `requested_*` edits the wrapper folds into the
    /// wrapper-owned `Clock` before the next tick.
    paused: bool,
    multiplier: f32,
    datetime_label: String,
    requested_paused: Option<bool>,
    requested_multiplier: Option<f32>,
}

/// The script-facing surface. Bound methods double as panel callbacks: the
/// script passes e.g. `scene.request_prograde` straight to a hold button.
#[pymethods]
impl ManualControlSceneInner {
    /// Lat/lon/alt readout at the frame's clock instant - a pure frame
    /// change: `advance` re-anchored the orbit, so `orbit_epoch` IS the
    /// frame's clock reading and the readout matches the rendered marker.
    fn telemetry(&self) -> SatelliteTelemetry {
        let state = satellite::resolve_orbit(&self.orbit, &self.orbit_epoch);
        SatelliteTelemetry {
            name: self.name.clone(),
            latitude_deg: state.latitude_deg as f32,
            longitude_deg: state.longitude_deg as f32,
            altitude_km: state.altitude_km as f32,
        }
    }

    /// Osculating apo/peri/speed; `None` after a burn to escape (e >= 1: no
    /// apsides - the script shows dashes).
    fn orbit_shape(&self) -> Option<OrbitShape> {
        satellite::orbit_shape(&self.orbit)
    }

    /// Current inertial speed, m/s - the escape-case fallback readout.
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

    // Clock mirror properties. Getters return frame snapshots, so both
    // discard-pass fires of a callback read the same value - idempotent.
    // Setters record requests folded in before the next tick. No `Clock`
    // instance crosses into Python.

    #[getter]
    fn get_paused(&self) -> bool {
        self.paused
    }

    #[setter]
    fn set_paused(&mut self, paused: bool) {
        self.requested_paused = Some(paused);
    }

    #[getter]
    fn get_multiplier(&self) -> f32 {
        self.multiplier
    }

    #[setter]
    fn set_multiplier(&mut self, multiplier: f32) {
        self.requested_multiplier = Some(multiplier);
    }

    /// The current simulation datetime formatted for display (UTC).
    fn datetime_label(&self) -> String {
        self.datetime_label.clone()
    }
}

/// The Rust-only half: construction and per-frame simulation work, mirroring
/// the Rust sibling body for body.
impl ManualControlSceneInner {
    fn new() -> Self {
        // The TLE lives exactly long enough to produce the initial
        // conditions (see the Rust sibling).
        let mut seed = Satellite::from_tle(ISS_TLE);
        let epoch = seed.epoch();
        let orbit = seed.state_at(&epoch).orbit;
        Self {
            name: seed.name,
            orbit,
            orbit_epoch: epoch,
            burn_prograde: false,
            burn_retrograde: false,
            burn_normal: false,
            burn_anti_normal: false,
            burn_radial_out: false,
            burn_radial_in: false,
            // Mirror defaults matching a fresh `Clock`; the wrapper's `new`
            // pushes the real snapshots right after it builds the clock.
            paused: false,
            multiplier: Clock::MIN_MULTIPLIER,
            datetime_label: String::new(),
            requested_paused: None,
            requested_multiplier: None,
        }
    }

    /// Unit thrust direction for the requested burns, in GCRF (identical to
    /// the Rust sibling).
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

    fn advance(&mut self, now: &Instant) {
        // Re-anchor to the clock instant the wrapper just ticked (the clock
        // lives on the wrapper, so the frame instant is passed in).
        let dt = (*now - self.orbit_epoch).as_seconds();
        if dt > 0.0 {
            self.orbit = satellite::propagate_numerical(&self.orbit, &self.orbit_epoch, now);
            self.orbit_epoch = *now;

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
    }
}

/// What the application owns: the `Py` handle, the script's `get_drawables`
/// (loaded once at startup), and the clock + camera + target as plain fields.
pub struct ManualControlPyScene {
    inner: Py<ManualControlSceneInner>,
    get_drawables_fn: Py<PyAny>,
    /// Kept only so per-frame failures name the file that raised.
    script: PathBuf,
    /// Deliberately OUTSIDE the pyclass: `SceneClock::clock_mut` hands out
    /// `&mut Clock`, which a pyclass cell's borrow guard could not. The
    /// script drives it through the Inner's snapshot/request mirror.
    clock: Clock,
    /// Deliberately OUTSIDE the pyclass: the script has no camera surface,
    /// and only a plain field can hand out the `&mut` borrows
    /// `ScenePtzCamera` needs.
    camera: PtzCamera,
    /// Fixed at Terra (no selector).
    camera_target: CameraTarget,
}

impl ManualControlPyScene {
    fn new(script: PathBuf) -> Self {
        let mut scene = Python::attach(|py| {
            let inner = Py::new(py, ManualControlSceneInner::new()).expect("scene pyclass");
            // The clock starts at the seed TLE's epoch, read back off the
            // freshly built Inner.
            let epoch = inner.borrow(py).orbit_epoch;
            Self {
                inner,
                get_drawables_fn: py::load_get_drawables(py, &script),
                script,
                clock: Clock::new(epoch),
                camera: PtzCamera::default(),
                camera_target: CameraTarget::terra(),
            }
        });
        // Seed the script-facing snapshots so a `get_drawables` that runs
        // before the first advance (the test harnesses do) reads real clock
        // values instead of the mirror defaults.
        scene.push_clock_snapshots();
        scene
    }

    /// Refreshes the Inner's script-facing clock snapshots from the wrapper
    /// clock. Values are read before attaching: the `SceneClock` getters
    /// need `&mut self`, which cannot overlap the cell borrow of `self.inner`.
    fn push_clock_snapshots(&mut self) {
        let paused = self.clock_paused();
        let multiplier = self.clock_multiplier();
        let datetime_label = self.clock_datetime_label();
        Python::attach(|py| {
            let mut inner = self.inner.borrow_mut(py);
            inner.paused = paused;
            inner.multiplier = multiplier;
            inner.datetime_label = datetime_label;
        });
    }
}

impl SceneClock for ManualControlPyScene {
    fn clock_mut(&mut self) -> &mut Clock {
        &mut self.clock
    }
}

impl Scene for ManualControlPyScene {
    fn tick_scene(&mut self) {
        // Fold the script's requested clock edits into the wrapper clock
        // BEFORE ticking - the same frame timing as the Rust sibling's
        // direct callbacks. Requests are drained out of the cell first: the
        // SceneClock setters need `&mut self`, which cannot overlap the cell
        // borrow.
        let (paused, multiplier) = Python::attach(|py| {
            let mut inner = self.inner.borrow_mut(py);
            (
                inner.requested_paused.take(),
                inner.requested_multiplier.take(),
            )
        });
        if let Some(paused) = paused {
            self.set_clock_paused(paused);
        }
        if let Some(multiplier) = multiplier {
            self.set_clock_multiplier(multiplier);
        }

        // The trait default's body: tick, then the scene-specific advance.
        self.tick_clock();
        self.advance();
    }

    fn advance(&mut self) {
        // Re-anchor the orbit to the already-ticked clock, then refresh the
        // Inner's clock snapshots for the coming get_drawables.
        let now = self.clock_now();
        Python::attach(|py| self.inner.borrow_mut(py).advance(&now));
        self.push_clock_snapshots();
    }
}

impl ScenePtzCamera for ManualControlPyScene {
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
        // Only the orbit state is read out of the pyclass cell - a transient
        // borrow; nothing here calls into Python.
        let now = self.clock_now();
        let sphere = CelestialSphere::at(&now);

        let celestial_to_world = sphere.star_rot_inv.transpose();
        let target = self.camera_target;
        let (eye, look_at, up) = self.camera.world_rig(&target, &sphere, celestial_to_world);

        // `advance` just re-anchored the state to `now`: pure frame change.
        let orbit = Python::attach(|py| self.inner.borrow(py).orbit);
        let state = satellite::resolve_orbit(&orbit, &now);
        let markers = vec![SatelliteMarker {
            position_km: state.position_km,
            visible: !marker_occluded(eye, state.position_km),
            propagation: Propagation::Numerical(orbit),
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

impl UIDrawable for ManualControlPyScene {
    /// Calls the script's `get_drawables(scene)` with the live pyclass
    /// handle. No Rust borrow of the inner cell is held while the script
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

/// The `scene manual_control_py` CLI arguments. `--script` is declared here
/// (not on a shared arg set) so clap itself requires it for this scene and
/// rejects it for the others.
#[derive(clap::Args)]
pub struct Args {
    /// Path to the scene's Python panel script (read at runtime: edit +
    /// relaunch, no rebuild).
    #[arg(long)]
    pub script: PathBuf,
}

/// Builds the Python-paneled manual-control scene and runs the winit event
/// loop until close.
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

    /// One CPU-only egui pass over the scene's panels. Panels are built
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

    fn clock_paused(scene: &mut ManualControlPyScene) -> bool {
        scene.clock_paused()
    }

    /// The repo's reference script, resolved against the manifest dir so the
    /// test runs from any cwd.
    fn repo_script() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("scenes")
            .join("manual_control_py.py")
    }

    /// The full Python round trip, GPU-free but driving the real script:
    /// three panels come back converted, and a synthetic click on the Run key
    /// flips the live clock through the script's Python callback.
    #[test]
    fn python_scene_round_trip() {
        celestial_sphere::init_satkit_for_tests();
        py::init();
        let mut scene = ManualControlPyScene::new(repo_script());

        let panels = scene.get_drawables();
        assert_eq!(panels.len(), 3, "script must return three panels");
        assert!(matches!(panels[0].anchor, PanelAnchor::TopLeft));
        assert!(matches!(panels[1].anchor, PanelAnchor::TopRight));
        assert!(matches!(panels[2].anchor, PanelAnchor::BottomCenter));
        drop(panels);

        // Probe click positions down the top-left panel until one lands on
        // the Run key (taffy sizes the panel to content, so the rect is not
        // known a priori). The script's on_toggle records a `paused` request
        // on the Inner; the next `tick_scene` folds it into the wrapper
        // clock - the assertion target.
        assert!(!clock_paused(&mut scene), "clock must start running");
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
                scene.tick_scene();
                if clock_paused(&mut scene) {
                    return;
                }
                x += 12.0;
            }
            y += 10.0;
        }
        panic!("no probed click reached the Run key");
    }
}
