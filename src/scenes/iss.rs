//! ISS tracking scene (CLI: `globe-experiment scene iss`).
//!
//! Follows the International Space Station around Terra from its 2024-001.5
//! TLE epoch, propagated by SGP4, with its dot and predicted orbit path
//! drawn over the globe. The camera orbits Terra (no target selector).
//! Panels: Time (UTC readout, run/pause, 1x-100x speed slider) top-left;
//! live ISS latitude / longitude / altitude top-right.

use engine::application::{self, ApplicationState};
use engine::camera::{PtzCamera, ScenePtzCamera};
use engine::scene::orbital_body::OrbitalBody;
use engine::scene::{
    self, BodyTelemetry, CameraTarget, Clock, Scene, SceneClock, SceneKinematicBodies,
    SceneOrbitalBodies,
};
use engine::ui::{
    DualReadout, Header, Instrument, InteractiveSlider, InteractiveToggle, PanelAnchor, Readout,
    Slider, Toggle, UIDrawable, UIDrawablePanel,
};

/// The International Space Station (ISS / ZARYA), epoch 2024-001.5. Real
/// element set. Column-sensitive format: each element line is exactly 69
/// chars - keep the exact spacing (satkit parses by column). Deliberately
/// duplicated per scene - do not factor into a shared const.
const ISS_TLE: &str = concat!(
    "ISS (ZARYA)\n",
    "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9003\n",
    "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299357\n",
);

/// Speed-slider range: real time to 100x.
const MIN_MULTIPLIER: f32 = 1.0;
const MAX_MULTIPLIER: f32 = 100.0;

#[derive(SceneClock, ScenePtzCamera, SceneOrbitalBodies, SceneKinematicBodies)]
pub struct IssScene {
    clock: Clock,
    camera: PtzCamera,
    camera_target: CameraTarget,
    orbital_bodies: Vec<OrbitalBody>,
}

impl IssScene {
    fn new() -> Self {
        let iss = OrbitalBody::from_tle(ISS_TLE);
        let epoch = iss.epoch();
        Self {
            clock: Clock::new(epoch),
            camera: PtzCamera::default(),
            camera_target: CameraTarget::terra(),
            orbital_bodies: vec![iss],
        }
    }
}

impl Scene for IssScene {
    fn advance(&mut self) {
        // Nothing scene-specific.
    }
}

impl UIDrawable for IssScene {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<Self>> {
        // Snapshot displayed values up front - the panels are owned and never
        // borrow the scene. Re-propagating at the frame's clock instant is
        // deterministic, so readouts match the rendered dots.
        let now = self.clock_now();
        let telemetry: Vec<BodyTelemetry> = self
            .orbital_bodies
            .iter_mut()
            .map(|body| {
                let state = body.state_at(&now);
                BodyTelemetry {
                    name: body.name.clone(),
                    latitude_deg: state.latitude_deg as f32,
                    longitude_deg: state.longitude_deg as f32,
                    altitude_km: state.altitude_km as f32,
                }
            })
            .collect();

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
        let mut rows: Vec<Vec<Box<dyn Instrument<Self>>>> = Vec::with_capacity(telemetry.len() * 3);
        for body in &telemetry {
            rows.push(vec![Box::new(Header {
                title: body.name.clone(),
            })]);
            // Padded to the widest form ("-179.99" / "9999.9"; monospace) so
            // the digit windows keep their size as the body moves.
            rows.push(vec![Box::new(DualReadout {
                left_label: "Lat".to_string(),
                left_value: format!("{:>7.2}", body.latitude_deg),
                left_unit: "deg".to_string(),
                right_label: "Lon".to_string(),
                right_value: format!("{:>7.2}", body.longitude_deg),
                right_unit: "deg".to_string(),
            })]);
            rows.push(vec![Box::new(Readout {
                label: "Alt".to_string(),
                value: format!("{:>6.1}", body.altitude_km),
                unit: "km".to_string(),
            })]);
        }
        panels.push(UIDrawablePanel {
            anchor: PanelAnchor::TopRight,
            rows,
        });
        panels
    }
}

/// The `scene iss` CLI arguments - none today.
#[derive(clap::Args)]
pub struct Args {}

pub fn run(_args: Args) {
    // Seed satkit's globals (embedded ephemeris + EOP) before any
    // propagation or CelestialSphere use.
    scene::init();

    application::run(ApplicationState::new(IssScene::new()));
}
