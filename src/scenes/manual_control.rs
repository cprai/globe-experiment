//! Manually-controlled satellite scene: one object seeded from the ISS TLE
//! but propagated **numerically** (satkit `orbitprop`, no TLE retained). A
//! bottom-center "Burns" panel holds six orbital-frame thrust keys; while
//! held, the thrust integrates into the GCRF velocity and the marker,
//! predicted path, and apsis readouts follow (CLI: `globe-experiment scene
//! manual_control`).

use glam::DVec3;
use satkit::Instant;

use crate::engine::application::{self, ApplicationState};
use crate::engine::camera::{CameraView, PtzCamera, ScenePtzCamera};
use crate::engine::scene::celestial_sphere::CelestialSphere;
use crate::engine::scene::satellite::{self, OrbitState, Propagation, Satellite};
use crate::engine::scene::{
    self, CameraTarget, Clock, RenderState, SatelliteMarker, Scene, SceneClock, marker_occluded,
};
use crate::engine::ui::{
    Button, DualReadout, Header, Instrument, InteractiveHoldButton, InteractiveSlider,
    InteractiveToggle, PanelAnchor, Readout, Slider, Toggle, UIDrawable, UIDrawablePanel,
};

// Seed TLE - see `iss.rs` for the column-sensitive format notes.
// Deliberately duplicated per scene - do not factor into a shared const.
// Used ONCE to bootstrap the initial GCRF state; after that the orbit
// belongs to the user.

/// The International Space Station (ISS / ZARYA), epoch 2024-001.5. Real
/// element set - the starting orbit only.
const ISS_TLE: &str = concat!(
    "ISS (ZARYA)\n",
    "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9003\n",
    "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299357\n",
);

/// Thrust while a burn key is held, m/s^2. Deliberately game-like (~1 g,
/// vastly stronger than any real station thruster): ~10 s of held prograde
/// adds ~100 m/s of delta-v, enough to visibly reshape the predicted orbit.
/// Scaled by simulation dt, so time-warp burns harder in wall time.
const BURN_ACCEL_M_S2: f64 = 10.0;

/// Speed-slider range: real time to 100x.
const MIN_MULTIPLIER: f32 = 1.0;
const MAX_MULTIPLIER: f32 = 100.0;

/// Manually-controlled simulation: clock, the satellite's live GCRF state
/// vector (re-anchored to the clock every frame), and the burn request flags.
#[derive(SceneClock)]
pub struct ManualControlScene {
    clock: Clock,
    /// Object name from the seed TLE, for the panel header.
    name: String,
    /// THE orbit - burns mutate its velocity, and each frame's numerical
    /// propagation carries the result forward. No TLE behind it after seeding.
    orbit: OrbitState,
    /// The instant `orbit` is valid at; advanced to the clock each frame.
    orbit_epoch: Instant,
    /// Burn request flags, one per key, set by the held keys' callbacks
    /// during the egui pass. Flags (not direct edits) because the burn is
    /// dt-scaled: only `advance` knows the frame's simulation dt.
    burn_prograde: bool,
    burn_retrograde: bool,
    burn_normal: bool,
    burn_anti_normal: bool,
    burn_radial_out: bool,
    burn_radial_in: bool,
    camera: PtzCamera,
    /// Fixed at Terra (no selector), so it never reframes.
    camera_target: CameraTarget,
}

impl ManualControlScene {
    fn new() -> Self {
        // The TLE lives exactly long enough to produce the initial
        // conditions: one SGP4 sample at its own epoch, converted to a GCRF
        // state vector (reads satkit globals - `scene::init` must have run).
        let mut seed = Satellite::from_tle(ISS_TLE);
        let epoch = seed.epoch();
        let orbit = seed.state_at(&epoch).orbit;
        Self {
            clock: Clock::new(epoch),
            name: seed.name,
            orbit,
            orbit_epoch: epoch,
            burn_prograde: false,
            burn_retrograde: false,
            burn_normal: false,
            burn_anti_normal: false,
            burn_radial_out: false,
            burn_radial_in: false,
            camera: PtzCamera::default(),
            camera_target: CameraTarget::terra(),
        }
    }

