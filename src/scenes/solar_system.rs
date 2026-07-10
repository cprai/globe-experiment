//! Solar-system scene: a free tour of the whole solar system with no tracked
//! objects - the celestial sphere wound to a fixed past date, and a Camera
//! Target panel (one latching key per body) that flies the camera to and
//! orbits any of Terra, Luna, or the seven planets (CLI: `globe-experiment
//! scene solar_system`).
//! Like the eclipse scenes it carries no `Satellite` list and draws no
//! markers; unlike them it draws all seven planets, each at its true
//! geocentric position and scale.
//!
//! Because the outer planets sit billions of km from Terra - far past f32
//! precision in world-km - a planet target renders with a floating origin (the
//! scene is drawn relative to the orbited planet's center; see
//! `CameraTarget::render_origin`). Terra/Luna targets keep the origin at Terra.

use satkit::Instant;

use crate::engine::application::{self, ApplicationState};
use crate::engine::camera::{CameraView, PtzCamera, ScenePtzCamera};
use crate::engine::scene::celestial_sphere::CelestialSphere;
use crate::engine::scene::{
    self, CameraTarget, CelestialBody, Clock, RenderState, Scene, SceneClock,
};
use crate::engine::ui::{
    Header, Instrument, InteractiveSlider, InteractiveToggle, PanelAnchor, Readout, Slider, Toggle,
    UIDrawable, UIDrawablePanel,
};

/// Empty solar-system simulation: just the clock. Its one-key-per-body
/// Camera Target panel flies the camera to any of the nine bodies by writing
/// `camera_target` directly. No satellites, and no celestial sphere stored -
/// `CelestialSphere::at` is a pure function of time, so `frame_state`
/// evaluates it fresh at each frame's clock instant (the same pattern as the
/// renderer).
pub struct SolarSystemScene {
    /// Simulation clock (datetime + play/paused + speed), reached only via
    /// the `SceneClock` API - the Time panel's Run-toggle/speed-slider
    /// callbacks included, which receive the scene at fire time and call the
    /// setters directly.
    clock: Clock,
    /// The body the camera orbits - owned by the scene, not the camera, and
    /// passed into every camera call that scales by or centers on it.
    /// Written directly by the Camera Target panel's key callbacks (via
    /// [`Self::set_camera_target`], which reframes the camera on a genuine
    /// body switch).
    camera_target: CameraTarget,
    /// The scene's interactive orbital camera (pan/tilt/zoom rig plus its
    /// animations); starts on the default whole-Terra view.
    camera: PtzCamera,
}

impl SolarSystemScene {
    fn new() -> Self {
        // A fixed recent past date, well inside the bundled EOP range
        // (1962-01-01 .. build date), so every body's position is accurate. The
        // clock auto-plays from here; the planets and their phases evolve.
        let epoch =
            Instant::from_datetime(2025, 6, 1, 0, 0, 0.0).expect("valid solar-system datetime");
        Self {
            clock: Clock::new(epoch),
            // Start on Terra (the familiar default view), matching the
            // whole-Terra camera below.
            camera_target: CameraTarget::terra(),
            camera: PtzCamera::default(),
        }
    }

    /// Switches the orbited body - the Camera Target panel keys' fire-time
    /// callback. A genuine switch reframes the camera on the spot
    /// (full-frame distance, re-aim, in-flight animations dropped) against
    /// the sphere at the current clock instant; re-selecting the already
    /// orbited body is a no-op, which also keeps the callback idempotent
    /// under egui's discard-pass double fire.
    fn set_camera_target(&mut self, target: CameraTarget) {
        if self.camera_target.same_kind(&target) {
            return;
        }
        let sphere = CelestialSphere::at(&self.clock_now());
        self.camera
            .reframe(&target, &sphere, sphere.star_rot_inv.transpose());
        self.camera_target = target;
    }
}

impl SceneClock for SolarSystemScene {
    fn clock_mut(&mut self) -> &mut Clock {
        &mut self.clock
    }
}

impl Scene for SolarSystemScene {
    fn advance(&mut self, _running: bool) {
        // Nothing scene-specific: the clock tick lives in `tick_scene` (any
        // body-key press or Time-panel edit already landed directly during
        // the previous egui pass), and `frame_state` re-derives the celestial
        // sphere at the frame's clock instant.
    }
}

