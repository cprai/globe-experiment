//! ISS-only scene: track the International Space Station from its
//! ~2024-001.5 TLE epoch. Same as `iss_and_hubble` but with Hubble omitted, so
//! a single marker renders (CLI: `globe-experiment scene iss`).

use crate::engine::application::{self, ApplicationState};
use crate::engine::camera::{
    CameraControl, CameraView, CursorHint, PointerButton, PtzCamera, ScrollDelta,
};
use crate::engine::scene::celestial_sphere::CelestialSphere;
use crate::engine::scene::satellite::{Propagation, Satellite};
use crate::engine::scene::{
    self, CameraTarget, Clock, RenderState, SatelliteMarker, SatelliteTelemetry, Scene,
    marker_occluded,
};
use crate::engine::ui::{
    DualReadout, Header, Instrument, InteractiveSlider, InteractiveToggle, PanelAnchor, Readout,
    Slider, Toggle, UIDrawable, UIDrawablePanel,
};

// This scene's tracked-object TLE, inlined as a source literal. Unlike the
// textures/ephemeris/EOP (build-downloaded straight into `OUT_DIR` and baked
// into the binary), this small element set lives directly in source so a fresh
// checkout needs no data file. The lines are column-sensitive TLE format (each
// element line is exactly 69 chars) - keep the exact spacing. `concat!` keeps
// source indentation out of the string. satkit parses by column and does not
// verify the trailing checksum digit. `new` below assembles the tracked array
// from this via `Satellite::from_tle`. (Deliberately duplicated from
// `iss_and_hubble.rs` - each scene owns its own TLE data.)

/// The International Space Station (ISS / ZARYA), epoch 2024-001.5. Real
/// element set.
const ISS_TLE: &str = concat!(
    "ISS (ZARYA)\n",
    "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9003\n",
    "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299357\n",
);

/// ISS-only simulation: the clock + celestial sphere held directly, plus this
/// scene's single tracked satellite.
pub struct IssScene {
    /// Simulation clock (datetime + play/paused + speed).
    clock: Clock,
    /// Ephemeris-driven celestial sphere, re-evaluated by `advance` while the
    /// clock runs.
    celestial_sphere: CelestialSphere,
    satellites: Vec<Satellite>,
    /// The scene's interactive orbital camera (pan/tilt/zoom rig plus its
    /// animations); the default whole-Terra view.
    camera: PtzCamera,
}

impl IssScene {
    fn new() -> Self {
        let satellites = vec![Satellite::from_tle(ISS_TLE)];
        let epoch = satellites.first().expect("TLE present").epoch();
        // `scene::init` must already have run (the celestial sphere reads
        // satkit globals).
        let clock = Clock::new(epoch);
        Self {
            celestial_sphere: CelestialSphere::at(&clock.now()),
            clock,
            satellites,
            camera: PtzCamera::default(),
        }
    }
}

impl Scene for IssScene {
    fn advance(&mut self) -> bool {
        // Advance the clock and, while it is running, re-evaluate the
        // ephemeris-driven celestial sphere at the new time. Returns whether
        // the clock is running - an "animating" source that keeps frames
        // coming; when paused nothing advances and the app can go idle.
        let running = self.clock.tick();
        if running {
            self.celestial_sphere = CelestialSphere::at(&self.clock.now());
        }
        running
    }
}

impl CameraControl for IssScene {
    // The input methods forward to the embedded PtzCamera; the forwarding
    // block is deliberately duplicated per scene (like the Time panel) so
    // a scene can diverge - e.g. gate input or swap the camera kind.
    fn pointer_press(&mut self, button: PointerButton) -> bool {
        self.camera.pointer_press(button)
    }

    fn pointer_release(&mut self, button: PointerButton) -> bool {
        self.camera.pointer_release(button)
    }

    fn pointer_move(&mut self, position: (f64, f64), viewport_height: f64) -> bool {
        self.camera.pointer_move(position, viewport_height)
    }

    fn scroll(&mut self, delta: ScrollDelta) -> bool {
        self.camera.scroll(delta)
    }

    fn tick(&mut self, viewport_height: f64) -> bool {
        self.camera.tick(viewport_height)
    }

    fn cursor_hint(&self) -> CursorHint {
        self.camera.cursor_hint()
    }
}