    /// Unit thrust direction for the held burn keys, in GCRF (prograde =
    /// velocity, radial = position, normal = r x v). Held opposing keys
    /// cancel; `None` when nothing is held or everything cancels.
    fn burn_direction(&self) -> Option<DVec3> {
        let radial = self.orbit.pos_gcrf_m.normalize();
        let prograde = self.orbit.vel_gcrf_m_s.normalize();
        let normal = self
            .orbit
            .pos_gcrf_m
            .cross(self.orbit.vel_gcrf_m_s)
            .normalize();

        let mut sum = DVec3::ZERO;
        if self.burn_prograde {
            sum += prograde;
        }
        if self.burn_retrograde {
            sum -= prograde;
        }
        if self.burn_normal {
            sum += normal;
        }
        if self.burn_anti_normal {
            sum -= normal;
        }
        if self.burn_radial_out {
            sum += radial;
        }
        if self.burn_radial_in {
            sum -= radial;
        }
        sum.try_normalize()
    }
}

impl Scene for ManualControlScene {
    fn advance(&mut self) {
        let now = self.clock_now();

        // Re-anchor the state vector to the clock: one numerical step over
        // this frame's simulation dt, so the stored initial conditions are
        // always "now" and a burn's velocity change compounds into every
        // later frame. Skipped when paused (dt = 0).
        let dt = (now - self.orbit_epoch).as_seconds();
        if dt > 0.0 {
            self.orbit = satellite::propagate_numerical(&self.orbit, &self.orbit_epoch, &now);
            self.orbit_epoch = now;

            // Thrust as one impulse per frame (Euler integration of a
            // continuous burn; frame dt keeps the chord error far below the
            // game-like thrust's own fiction). dt-scaled, so a paused clock
            // burns nothing.
            if let Some(direction) = self.burn_direction() {
                self.orbit.vel_gcrf_m_s += direction * (BURN_ACCEL_M_S2 * dt);
            }
        }

        // Held keys re-set their flags during the coming egui pass; clearing
        // here (not after the pass) is what makes "held" mean "burning".
        self.burn_prograde = false;
        self.burn_retrograde = false;
        self.burn_normal = false;
        self.burn_anti_normal = false;
        self.burn_radial_out = false;
        self.burn_radial_in = false;
    }
}

