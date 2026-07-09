//! The solar-system scene with its UI panels produced by **Python**: a clone
//! of `solar_system` (same nine-body selector, camera, and clock) whose
//! `UIDrawable::get_drawables` delegates to `scenes/solar_system_py.py`. The
//! two scenes live side by side so the Rust and Python panel APIs can be
//! compared (CLI: `globe-experiment scene solar_system_py`).
//!
//! Same structure as `manual_control_py`: the scene state is a `#[pyclass]`
//! ([`SolarSystemSceneInner`]) handed live to the script, wrapped by
//! [`SolarSystemPyScene`] for the four engine traits; borrows of the pyclass
//! cell never span a call into Python. The script sees the shared `clock`
//! and `selector` handles - its selector panel reads `selector.selected` /
//! `BodySelector.body_names()` and its key callbacks call
//! `selector.request(i)`, the index twin of the Rust panel's disjoint
//! request flags. Script errors fail fast (traceback + panic); callback
//! errors only print (see `ui::py`).

use pyo3::prelude::*;
use satkit::Instant;

use crate::engine::application::{self, ApplicationState};
use crate::engine::camera::{
    CameraControl, CameraView, CursorHint, PointerButton, PtzCamera, ScrollDelta,
};
use crate::engine::py;
use crate::engine::scene::celestial_sphere::CelestialSphere;
use crate::engine::scene::{self, BodySelector, CameraTarget, Clock, RenderState, Scene};
use crate::engine::ui::{self, UIDrawable, UIDrawablePanel};

/// The scene's script, resolved under the repo-root `scenes/` directory.
const SCRIPT_FILE: &str = "solar_system_py.py";

/// The live scene state, as a pyclass: the script reads/drives the shared
/// `clock` and `selector`; the camera and its target stay Rust-private.
#[pyclass(name = "SolarSystemScene", module = "globe")]
pub struct SolarSystemSceneInner {
    /// Simulation clock (datetime + play/paused + speed), shared with the
    /// script. The getter hands Python the same live object.
    #[pyo3(get)]
    clock: Py<Clock>,
    /// The nine-body camera-target selector, shared with the script (which
    /// rebuilds its panel in Python and requests switches through it).
    #[pyo3(get)]
    selector: Py<BodySelector>,
    /// The scene's interactive orbital camera (pan/tilt/zoom rig plus its
    /// animations); starts on the default whole-Terra view.
    camera: PtzCamera,
    /// The body the camera orbits this frame - refreshed from the selector
    /// each frame; a genuine switch reframes the camera.
    camera_target: CameraTarget,
}

/// The Rust-only half: construction and the per-frame work, mirroring the
/// Rust sibling body for body (only the clock/selector accesses differ -
/// `borrow` through the shared pyclass cells).
impl SolarSystemSceneInner {
    fn new(py: Python<'_>) -> Self {
        // A fixed recent past date, well inside the bundled EOP range (see
        // the Rust sibling).
        let epoch =
            Instant::from_datetime(2025, 6, 1, 0, 0, 0.0).expect("valid solar-system datetime");
        Self {
            clock: Py::new(py, Clock::new(epoch)).expect("clock pyclass"),
            selector: Py::new(py, BodySelector::default()).expect("selector pyclass"),
            camera: PtzCamera::default(),
            // Matches the selector default (Terra) and the whole-Terra camera
            // above, so the first frame does not reframe.
            camera_target: CameraTarget::terra(),
        }
    }

    fn advance(&mut self, py: Python<'_>) -> bool {
        // Fold in any pending body-key request before the camera target is
        // read (the script's key callbacks called `selector.request(i)`
        // during the previous egui pass).
        self.selector.borrow_mut(py).apply_requests();
        self.clock.borrow_mut(py).tick()
    }

    fn frame_state(&mut self, py: Python<'_>) -> RenderState {
        let now = self.clock.borrow(py).now();
        // This frame's celestial sphere, evaluated on the spot (pure
        // function of time, same as the Rust sibling).
        let sphere = CelestialSphere::at(&now);

        // Refresh the scene-owned camera target from the selector; a genuine
        // body switch reframes the camera.
        let celestial_to_world = sphere.star_rot_inv.transpose();
        let target = self.selector.borrow(py).resolve();
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

/// What the application owns: the `Py` handle to the scene pyclass plus the
/// script's `get_drawables` function (loaded once at startup).
pub struct SolarSystemPyScene {
    inner: Py<SolarSystemSceneInner>,
    get_drawables_fn: Py<PyAny>,
}

impl SolarSystemPyScene {
    fn new() -> Self {
        Python::attach(|py| Self {
            inner: Py::new(py, SolarSystemSceneInner::new(py)).expect("scene pyclass"),
            get_drawables_fn: py::load_get_drawables(py, SCRIPT_FILE),
        })
    }
}

impl Scene for SolarSystemPyScene {
    fn advance(&mut self) -> bool {
        Python::attach(|py| self.inner.borrow_mut(py).advance(py))
    }
}

impl CameraControl for SolarSystemPyScene {
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

impl CameraView for SolarSystemPyScene {
    fn frame_state(&mut self) -> RenderState {
        Python::attach(|py| self.inner.borrow_mut(py).frame_state(py))
    }
}

impl UIDrawable for SolarSystemPyScene {
    /// Call the script's `get_drawables(scene)` with the live pyclass handle
    /// and convert its panels; no Rust borrow of the inner cell is held while
    /// the script runs.
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        Python::attach(|py| {
            self.get_drawables_fn
                .bind(py)
                .call1((self.inner.bind(py),))
                .and_then(|panels| ui::py::panels_from_python(py, &panels))
                .unwrap_or_else(|err| {
                    err.print(py);
                    panic!("scenes/{SCRIPT_FILE}: get_drawables failed");
                })
        })
    }
}

/// Builds the Python-paneled solar-system scene and hands off to the winit
/// event loop. Starts on the default whole-Terra view.
pub fn run() {
    // satkit globals first, then the embedded interpreter (inittab before
    // init - see `engine::py`), then the scene, whose construction loads the
    // script.
    scene::init();
    py::init();

    application::run(ApplicationState::new(SolarSystemPyScene::new()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ui::PanelAnchor;

    /// Executes the real `scenes/solar_system_py.py` end to end: the script
    /// must load, its selector loop must build one key per selectable body,
    /// and the converted panels must match the Rust sibling's shape (Time
    /// top-left, the 10-row selector - header + 9 keys - top-right). No
    /// satkit seeding: this path only reads the clock (pure time math) and
    /// the selector. The interactive callback plumbing is proven once, by
    /// manual_control_py's round-trip test.
    #[test]
    fn solar_system_script_builds_selector() {
        py::init();
        let mut scene = SolarSystemPyScene::new();

        let panels = scene.get_drawables();
        assert_eq!(panels.len(), 2, "script must return two panels");
        assert!(matches!(panels[0].anchor, PanelAnchor::TopLeft));
        assert!(matches!(panels[1].anchor, PanelAnchor::TopRight));
        assert_eq!(
            panels[1].rows.len(),
            10,
            "selector panel must hold a header plus one key per body"
        );
        assert!(panels[1].rows.iter().all(|row| row.len() == 1));
    }
}