impl ScenePtzCamera for SolarSystemScene {
    // The accessors behind the blanket `CameraControl` impl, which forwards
    // every input event into the embedded camera: where the camera and the
    // scene-owned orbit target live in this struct. A scene that needs to
    // diverge (gate input, swap the camera kind) implements `CameraControl`
    // directly instead.
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

impl CameraView for SolarSystemScene {
    fn frame_state(&mut self) -> RenderState {
        let now = self.clock_now();
        // This frame's celestial sphere, evaluated on the spot: `at` is a
        // pure function of time, so it needs no stashing between frames (the
        // renderer re-derives the same sphere from `RenderState.time`).
        let sphere = CelestialSphere::at(&now);

        // Resolve the inertial rig into the render frame (the orbited body's
        // moving center is re-resolved from the ephemeris inside
        // `world_rig`). Any body switch already landed - and reframed the
        // camera - directly in the Camera Target key's callback
        // (`set_camera_target`) during the previous egui pass, so the target
        // here is simply the scene-owned one.
        let celestial_to_world = sphere.star_rot_inv.transpose();
        let target = self.camera_target;
        let (eye, look_at, up) = self.camera.world_rig(&target, &sphere, celestial_to_world);

        // No satellites: an empty marker list. The renderer derives every
        // body's position from the frame's time and uses the camera target's
        // render origin (which must match the one the camera built its rig
        // against - both are the single `target` resolved above).
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

impl UIDrawable for SolarSystemScene {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<Self>> {
        // The Time panel (datetime + run/speed) plus the one-key-per-body
        // Camera Target panel. The panel builders are deliberately kept
        // per-scene - scenes may diverge in what they expose.
        //
        // Snapshot the displayed values up front (owned `String`/`f32`/`bool`)
        // - the panels are owned and never borrow the scene. The control
        // callbacks receive the scene as `&mut Self` at fire time and call
        // the SceneClock setters directly; each stays idempotent under
        // egui's discard-pass double fire by writing snapshot-derived values
        // (the Run toggle sets the pre-click `running`, never a re-read
        // flip).
        let datetime = self.clock_datetime_label();
        // Padded to the widest value (MAX_MULTIPLIER "100.0" = 5 chars): the
        // font is monospace, so a fixed-width value keeps the digit window
        // from resizing as the speed changes.
        let speed = format!("{:>5.1}", self.clock_multiplier());
        let running = !self.clock_paused();

        // Exponential (base e) speed: the slider edits the exponent, so
        // multiplier = e^exp - real time (e^0 = 1x) at the left, 100x at the
        // right, 10x at the midpoint. The mapping lives here, not in the panel.
        let speed_exp = self.clock_multiplier().ln();
        let exp_range = Clock::MIN_MULTIPLIER.ln()..=Clock::MAX_MULTIPLIER.ln();

        // The producer groups instruments into rows + picks content only; all
        // styling and every metric live in the instrument modules / theme
        // (taffy bottom-aligns the Run key with the speed window beside it).
        let time_rows: Vec<Vec<Box<dyn Instrument<Self>>>> = vec![
            vec![Box::new(Header {
                title: "Time".to_string(),
            })],
            vec![Box::new(Readout {
                label: "UTC".to_string(),
                value: datetime,
                unit: String::new(),
            })],
            vec![
                Box::new(Readout {
                    label: "Speed".to_string(),
                    value: speed,
                    unit: "x".to_string(),
                }),
                Box::new(InteractiveToggle {
                    toggle: Toggle {
                        label: "Run".to_string(),
                        active: running,
                    },
                    // Pausing = setting the snapshotted pre-click `running`.
                    on_toggle: Box::new(move |scene: &mut Self| scene.set_clock_paused(running)),
                }),
            ],
            vec![Box::new(InteractiveSlider {
                slider: Slider {
                    value: speed_exp,
                    range: exp_range,
                },
                on_change: Box::new(|scene: &mut Self, exp| scene.set_clock_multiplier(exp.exp())),
            })],
        ];
        let mut panels = vec![UIDrawablePanel {
            anchor: PanelAnchor::TopLeft,
            rows: time_rows,
        }];

        // The Camera Target panel: a header plus one latching key per body,
        // a single column in `CelestialBody::ALL` order (distance from Sol,
        // the orbited body lit; each lone key fills its row). Each key's
        // callback receives the scene at fire time and writes its own fixed
        // body into the camera target directly through `set_camera_target`
        // (which reframes on a genuine switch and no-ops on the
        // already-orbited body - idempotent) - the scene holds no selection
        // state beyond `camera_target` itself.
        let mut target_rows: Vec<Vec<Box<dyn Instrument<Self>>>> = vec![vec![Box::new(Header {
            title: "Camera Target".to_string(),
        })]];
        for body in CelestialBody::ALL {
            let target = CameraTarget::Body(body);
            target_rows.push(vec![Box::new(InteractiveToggle {
                toggle: Toggle {
                    label: body.name().to_string(),
                    active: self.camera_target.same_kind(&target),
                },
                on_toggle: Box::new(move |scene: &mut Self| scene.set_camera_target(target)),
            })]);
        }
        panels.push(UIDrawablePanel {
            anchor: PanelAnchor::TopRight,
            rows: target_rows,
        });
        panels
    }
}

/// The `scene solar_system` CLI arguments - none today. Each scene
/// subcommand declares its own arguments, so a future flag for this scene is
/// added here, not in `main` (which only dispatches).
#[derive(clap::Args)]
pub struct Args {}

/// Builds the solar-system scene and hands off to the winit event loop. Starts
/// on the default whole-Terra view; the Camera Target keys then tour the
/// system.
pub fn run(_args: Args) {
    scene::init();
    application::run(ApplicationState::new(SolarSystemScene::new()));
}
