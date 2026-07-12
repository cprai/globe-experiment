//! Solar-system scene: a free tour with no tracked objects - a one-key-per-
//! body Camera Target panel orbits any of the nine bodies at true position
//! and scale (CLI: `globe-experiment scene solar_system`).
//!
//! The outer planets sit billions of km out, past f32 precision in world-km,
//! so a planet target renders with a floating origin (drawn relative to the
//! orbited body's center; see `CameraTarget::render_origin`).

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

/// Speed-slider range: real time to 100x.
const MIN_MULTIPLIER: f32 = 1.0;
const MAX_MULTIPLIER: f32 = 100.0;

/// Empty solar-system simulation: just the clock; no satellites. A
/// one-key-per-body Camera Target panel writes `camera_target` directly.
#[derive(SceneClock, ScenePtzCamera)]
pub struct SolarSystemScene {
    clock: Clock,
    /// Written directly by the Camera Target keys via
    /// [`Self::set_camera_target`] (reframes on a genuine switch).
    camera_target: CameraTarget,
    camera: PtzCamera,
}

impl SolarSystemScene {
    fn new() -> Self {
        // A fixed recent past date, well inside the bundled EOP range
        // (1962-01-01 .. build date), so every body's position is accurate.
        let epoch =
            Instant::from_datetime(2025, 6, 1, 0, 0, 0.0).expect("valid solar-system datetime");
        Self {
            clock: Clock::new(epoch),
            camera_target: CameraTarget::terra(),
            camera: PtzCamera::default(),
        }
    }

    /// Switches the orbited body - the Camera Target keys' fire-time
    /// callback. A genuine switch reframes the camera; re-selecting the
    /// orbited body no-ops, keeping the callback idempotent under egui's
    /// discard-pass double fire.
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

impl Scene for SolarSystemScene {
    fn advance(&mut self) {
        // Nothing scene-specific.
    }
}

impl CameraView for SolarSystemScene {
    fn frame_state(&mut self) -> RenderState {
        let now = self.clock_now();
        let sphere = CelestialSphere::at(&now);

        let celestial_to_world = sphere.star_rot_inv.transpose();
        let target = self.camera_target;
        let (eye, look_at, up) = self.camera.world_rig(&target, &sphere, celestial_to_world);

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
        // Snapshot displayed values up front - the panels are owned and
        // never borrow the scene.
        let datetime = self.clock_datetime_label();
        // Padded to the widest value (monospace font) so the digit window
        // does not resize as the speed changes.
        let speed = format!("{:>5.1}", self.clock_multiplier());
        let running = !self.clock_paused();

        // The slider edits the exponent: multiplier = e^exp, so 1x at the
        // left, 100x at the right, 10x at the midpoint.
        let speed_exp = self.clock_multiplier().ln();
        let exp_range = MIN_MULTIPLIER.ln()..=MAX_MULTIPLIER.ln();

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
                    // Writes the pre-click snapshot - keep idempotent: egui's
                    // discard pass can fire this twice per frame.
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

        // One latching key per body in `CelestialBody::ALL` order (distance
        // from Sol). Each key writes its fixed body through
        // `set_camera_target` (no-op on re-select - idempotent); the scene
        // holds no selection state beyond `camera_target` itself.
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

/// The `scene solar_system` CLI arguments - none today.
#[derive(clap::Args)]
pub struct Args {}

/// Builds the solar-system scene and runs the winit event loop. Starts on
/// the default whole-Terra view.
pub fn run(_args: Args) {
    scene::init();
    application::run(ApplicationState::new(SolarSystemScene::new()));
}
