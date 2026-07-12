//! Lunar-eclipse scene: the 2025-03-14 total lunar eclipse, no tracked
//! objects - Terra's shadow turns Luna blood-red (CLI: `globe-experiment
//! scene lunar_eclipse`).

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

/// Eye distance for the Luna framing (km): ~2 lunar radii above the surface,
/// so the eclipsed disc fills the frame with a little margin (the camera
/// orbits Luna, so the distance is relative to its surface, not Terra's).
const VIEW_DISTANCE_KM: f64 = 3500.0;

/// Speed-slider range: real time to 100x.
const MIN_MULTIPLIER: f32 = 1.0;
const MAX_MULTIPLIER: f32 = 100.0;

/// Empty lunar-eclipse simulation: just the clock; no tracked bodies. A Terra /
/// Luna Camera Target panel writes `camera_target` directly.
#[derive(SceneClock, ScenePtzCamera)]
pub struct LunarEclipseScene {
    clock: Clock,
    /// Written directly by the Camera Target keys via
    /// [`Self::set_camera_target`] (reframes on a genuine switch).
    camera_target: CameraTarget,
    camera: PtzCamera,
}

impl LunarEclipseScene {
    fn new() -> Self {
        // ~30 min before greatest eclipse (06:58 UTC) - the start of totality
        // - so the auto-playing clock runs through the deep umbral phase.
        // Well inside the bundled EOP range (1962-01-01 .. build date).
        let epoch =
            Instant::from_datetime(2025, 3, 14, 6, 28, 0.0).expect("valid lunar-eclipse datetime");
        // Throwaway sphere at the epoch for the launch framing (the clock has
        // not ticked yet); `scene::init` must already have run.
        let celestial_sphere = CelestialSphere::at(&epoch);

        // Looking *toward* Luna puts the eye on its Terra-facing (eclipsed)
        // side, so Terra is behind the camera and never occludes the disc -
        // no limb nudge needed. The sphere is heliocentric, so the direction
        // is Luna's center minus Terra's center, not Luna's raw position.
        let center = celestial_sphere.luna().placement.pos_world
            - celestial_sphere.center_world(CelestialBody::TERRA);
        // Seed camera_target with the body the framing orbits, so the first
        // frame does not reframe.
        let camera_target = CameraTarget::Body(CelestialBody::LUNA);
        let camera = PtzCamera::looking_toward(
            &camera_target,
            celestial_sphere.star_rot_inv,
            center,
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

impl Scene for LunarEclipseScene {
    fn advance(&mut self) {
        // Nothing scene-specific.
    }
}

impl CameraView for LunarEclipseScene {
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
            tracked_bodies: Vec::new(),
        }
    }
}

impl UIDrawable for LunarEclipseScene {
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

/// The `scene lunar_eclipse` CLI arguments - none today.
#[derive(clap::Args)]
pub struct Args {}

/// Builds the lunar-eclipse scene (camera seeded orbiting Luna in `new`) and
/// runs the winit event loop.
pub fn run(_args: Args) {
    scene::init();

    application::run(ApplicationState::new(LunarEclipseScene::new()));
}
