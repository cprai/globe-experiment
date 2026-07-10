//! The solar-system scene with its UI panels produced by **Python**: a clone
//! of `solar_system` (same nine-body Camera Target panel, camera, and clock)
//! whose `UIDrawable::get_drawables` delegates to a Python script whose path
//! is the scene's required `--script` argument (the repo ships
//! `scenes/solar_system_py.py`). The two scenes live side by side so the
//! Rust and Python panel APIs can be compared (CLI: `globe-experiment scene
//! solar_system_py --script scenes/solar_system_py.py`).
//!
//! Same structure as `manual_control_py`: the scene state is a `#[pyclass]`
//! ([`SolarSystemSceneInner`]) handed live to the script, wrapped by
//! [`SolarSystemPyScene`] for the four engine traits; borrows of the pyclass
//! cell never span a call into Python. The script sees the clock properties
//! (`paused`/`multiplier`/`datetime_label()` - a snapshot/request mirror of
//! the wrapper-owned clock) and the camera-target properties
//! (`selected_body` / `body_names()` / `request_body(i)`, the pymethod face
//! of the Rust sibling's direct camera-target write - the script cannot
//! touch the wrapper-owned camera, so the Inner holds the requested body and
//! the wrapper's `frame_state` folds it into the camera target, just as the
//! wrapper's `tick_scene` folds the requested clock edits). Script errors
//! fail fast (traceback + panic); callback errors only print (see
//! `ui::py`).

use std::path::PathBuf;

use pyo3::prelude::*;
use satkit::Instant;

use crate::engine::application::{self, ApplicationState};
use crate::engine::camera::{CameraView, PtzCamera, ScenePtzCamera};
use crate::engine::py;
use crate::engine::scene::celestial_sphere::CelestialSphere;
use crate::engine::scene::{
    self, CameraTarget, CelestialBody, Clock, RenderState, Scene, SceneClock,
};
use crate::engine::ui::{self, UIDrawable, UIDrawablePanel};

/// The live simulation state, as a pyclass: the script reads/drives the
/// clock properties (the snapshot/request mirror below) and the
/// camera-target properties. The clock and camera are not even in here:
/// they live on the wrapper, outside the pyclass, so a script's only
/// influence on either is what it requests through the mirror setters and
/// `request_body`.
#[pyclass(name = "SolarSystemScene", module = "globe")]
pub struct SolarSystemSceneInner {
    /// Script-facing clock mirror. The `Clock` itself lives on the WRAPPER
    /// (`SceneClock::clock_mut` hands out `&mut Clock`, which a pyclass
    /// cell's borrow guard could not), so the script reads these snapshots -
    /// refreshed by the wrapper after every advance - and its setters record
    /// the `requested_*` values the wrapper folds into the clock before the
    /// next tick: the clock twin of the `selected_body` fold below.
    paused: bool,
    multiplier: f32,
    datetime_label: String,
    requested_paused: Option<bool>,
    requested_multiplier: Option<f32>,
    /// Index into [`CelestialBody::ALL`] of the body the camera orbits -
    /// the Python face of the Rust sibling's direct camera-target write.
    /// The script reads it (`selected_body`) and requests switches
    /// (`request_body(i)`); the wrapper's `frame_state` folds it into the
    /// wrapper-owned `camera_target` (reframing the camera on a genuine
    /// switch), since the script cannot touch the camera itself.
    selected_body: usize,
}

/// The script-facing clock surface: the wrapper-owned clock mirrored as
/// pyclass properties. Getters return the frame's snapshots (pushed by the
/// wrapper after each advance), so both discard-pass fires of a callback
/// like `scene.paused = not scene.paused` read the same value - idempotent.
/// Setters record requests the wrapper folds into the clock before the next
/// tick (`tick_scene`), the same frame timing as the Rust sibling's direct
/// setter callbacks. No `Clock` instance crosses into Python.
#[pymethods]
impl SolarSystemSceneInner {
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

    /// Index into [`CelestialBody::ALL`] (= the panel's key order) of the
    /// body the camera orbits.
    #[getter]
    fn selected_body(&self) -> usize {
        self.selected_body
    }

    /// Every selectable body's display name, in [`CelestialBody::ALL`]
    /// (panel key) order - what the script labels its keys with.
    #[staticmethod]
    fn body_names() -> Vec<String> {
        CelestialBody::ALL
            .iter()
            .map(|body| body.name().to_string())
            .collect()
    }

    /// Requests a switch to `CelestialBody::ALL[index]` - the pymethod twin
    /// of a Rust key's direct camera-target write (a script callback fires
    /// during the egui pass; the wrapper's `frame_state` folds the new body
    /// into the camera target next frame). Writing a fixed index per key is
    /// idempotent under egui's discard-pass double fire.
    fn request_body(&mut self, index: usize) -> PyResult<()> {
        if index >= CelestialBody::ALL.len() {
            return Err(pyo3::exceptions::PyIndexError::new_err(format!(
                "body index {index} out of range 0..{}",
                CelestialBody::ALL.len()
            )));
        }
        self.selected_body = index;
        Ok(())
    }
}

