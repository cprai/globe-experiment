//! Solar-eclipse scene: the 2024-04-08 total solar eclipse, no tracked
//! objects - Luna's shadow sweeps the daylit Terra (CLI: `globe-experiment
//! scene solar_eclipse`).

use satkit::Instant;

use crate::engine::application::{self, ApplicationState};
use crate::engine::camera::{PtzCamera, ScenePtzCamera};
use crate::engine::scene::celestial_sphere::CelestialSphere;
use crate::engine::scene::{
    self, CameraTarget, CelestialBody, Clock, Scene, SceneClock, SceneKinematicBodies,
    SceneOrbitalBodies,
};
use crate::engine::ui::{
    Header, Instrument, InteractiveSlider, InteractiveToggle, PanelAnchor, Readout, Slider, Toggle,
    UIDrawable, UIDrawablePanel,
};

/// Eye distance for the day-side framing (km): Terra fills most of the
/// frame with Luna's umbral shadow spot centered near the subsolar point.
const VIEW_DISTANCE_KM: f64 = 22000.0;

/// Speed-slider range: real time to 100x.
const MIN_MULTIPLIER: f32 = 1.0;
const MAX_MULTIPLIER: f32 = 100.0;

/// Empty solar-eclipse simulation: just the clock; no tracked bodies. A Terra /
/// Luna Camera Target panel writes `camera_target` directly.
#[derive(SceneClock, ScenePtzCamera, SceneOrbitalBodies, SceneKinematicBodies)]
pub struct SolarEclipseScene {
    clock: Clock,
    /// Written directly by the Camera Target keys via
    /// [`Self::set_camera_target`] (reframes on a genuine switch).
    camera_target: CameraTarget,
    camera: PtzCamera,
}

impl SolarEclipseScene {
    fn new() -> Self {
        // ~30 min before greatest eclipse (18:17 UTC), so the auto-playing
        // clock runs through the umbra's crossing of North America. Well
        // inside the bundled EOP range (1962-01-01 .. build date).
        let epoch =
            Instant::from_datetime(2024, 4, 8, 17, 47, 0.0).expect("valid solar-eclipse datetime");
        // Throwaway sphere at the epoch for the launch framing (the clock has
        // not ticked yet); `scene::init` must already have run.
        let celestial_sphere = CelestialSphere::at(&epoch);

        // Frame the sunlit face by looking toward Sol. The sphere is
        // heliocentric, so the Terra->Sol direction is Sol's position minus
        // Terra's center (not just the Sol position).
        let terra_to_sol =
            celestial_sphere.sol_pos_world - celestial_sphere.center_world(CelestialBody::TERRA);
        // Seed camera_target with the body the framing orbits, so the first
        // frame does not reframe.
        let camera_target = CameraTarget::terra();
        let camera = PtzCamera::looking_toward(
            &camera_target,
            celestial_sphere.star_rot_inv,
            -terra_to_sol.normalize(),
            VIEW_DISTANCE_KM,
        );

        Self {
            clock: Clock::new(epoch),
            camera_target,
            camera,
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

impl Scene for SolarEclipseScene {
    fn advance(&mut self) {
        // Nothing scene-specific.
    }
}

impl UIDrawable for SolarEclipseScene {
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

        // Terra / Luna Camera Target keys: each writes its fixed body through
        // `set_camera_target` (no-op on re-select - idempotent); the scene
        // holds no selection state beyond `camera_target` itself.
        let luna_active = self
            .camera_target
            .same_kind(&CameraTarget::Body(CelestialBody::LUNA));
        let target_rows: Vec<Vec<Box<dyn Instrument<Self>>>> = vec![
            vec![Box::new(Header {
                title: "Camera Target".to_string(),
            })],
            vec![
                Box::new(InteractiveToggle {
                    toggle: Toggle {
                        label: "Terra".to_string(),
                        active: !luna_active,
                    },
                    on_toggle: Box::new(|scene: &mut Self| {
                        scene.set_camera_target(CameraTarget::terra())
                    }),
                }),
                Box::new(InteractiveToggle {
                    toggle: Toggle {
                        label: "Luna".to_string(),
                        active: luna_active,
                    },
                    on_toggle: Box::new(|scene: &mut Self| {
                        scene.set_camera_target(CameraTarget::Body(CelestialBody::LUNA))
                    }),
                }),
            ],
        ];
        panels.push(UIDrawablePanel {
            anchor: PanelAnchor::TopRight,
            rows: target_rows,
        });
        panels
    }
}

/// The `scene solar_eclipse` CLI arguments - none today.
#[derive(clap::Args)]
pub struct Args {}

/// Builds the solar-eclipse scene (camera seeded on the daylit face in
/// `new`) and runs the winit event loop.
pub fn run(_args: Args) {
    // Seed satkit's globals before the celestial sphere is built in `new`.
    scene::init();

    application::run(ApplicationState::new(SolarEclipseScene::new()));
}
