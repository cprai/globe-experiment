//! ISS + Hubble scene: track the International Space Station and the Hubble
//! Space Telescope from their shared ~2024-001.5 TLE epoch. This is the
//! original default scene, now expressed as a named scene (CLI:
//! `globe-experiment scene iss_and_hubble`).

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

// This scene's tracked-object TLEs, inlined as source literals. Unlike the
// textures/ephemeris/EOP (build-downloaded straight into `OUT_DIR` and baked
// into the binary), these small element sets live directly in source so a fresh
// checkout needs no data file. The lines are column-sensitive TLE format (each
// element line is exactly 69 chars) - keep the exact spacing. `concat!` keeps
// source indentation out of the string. satkit parses by column and does not
// verify the trailing checksum digit. `new` below assembles the tracked array
// from these via `Satellite::from_tle`.

/// The International Space Station (ISS / ZARYA), epoch 2024-001.5. Real
/// element set.
const ISS_TLE: &str = concat!(
    "ISS (ZARYA)\n",
    "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9003\n",
    "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299357\n",
);

/// The Hubble Space Telescope (HST), epoch ~2024-001.5. NOTE: the orbit shape
/// is realistic (inclination 28.47 deg, ~540 km / ~15.1 rev/day), but this set
/// was assembled from memory - the RAAN/anomaly/epoch-fraction phase is
/// approximate. Replace with a freshly fetched TLE for true positional
/// accuracy. Included as a second tracked object so multiple markers render.
const HST_TLE: &str = concat!(
    "HST\n",
    "1 20580U 90037B   24001.49473380  .00002000  00000-0  10000-3 0  9990\n",
    "2 20580  28.4690  85.5400 0002600 310.0000  50.0000 15.09600000123456\n",
);

/// ISS + Hubble simulation: the clock held directly, plus this scene's two
/// tracked satellites. No celestial sphere is stored - `CelestialSphere::at`
/// is a pure function of time, so `frame_state` evaluates it fresh at each
/// frame's clock instant (the same pattern as the renderer).
pub struct IssAndHubbleScene {
    /// Simulation clock (datetime + play/paused + speed), reached only via
    /// the `SceneClock` API - the Time panel's Run-toggle/speed-slider
    /// callbacks included, which receive the scene at fire time and call the
    /// setters directly.
    clock: Clock,
    satellites: Vec<Satellite>,
    /// The scene's interactive orbital camera (pan/tilt/zoom rig plus its
    /// animations); the default whole-Terra view.
    camera: PtzCamera,
    /// The body the camera orbits - owned by the scene, not the camera, and
    /// passed into every camera call that scales by or centers on it. Fixed
    /// at Terra here (no selector), so it never reframes.
    camera_target: CameraTarget,
}

impl IssAndHubbleScene {
    fn new() -> Self {
        // The clock starts at the first satellite's TLE epoch, so order
        // matters: the primary object (ISS) goes first.
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
    fn advance(&mut self, _running: bool) {
        // Nothing scene-specific: the clock tick lives in `tick_scene` (any
        // Time-panel pause/speed edit already landed via the SceneClock
        // setters during the previous egui pass), and `frame_state`
        // re-derives the celestial sphere at the frame's clock instant.
    }
}

impl ScenePtzCamera for IssAndHubbleScene {
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

impl CameraView for IssAndHubbleScene {
    fn frame_state(&mut self) -> RenderState {
        let now = self.clock_now();
        // This frame's celestial sphere, evaluated on the spot: `at` is a
        // pure function of time, so it needs no stashing between frames (the
        // renderer re-derives the same sphere from `RenderState.time`).
        let sphere = CelestialSphere::at(&now);

        // Resolve the camera first: build the rig against this frame's sphere
        // (Terra's moving center is re-resolved inside `world_rig`; the fixed
        // Terra target never reframes) - the eye feeds the marker-occlusion
        // test below.
        let celestial_to_world = sphere.star_rot_inv.transpose();
        let target = self.camera_target;
        let (eye, look_at, up) = self.camera.world_rig(&target, &sphere, celestial_to_world);

        let mut markers = Vec::with_capacity(self.satellites.len());
        for (i, sat) in self.satellites.iter_mut().enumerate() {
            let state = sat.state_at(&now);
            // The renderer propagates this ahead for the orbit path. The two
            // objects deliberately use different backends - ISS the analytic
            // SGP4 element set, Hubble numerical integration from its current
            // GCRF state vector - demonstrating (and continuously exercising)
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
            // Terra target: the renderer derives the Terra system from the
            // time and keeps the origin at Terra.
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
        // The Time panel first, then this scene's own telemetry panel. Both
        // the panel builder and the readout loop are deliberately kept
        // per-scene (like the propagation loop) - scenes may diverge in what
        // they expose and how.
        //
        // Snapshot the displayed values up front (owned values only) - the
        // panels are owned and never borrow the scene. The two control
        // callbacks receive the scene as `&mut Self` at fire time and call
        // the SceneClock setters directly; each stays idempotent under
        // egui's discard-pass double fire by writing snapshot-derived values
        // (the Run toggle sets the pre-click `running`, never a re-read
        // flip).
        //
        // The readout re-propagates each satellite at the same instant
        // `frame_state` used (`Clock::now()` is pure and nothing ticks the
        // clock between the two calls) and SGP4 is deterministic, so the
        // values match the rendered markers with no stashed state.
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
        let mut rows: Vec<Vec<Box<dyn Instrument<Self>>>> = Vec::with_capacity(telemetry.len() * 3);
        for sat in &telemetry {
            // One header + two readout rows per satellite; taffy stacks the
            // groups (the repeated header rules the panel into sections).
            rows.push(vec![Box::new(Header {
                title: sat.name.clone(),
            })]);
            // Values are padded to their widest form ("-179.99" / "9999.9"):
            // the font is monospace, so fixed-width values keep the digit
            // windows from resizing (and the Lon window from shifting) as the
            // satellites move.
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

/// The `scene iss_and_hubble` CLI arguments - none today. Each scene
/// subcommand declares its own arguments, so a future flag for this scene is
/// added here, not in `main` (which only dispatches).
#[derive(clap::Args)]
pub struct Args {}

/// Builds the ISS + Hubble simulation and hands off to the winit event loop.
/// Blocks until the window closes.
pub fn run(_args: Args) {
    // Seed satkit's global state (embedded ephemeris + EOP table) before
    // anything else: the satellite propagation and the per-frame
    // CelestialSphere evaluation read satkit globals. Doing it here keeps
    // satkit fully offline and data-dir-free.
    scene::init();

    application::run(ApplicationState::new(IssAndHubbleScene::new()));
}