/// The Rust-only half: construction. There is no per-frame Inner work at all
/// (the clock tick lives in the wrapper's `tick_scene`; the sole simulation
/// state is the script-requested body index, folded by the wrapper's
/// `frame_state`).
impl SolarSystemSceneInner {
    fn new() -> Self {
        Self {
            // Mirror defaults matching a fresh `Clock`; the wrapper's `new`
            // pushes the real snapshots right after it builds the clock.
            paused: false,
            multiplier: Clock::MIN_MULTIPLIER,
            datetime_label: String::new(),
            requested_paused: None,
            requested_multiplier: None,
            // Start on Terra (the familiar default view), matching the
            // wrapper's whole-Terra camera + Terra camera_target. Looked up
            // rather than hard-coded so the index space stays correct by
            // construction if `ALL`'s order ever changes.
            selected_body: CelestialBody::ALL
                .iter()
                .position(|body| *body == CelestialBody::TERRA)
                .expect("Terra is in CelestialBody::ALL"),
        }
    }
}

/// What the application owns: the `Py` handle to the scene pyclass, the
/// script's `get_drawables` function (loaded once at startup), the clock,
/// and the camera. The pyclass-touching trait methods open their own
/// interpreter attach + pyclass borrow; the clock and camera paths never
/// attach at all.
pub struct SolarSystemPyScene {
    inner: Py<SolarSystemSceneInner>,
    get_drawables_fn: Py<PyAny>,
    /// The script's CLI-given path, kept only so per-frame failures name the
    /// file that raised (the function itself is already loaded).
    script: PathBuf,
    /// Simulation clock (datetime + play/paused + speed), reached only via
    /// the `SceneClock` API. A plain wrapper field like the camera,
    /// deliberately OUTSIDE the pyclass: `SceneClock::clock_mut` hands out
    /// `&mut Clock`, which a pyclass cell's borrow guard could not. The
    /// script drives it through the Inner's snapshot/request mirror.
    clock: Clock,
    /// The scene's interactive orbital camera (pan/tilt/zoom rig plus its
    /// animations); starts on the default whole-Terra view. A plain wrapper
    /// field, deliberately OUTSIDE the pyclass: the script has no camera
    /// surface (only the `request_body` pymethod), and keeping it here lets
    /// the wrapper hand out the real borrows `ScenePtzCamera` needs (a
    /// pyclass cell's borrow guard could not), taking the same blanket
    /// `CameraControl` impl as every Rust scene.
    camera: PtzCamera,
    /// The body the camera orbits this frame - refreshed from the Inner's
    /// script-requested `selected_body` each frame; a genuine switch
    /// reframes the camera.
    camera_target: CameraTarget,
}

impl SolarSystemPyScene {
    fn new(script: PathBuf) -> Self {
        // A fixed recent past date, well inside the bundled EOP range (see
        // the Rust sibling).
        let epoch =
            Instant::from_datetime(2025, 6, 1, 0, 0, 0.0).expect("valid solar-system datetime");
        let mut scene = Python::attach(|py| Self {
            inner: Py::new(py, SolarSystemSceneInner::new()).expect("scene pyclass"),
            get_drawables_fn: py::load_get_drawables(py, &script),
            script,
            clock: Clock::new(epoch),
            camera: PtzCamera::default(),
            // Matches the Inner's start body (Terra) and the whole-Terra
            // camera above, so the first frame does not reframe.
            camera_target: CameraTarget::terra(),
        });
        // Seed the script-facing snapshots so a `get_drawables` that runs
        // before the first advance (the test harnesses do) reads real clock
        // values instead of the mirror defaults.
        scene.push_clock_snapshots();
        scene
    }

    /// Refreshes the Inner's script-facing clock snapshots (paused /
    /// multiplier / datetime label) from the wrapper clock, so the coming
    /// `get_drawables` reads current values. The values are read before
    /// attaching: the `SceneClock` getters need `&mut self`, which cannot
    /// overlap the cell borrow of `self.inner`.
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

impl SceneClock for SolarSystemPyScene {
    // The hook behind the `Scene` trait's provided `tick_scene`, same as
    // every Rust scene - possible because the clock lives on the wrapper,
    // not behind the pyclass cell's borrow guard.
    fn clock_mut(&mut self) -> &mut Clock {
        &mut self.clock
    }
}

impl Scene for SolarSystemPyScene {
    fn tick_scene(&mut self) -> bool {
        // Fold the script's requested clock edits (recorded by the pymethod
        // setters during the previous egui pass) into the wrapper clock
        // BEFORE ticking - the same frame timing as the Rust sibling, whose
        // Time-panel callbacks write the clock directly at fire time. The
        // requests are drained out of the cell first: the SceneClock setters
        // need `&mut self`, which cannot overlap the cell borrow.
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
        let running = self.tick_clock();
        self.advance(running);
        running
    }

