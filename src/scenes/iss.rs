//! ISS-only scene: track the International Space Station from its
//! ~2024-001.5 TLE epoch (CLI: `globe-experiment scene iss`).

use crate::engine::application::{self, ApplicationState};
use crate::engine::camera::{CameraView, PtzCamera, ScenePtzCamera};
use crate::engine::scene::celestial_sphere::CelestialSphere;
use crate::engine::scene::satellite::{Propagation, Satellite};
use crate::engine::scene::{
    self, CameraTarget, Clock, RenderState, SatelliteMarker, SatelliteTelemetry, Scene, SceneClock,
    marker_occluded,
};
use crate::engine::ui::{
    DualReadout, Header, Instrument, InteractiveSlider, InteractiveToggle, PanelAnchor, Readout,
    Slider, Toggle, UIDrawable, UIDrawablePanel,
};

// Column-sensitive TLE format: each element line is exactly 69 chars - keep
// the exact spacing (satkit parses by column; the trailing checksum digit is
// unverified). Deliberately duplicated per scene - do not factor into a
// shared const.

/// The International Space Station (ISS / ZARYA), epoch 2024-001.5. Real
/// element set.
const ISS_TLE: &str = concat!(
    "ISS (ZARYA)\n",
    "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9003\n",
    "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299357\n",
);

/// Speed-slider range: real time to 100x.
const MIN_MULTIPLIER: f32 = 1.0;
const MAX_MULTIPLIER: f32 = 100.0;

/// ISS-only simulation: clock plus a single tracked satellite.
#[derive(SceneClock, ScenePtzCamera)]
pub struct IssScene {
    clock: Clock,
    satellites: Vec<Satellite>,
    camera: PtzCamera,
    /// Fixed at Terra (no selector), so it never reframes.
    camera_target: CameraTarget,
}

impl IssScene {
    fn new() -> Self {
        let satellites = vec![Satellite::from_tle(ISS_TLE)];
        let epoch = satellites.first().expect("TLE present").epoch();
        Self {
            clock: Clock::new(epoch),
            satellites,
            camera: PtzCamera::default(),
            camera_target: CameraTarget::terra(),
        }
    }
}

impl Scene for IssScene {
    fn advance(&mut self) {
        // Nothing scene-specific.
    }
}

impl CameraView for IssScene {
    fn frame_state(&mut self) -> RenderState {
        let now = self.clock_now();
        let sphere = CelestialSphere::at(&now);

        let celestial_to_world = sphere.star_rot_inv.transpose();
        let target = self.camera_target;
        let (eye, look_at, up) = self.camera.world_rig(&target, &sphere, celestial_to_world);

        let mut markers = Vec::with_capacity(self.satellites.len());
        for sat in &mut self.satellites {
            let state = sat.state_at(&now);
            markers.push(SatelliteMarker {
                position_km: state.position_km,
                // Terra target, so the render-frame eye is the absolute eye.
                visible: !marker_occluded(eye, state.position_km),
                propagation: Propagation::Sgp4(Box::new(sat.tle().clone())),
            });
        }

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

impl UIDrawable for IssScene {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<Self>> {
        // Snapshot displayed values up front - the panels are owned and never
        // borrow the scene. Re-propagating at the frame's clock instant is
        // deterministic, so readouts match the rendered markers with no
        // stashed state.
        let now = self.clock_now();
        let telemetry: Vec<SatelliteTelemetry> = self
            .satellites
            .iter_mut()
            .map(|sat| {
                let state = sat.state_at(&now);
                SatelliteTelemetry {
                    name: sat.name.clone(),
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
        for sat in &telemetry {
            rows.push(vec![Box::new(Header {
                title: sat.name.clone(),
            })]);
            // Padded to the widest form ("-179.99" / "9999.9"; monospace) so
            // the digit windows keep their size as the satellite moves.
            rows.push(vec![Box::new(DualReadout {
                left_label: "Lat".to_string(),
                left_value: format!("{:>7.2}", sat.latitude_deg),
                left_unit: "deg".to_string(),
                right_label: "Lon".to_string(),
                right_value: format!("{:>7.2}", sat.longitude_deg),
                right_unit: "deg".to_string(),
            })]);
            rows.push(vec![Box::new(Readout {
                label: "Alt".to_string(),
                value: format!("{:>6.1}", sat.altitude_km),
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

/// Builds the ISS simulation and runs the winit event loop until close.
pub fn run(_args: Args) {
    // Seed satkit's globals (embedded ephemeris + EOP) before any
    // propagation or CelestialSphere use.
    scene::init();

    application::run(ApplicationState::new(IssScene::new()));
}
