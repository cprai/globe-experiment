//! ISS + Hubble scene: track both from their shared ~2024-001.5 TLE epoch.
//! The original default scene (CLI: `globe-experiment scene iss_and_hubble`).

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

/// Hubble (HST), epoch ~2024-001.5. Orbit shape is realistic but this set was
/// assembled from memory - RAAN/anomaly/epoch-fraction phase is approximate;
/// replace with a freshly fetched TLE for true positional accuracy.
const HST_TLE: &str = concat!(
    "HST\n",
    "1 20580U 90037B   24001.49473380  .00002000  00000-0  10000-3 0  9990\n",
    "2 20580  28.4690  85.5400 0002600 310.0000  50.0000 15.09600000123456\n",
);

/// ISS + Hubble simulation: clock plus two tracked satellites.
pub struct IssAndHubbleScene {
    clock: Clock,
    satellites: Vec<Satellite>,
    camera: PtzCamera,
    /// Fixed at Terra (no selector), so it never reframes.
    camera_target: CameraTarget,
}

impl IssAndHubbleScene {
    fn new() -> Self {
        // The clock starts at the FIRST satellite's TLE epoch - the primary
        // object (ISS) must go first.
        let satellites = vec![Satellite::from_tle(ISS_TLE), Satellite::from_tle(HST_TLE)];
        let epoch = satellites.first().expect("TLE present").epoch();
        Self {
            clock: Clock::new(epoch),
            satellites,
            camera: PtzCamera::default(),
            camera_target: CameraTarget::terra(),
        }
    }
}

impl SceneClock for IssAndHubbleScene {
    fn clock_mut(&mut self) -> &mut Clock {
        &mut self.clock
    }
}

impl Scene for IssAndHubbleScene {
    fn advance(&mut self) {
        // Nothing scene-specific.
    }
}

impl ScenePtzCamera for IssAndHubbleScene {
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

impl CameraView for IssAndHubbleScene {
    fn frame_state(&mut self) -> RenderState {
        let now = self.clock_now();
        let sphere = CelestialSphere::at(&now);

        let celestial_to_world = sphere.star_rot_inv.transpose();
        let target = self.camera_target;
        let (eye, look_at, up) = self.camera.world_rig(&target, &sphere, celestial_to_world);

        let mut markers = Vec::with_capacity(self.satellites.len());
        for (i, sat) in self.satellites.iter_mut().enumerate() {
            let state = sat.state_at(&now);
            // Deliberately mixed backends - ISS analytic SGP4, Hubble
            // numerical from its current GCRF state - continuously exercising
            // the mixed-propagation capability in one scene.
            let propagation = if i == 0 {
                Propagation::Sgp4(Box::new(sat.tle().clone()))
            } else {
                Propagation::Numerical(state.orbit)
            };
            markers.push(SatelliteMarker {
                position_km: state.position_km,
                // Terra target, so the render-frame eye is the absolute eye.
                visible: !marker_occluded(eye, state.position_km),
                propagation,
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

impl UIDrawable for IssAndHubbleScene {
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
        let exp_range = Clock::MIN_MULTIPLIER.ln()..=Clock::MAX_MULTIPLIER.ln();

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
            // the digit windows keep their size as the satellites move.
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

/// The `scene iss_and_hubble` CLI arguments - none today.
#[derive(clap::Args)]
pub struct Args {}

/// Builds the ISS + Hubble simulation and runs the winit event loop until
/// close.
pub fn run(_args: Args) {
    // Seed satkit's globals (embedded ephemeris + EOP) before any
    // propagation or CelestialSphere use.
    scene::init();

    application::run(ApplicationState::new(IssAndHubbleScene::new()));
}