    fn advance(&mut self, _running: bool) {
        // Nothing scene-specific beyond refreshing the Inner's script-facing
        // clock snapshots for the coming get_drawables (any body-key press
        // already landed directly via `request_body(i)`; this frame's
        // `frame_state` folds it into the camera target).
        self.push_clock_snapshots();
    }
}

impl ScenePtzCamera for SolarSystemPyScene {
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

impl CameraView for SolarSystemPyScene {
    fn frame_state(&mut self) -> RenderState {
        // The Rust sibling's frame recipe: the clock and camera rig live on
        // the wrapper itself, so only the script-requested body is read out
        // of the pyclass cell (a transient borrow; nothing here calls into
        // Python). The Rust sibling retargets directly in its key callbacks;
        // here the script can only reach the Inner, so the fold into the
        // wrapper-owned camera target happens one frame later, right here.
        let now = self.clock_now();
        // This frame's celestial sphere, evaluated on the spot (pure
        // function of time, same as the Rust sibling).
        let sphere = CelestialSphere::at(&now);

        // Refresh the wrapper-owned camera target from the Inner's
        // requested body (any script-requested switch already landed via
        // `request_body(i)` during the previous egui pass); a genuine
        // body switch reframes the camera.
        let selected_body = Python::attach(|py| self.inner.borrow(py).selected_body);
        let celestial_to_world = sphere.star_rot_inv.transpose();
        let target = CameraTarget::Body(CelestialBody::ALL[selected_body]);
        if !self.camera_target.same_kind(&target) {
            self.camera.reframe(&target, &sphere, celestial_to_world);
        }
        self.camera_target = target;
        let (eye, look_at, up) = self.camera.world_rig(&target, &sphere, celestial_to_world);

        // No satellites: an empty marker list (see the Rust sibling).
        RenderState {
            time: now,
            camera_target: target,
            camera_pos: eye,
            camera_look_at: look_at,
            camera_up: up,
            markers: Vec::new(),
        }
    }
}

impl UIDrawable for SolarSystemPyScene {
    /// Call the script's `get_drawables(scene)` with the live pyclass handle
    /// and convert its panels; no Rust borrow of the inner cell is held while
    /// the script runs.
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

/// The `scene solar_system_py` CLI arguments. Only the Python-paneled
/// scenes have a script: declaring it here (not on a shared scene arg set)
/// is what lets clap itself require it for this scene and reject it for the
/// others.
#[derive(clap::Args)]
pub struct Args {
    /// Path to the scene's Python panel script, e.g.
    /// `scenes/solar_system_py.py` (read at runtime: edit + relaunch, no
    /// rebuild).
    #[arg(long)]
    pub script: PathBuf,
}

/// Builds the Python-paneled solar-system scene around the `--script` panel
/// script and hands off to the winit event loop. Starts on the default
/// whole-Terra view.
pub fn run(args: Args) {
    // satkit globals first, then the embedded interpreter (inittab before
    // init - see `engine::py`), then the scene, whose construction loads the
    // script.
    scene::init();
    py::init();

    application::run(ApplicationState::new(SolarSystemPyScene::new(args.script)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ui::PanelAnchor;

    /// Executes the real `scenes/solar_system_py.py` end to end: the script
    /// must load, its Camera Target loop must build one key per body, and
    /// the converted panels must match the Rust sibling's shape (Time
    /// top-left, the 10-row Camera Target panel - header + 9 keys -
    /// top-right). No satkit seeding: this path only reads the clock (pure
    /// time math) and the requested body index. The interactive callback
    /// plumbing is proven once, by manual_control_py's round-trip test.
    /// The repo's reference script, resolved against the manifest dir so the
    /// test runs from any cwd - the same file the CLI example passes.
    fn repo_script() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("scenes")
            .join("solar_system_py.py")
    }

    #[test]
    fn solar_system_script_builds_selector() {
        py::init();
        let mut scene = SolarSystemPyScene::new(repo_script());

        let panels = scene.get_drawables();
        assert_eq!(panels.len(), 2, "script must return two panels");
        assert!(matches!(panels[0].anchor, PanelAnchor::TopLeft));
        assert!(matches!(panels[1].anchor, PanelAnchor::TopRight));
        assert_eq!(
            panels[1].rows.len(),
            10,
            "Camera Target panel must hold a header plus one key per body"
        );
        assert!(panels[1].rows.iter().all(|row| row.len() == 1));
    }
}