impl ScenePtzCamera for ManualControlScene {
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

impl CameraView for ManualControlScene {
    fn frame_state(&mut self) -> RenderState {
        let now = self.clock_now();
        let sphere = CelestialSphere::at(&now);

        let celestial_to_world = sphere.star_rot_inv.transpose();
        let target = self.camera_target;
        let (eye, look_at, up) = self.camera.world_rig(&target, &sphere, celestial_to_world);

        // `advance` just re-anchored the state to `now`, so this is a pure
        // frame change (GCRF -> world-frame marker), no propagation.
        let state = satellite::resolve_orbit(&self.orbit, &now);
        let markers = vec![SatelliteMarker {
            position_km: state.position_km,
            // Terra target, so the render-frame eye is the absolute eye.
            visible: !marker_occluded(eye, state.position_km),
            // Numerical propagation from the live post-burn state: the
            // predicted orbit path reshapes as the burn happens.
            propagation: Propagation::Numerical(self.orbit),
        }];

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

impl UIDrawable for ManualControlScene {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<Self>> {
        // Snapshot displayed values up front - the panels are owned and never
        // borrow the scene. The readout re-derives from the live state at the
        // same instant `frame_state` used (`advance` anchored `self.orbit` to
        // it this redraw), so it matches the rendered marker. `shape` is
        // `None` after a burn to escape (e >= 1: no apsides).
        let now = self.clock_now();
        let state = satellite::resolve_orbit(&self.orbit, &now);
        let shape = satellite::orbit_shape(&self.orbit);

        let datetime = self.clock_datetime_label();
        // Padded to the widest value (monospace font) so the digit window
        // does not resize as the speed changes.
        let clock_speed = format!("{:>5.1}", self.clock_multiplier());
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
                    value: clock_speed,
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

        // Values padded to their widest form (monospace) - see iss.rs. Apsis
        // windows show dashes on an escape orbit (no apsides).
        let (apo, peri) = match &shape {
            Some(shape) => (
                format!("{:>7.1}", shape.apoapsis_alt_km),
                format!("{:>7.1}", shape.periapsis_alt_km),
            ),
            None => (format!("{:>7}", "---"), format!("{:>7}", "---")),
        };
        let speed = match &shape {
            Some(shape) => format!("{:>7.1}", shape.speed_m_s),
            None => format!("{:>7.1}", self.orbit.vel_gcrf_m_s.length()),
        };
        let rows: Vec<Vec<Box<dyn Instrument<Self>>>> = vec![
            vec![Box::new(Header {
                title: self.name.clone(),
            })],
            vec![Box::new(DualReadout {
                left_label: "Lat".to_string(),
                left_value: format!("{:>7.2}", state.latitude_deg as f32),
                left_unit: "deg".to_string(),
                right_label: "Lon".to_string(),
                right_value: format!("{:>7.2}", state.longitude_deg as f32),
                right_unit: "deg".to_string(),
            })],
            vec![
                Box::new(Readout {
                    label: "Alt".to_string(),
                    value: format!("{:>6.1}", state.altitude_km as f32),
                    unit: "km".to_string(),
                }),
                Box::new(Readout {
                    label: "Speed".to_string(),
                    value: speed,
                    unit: "m/s".to_string(),
                }),
            ],
            vec![Box::new(DualReadout {
                left_label: "Apo".to_string(),
                left_value: apo,
                left_unit: "km".to_string(),
                right_label: "Peri".to_string(),
                right_value: peri,
                right_unit: "km".to_string(),
            })],
        ];
        panels.push(UIDrawablePanel {
            anchor: PanelAnchor::TopRight,
            rows,
        });

        // Burns panel: six hold-to-fire keys in opposing pairs. A held key
        // sets its request flag every frame; `advance` turns flags into
        // thrust.
        let rows: Vec<Vec<Box<dyn Instrument<Self>>>> = vec![
            vec![Box::new(Header {
                title: "Burns".to_string(),
            })],
            vec![
                Box::new(InteractiveHoldButton {
                    button: Button {
                        label: "Prograde".to_string(),
                    },
                    on_hold: Box::new(|scene: &mut Self| scene.burn_prograde = true),
                }),
                Box::new(InteractiveHoldButton {
                    button: Button {
                        label: "Retrograde".to_string(),
                    },
                    on_hold: Box::new(|scene: &mut Self| scene.burn_retrograde = true),
                }),
            ],
            vec![
                Box::new(InteractiveHoldButton {
                    button: Button {
                        label: "Normal".to_string(),
                    },
                    on_hold: Box::new(|scene: &mut Self| scene.burn_normal = true),
                }),
                Box::new(InteractiveHoldButton {
                    button: Button {
                        label: "Anti-Normal".to_string(),
                    },
                    on_hold: Box::new(|scene: &mut Self| scene.burn_anti_normal = true),
                }),
            ],
            vec![
                Box::new(InteractiveHoldButton {
                    button: Button {
                        label: "Radial Out".to_string(),
                    },
                    on_hold: Box::new(|scene: &mut Self| scene.burn_radial_out = true),
                }),
                Box::new(InteractiveHoldButton {
                    button: Button {
                        label: "Radial In".to_string(),
                    },
                    on_hold: Box::new(|scene: &mut Self| scene.burn_radial_in = true),
                }),
            ],
        ];
        panels.push(UIDrawablePanel {
            anchor: PanelAnchor::BottomCenter,
            rows,
        });

        panels
    }
}

/// The `scene manual_control` CLI arguments - none today.
#[derive(clap::Args)]
pub struct Args {}

/// Builds the manual-control simulation and runs the winit event loop until
/// close.
pub fn run(_args: Args) {
    // Seed satkit's globals (ephemeris + EOP + IERS tables + EGM96 gravity)
    // before the TLE parse and the per-frame numerical propagation.
    scene::init();

    application::run(ApplicationState::new(ManualControlScene::new()));
}