impl CameraView for IssScene {
    fn frame_state(&mut self) -> RenderState {
        let now = self.clock.now();

        // Resolve the camera first: re-aim at Terra (a same-body refresh) and
        // build the rig against this frame's sphere - the eye feeds the
        // marker-occlusion test below.
        let celestial_to_world = self.celestial_sphere.star_rot_inv.transpose();
        let target = CameraTarget::terra();
        self.camera
            .retarget(target, &self.celestial_sphere, celestial_to_world);
        let (eye, look_at, up) = self
            .camera
            .world_rig(&self.celestial_sphere, celestial_to_world);

        let mut markers = Vec::with_capacity(self.satellites.len());
        for sat in &mut self.satellites {
            let state = sat.state_at(&now);
            markers.push(SatelliteMarker {
                position_km: state.position_km,
                // Terra target, so the render-frame eye is the absolute eye.
                visible: !marker_occluded(eye, state.position_km),
                // The renderer propagates this ahead for the orbit path.
                propagation: Propagation::Sgp4(Box::new(sat.tle().clone())),
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

impl UIDrawable for IssScene {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        // The Time panel first, then this scene's own telemetry panel. Both
        // the panel builder and the readout loop are deliberately kept
        // per-scene (like the propagation loop) - scenes may diverge in what
        // they expose and how.
        //
        // Snapshot the displayed values up front (owned values only), so no
        // borrow of the clock or the satellites outlives into the mutable
        // callback captures below. The two control callbacks capture disjoint
        // clock fields (`paused` vs `multiplier`) via direct field assignment
        // - a `Clock` method would borrow the whole clock and collide.
        //
        // The readout re-propagates each satellite at the same instant
        // `frame_state` used (`Clock::now()` is pure and nothing ticks the
        // clock between the two calls) and SGP4 is deterministic, so the
        // values match the rendered markers with no stashed state.
        let now = self.clock.now();
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

        let datetime = self.clock.datetime_label();
        // Padded to the widest value (MAX_MULTIPLIER "100.0" = 5 chars): the
        // font is monospace, so a fixed-width value keeps the digit window
        // from resizing as the speed changes.
        let speed = format!("{:>5.1}", self.clock.multiplier);
        let running = !self.clock.paused;

        // Exponential (base e) speed: the slider edits the exponent, so
        // multiplier = e^exp - real time (e^0 = 1x) at the left, 100x at the
        // right, 10x at the midpoint. The mapping lives here, not in the panel.
        let speed_exp = self.clock.multiplier.ln();
        let exp_range = Clock::MIN_MULTIPLIER.ln()..=Clock::MAX_MULTIPLIER.ln();

        // The producer groups instruments into rows + picks content only; all
        // styling and every metric live in the instrument modules / theme
        // (taffy bottom-aligns the Run key with the speed window beside it).
        let time_rows: Vec<Vec<Box<dyn Instrument + '_>>> = vec![
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
                    on_toggle: Box::new(|| self.clock.paused = !self.clock.paused),
                }),
            ],
            vec![Box::new(InteractiveSlider {
                slider: Slider {
                    value: speed_exp,
                    range: exp_range,
                },
                on_change: Box::new(|exp| self.clock.multiplier = exp.exp()),
            })],
        ];
        let mut panels = vec![UIDrawablePanel {
            anchor: PanelAnchor::TopLeft,
            rows: time_rows,
        }];
        let mut rows: Vec<Vec<Box<dyn Instrument>>> = Vec::with_capacity(telemetry.len() * 3);
        for sat in &telemetry {
            // One header + two readout rows per satellite; taffy stacks the
            // groups (the repeated header rules the panel into sections).
            rows.push(vec![Box::new(Header {
                title: sat.name.clone(),
            })]);
            // Values are padded to their widest form ("-179.99" / "9999.9"):
            // the font is monospace, so fixed-width values keep the digit
            // windows from resizing (and the Lon window from shifting) as the
            // satellite moves.
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

/// Builds the ISS simulation and hands off to the winit event loop. Blocks
/// until the window closes.
pub fn run() {
    // Seed satkit's global state (embedded ephemeris + EOP table) before
    // anything else: IssScene::new below builds the CelestialSphere
    // (which reads the ephemeris) and the satellite parses a TLE. Doing it
    // here keeps satkit fully offline and data-dir-free.
    scene::init();

    application::run(ApplicationState::new(IssScene::new()));
}
