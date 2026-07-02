//! Manually-controlled satellite scenario: one object seeded from the ISS TLE
//! but propagated **numerically** (satkit `orbitprop`, no TLE retained), so
//! the user can reshape the orbit at runtime. A bottom-center "Burns" panel
//! offers the six orbital-frame thrust keys (prograde / retrograde, normal /
//! anti-normal, radial out / radial in); while a key is held the thrust
//! acceleration integrates into the GCRF velocity, and the marker, predicted
//! orbit path, and apsis readouts all follow the post-burn state
//! (CLI: `globe-experiment scenario manual_control`).

use glam::{DVec3, Vec3};
use satkit::Instant;

use crate::application::{self, ApplicationState};
use crate::simulation::celestial_sphere::CelestialSphere;
use crate::simulation::satellite::{self, OrbitShape, OrbitState, Propagation, Satellite};
use crate::simulation::{
    self, RenderState, SatelliteMarker, Simulation, SimulationState, marker_occluded,
};
use crate::ui::{
    Button, DualReadout, Header, Instrument, InteractiveHoldButton, PanelAnchor, Readout,
    UIDrawable, UIDrawablePanel,
};

// This scenario's seed TLE, inlined as a source literal - see `iss.rs` for the
// format notes. (Deliberately duplicated per scenario.) Unlike the tracking
// scenarios the element set is used ONCE, to bootstrap the initial GCRF state
// vector; after that the orbit belongs to the user.

/// The International Space Station (ISS / ZARYA), epoch 2024-001.5. Real
/// element set - the starting orbit only.
const ISS_TLE: &str = concat!(
    "ISS (ZARYA)\n",
    "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9003\n",
    "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299357\n",
);

/// Thrust acceleration while a burn key is held, m/s^2. Deliberately game-like
/// (~1 g, vastly stronger than any real station thruster): holding prograde
/// for ~10 s of simulation time adds ~100 m/s of delta-v, enough to visibly
/// reshape the predicted orbit ellipse while you watch. Scaled by simulation
/// dt, so time-warp burns harder in wall time (physically consistent).
const BURN_ACCEL_M_S2: f64 = 10.0;

/// This frame's panel readout, stashed by `frame_state` for the
/// immediately-following `get_drawables` so it matches the rendered marker.
/// `shape` is `None` after a burn to escape (e >= 1: no apsides).
struct ManualTelemetry {
    latitude_deg: f32,
    longitude_deg: f32,
    altitude_km: f32,
    shape: Option<OrbitShape>,
}

/// Manually-controlled simulation: the shared core by composition, plus the
/// satellite's live GCRF state vector (re-anchored to the clock every frame)
/// and the six burn request flags.
pub struct ManualControlSimulation {
    simulation: SimulationState,
    /// Object name from the seed TLE, for the panel header.
    name: String,
    /// The satellite's GCRF state vector, valid at `orbit_epoch`. THE orbit -
    /// burns mutate its velocity, and each frame's numerical propagation
    /// carries the result forward. No TLE behind it after seeding.
    orbit: OrbitState,
    /// The instant `orbit` is valid at; advanced to the clock each frame.
    orbit_epoch: Instant,
    /// Burn request flags, one per key. A held key sets its flag during the
    /// egui pass; `advance` folds them into a velocity change next frame,
    /// then clears them (the selector request-flag pattern). Disjoint fields
    /// (not an array) so the six key callbacks can each capture one `&mut`.
    burn_prograde: bool,
    burn_retrograde: bool,
    burn_normal: bool,
    burn_anti_normal: bool,
    burn_radial_out: bool,
    burn_radial_in: bool,
    /// See [`ManualTelemetry`]. `None` until the first frame.
    last_telemetry: Option<ManualTelemetry>,
}

impl ManualControlSimulation {
    fn new() -> Self {
        // The TLE lives exactly long enough to produce the initial conditions:
        // one SGP4 sample at its own epoch, converted to a GCRF state vector.
        let mut seed = Satellite::from_tle(ISS_TLE);
        let epoch = seed.epoch();
        let orbit = seed.state_at(&epoch).orbit;
        Self {
            simulation: SimulationState::new(epoch),
            name: seed.name,
            orbit,
            orbit_epoch: epoch,
            burn_prograde: false,
            burn_retrograde: false,
            burn_normal: false,
            burn_anti_normal: false,
            burn_radial_out: false,
            burn_radial_in: false,
            last_telemetry: None,
        }
    }

    /// The unit thrust direction for the currently-held burn keys, in GCRF:
    /// the orbital frame is derived from the live state (prograde = velocity,
    /// radial = position, normal = angular momentum r x v). Held opposing
    /// keys cancel; `None` when nothing is held (or everything cancels).
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

impl Simulation for ManualControlSimulation {
    fn advance(&mut self) -> bool {
        let running = self.simulation.advance();
        let now = self.simulation.clock.now();

        // Re-anchor the state vector to the clock: one numerical step over
        // this frame's simulation dt, so the stored initial conditions are
        // always "now" and a burn's velocity change compounds into every
        // later frame. Skipped when paused (dt = 0).
        let dt = (now - self.orbit_epoch).as_seconds();
        if dt > 0.0 {
            self.orbit = satellite::propagate_numerical(&self.orbit, &self.orbit_epoch, &now);
            self.orbit_epoch = now;

            // Thrust as one impulse per frame (Euler integration of a
            // continuous burn - frame dt is small enough that the chord
            // error is far below the game-like thrust's own fiction).
            // dt-scaled, so a paused clock burns nothing.
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

        running
    }

    fn celestial(&self) -> &CelestialSphere {
        &self.simulation.celestial_sphere
    }

    fn frame_state(&mut self, camera_pos: Vec3, look_at: Vec3, up: Vec3) -> RenderState {
        let now = self.simulation.clock.now();

        // `advance` just re-anchored the state to `now`, so this is a pure
        // frame change (GCRF -> the world-frame marker), no propagation.
        let state = satellite::resolve_orbit(&self.orbit, &now);
        let markers = vec![SatelliteMarker {
            position_km: state.position_km,
            // Terra target, so the render-frame eye is the absolute eye.
            visible: !marker_occluded(camera_pos, state.position_km),
            // Numerical propagation from the live post-burn state: the
            // predicted orbit path reshapes as the burn happens.
            propagation: Propagation::Numerical(self.orbit),
        }];

        self.last_telemetry = Some(ManualTelemetry {
            latitude_deg: state.latitude_deg,
            longitude_deg: state.longitude_deg,
            altitude_km: state.altitude_km,
            shape: satellite::orbit_shape(&self.orbit),
        });

        RenderState {
            time: now,
            camera_target: self.camera_target(),
            camera_pos,
            camera_look_at: look_at,
            camera_up: up,
            markers,
        }
    }
}

impl UIDrawable for ManualControlSimulation {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        // The shared-core panel first (its callbacks borrow `self.simulation`),
        // then the telemetry panel from the disjoint `last_telemetry`, then
        // the Burns panel whose key callbacks each capture one disjoint
        // `burn_*` flag - all coexisting borrows of separate fields.
        let mut panels = self.simulation.get_drawables();

        if let Some(telemetry) = &self.last_telemetry {
            // Values padded to their widest form (monospace font), so the
            // digit windows keep their size as the numbers move - see iss.rs.
            // Apsis windows show dashes on an escape orbit (no apsides).
            let (apo, peri) = match &telemetry.shape {
                Some(shape) => (
                    format!("{:>7.1}", shape.apoapsis_alt_km),
                    format!("{:>7.1}", shape.periapsis_alt_km),
                ),
                None => (format!("{:>7}", "---"), format!("{:>7}", "---")),
            };
            let speed = match &telemetry.shape {
                Some(shape) => format!("{:>7.1}", shape.speed_m_s),
                None => format!("{:>7.1}", self.orbit.vel_gcrf_m_s.length()),
            };
            let rows: Vec<Vec<Box<dyn Instrument + '_>>> = vec![
                vec![Box::new(Header {
                    title: self.name.clone(),
                })],
                vec![Box::new(DualReadout {
                    left_label: "Lat".to_string(),
                    left_value: format!("{:>7.2}", telemetry.latitude_deg),
                    left_unit: "deg".to_string(),
                    right_label: "Lon".to_string(),
                    right_value: format!("{:>7.2}", telemetry.longitude_deg),
                    right_unit: "deg".to_string(),
                })],
                vec![
                    Box::new(Readout {
                        label: "Alt".to_string(),
                        value: format!("{:>6.1}", telemetry.altitude_km),
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
        }

        // The Burns panel: six hold-to-fire keys in opposing pairs (paired
        // keys split each row). A held key sets its request flag every frame;
        // `advance` turns the flags into thrust.
        let rows: Vec<Vec<Box<dyn Instrument + '_>>> = vec![
            vec![Box::new(Header {
                title: "Burns".to_string(),
            })],
            vec![
                Box::new(InteractiveHoldButton {
                    button: Button {
                        label: "Prograde".to_string(),
                    },
                    on_hold: Box::new(|| self.burn_prograde = true),
                }),
                Box::new(InteractiveHoldButton {
                    button: Button {
                        label: "Retrograde".to_string(),
                    },
                    on_hold: Box::new(|| self.burn_retrograde = true),
                }),
            ],
            vec![
                Box::new(InteractiveHoldButton {
                    button: Button {
                        label: "Normal".to_string(),
                    },
                    on_hold: Box::new(|| self.burn_normal = true),
                }),
                Box::new(InteractiveHoldButton {
                    button: Button {
                        label: "Anti-Normal".to_string(),
                    },
                    on_hold: Box::new(|| self.burn_anti_normal = true),
                }),
            ],
            vec![
                Box::new(InteractiveHoldButton {
                    button: Button {
                        label: "Radial Out".to_string(),
                    },
                    on_hold: Box::new(|| self.burn_radial_out = true),
                }),
                Box::new(InteractiveHoldButton {
                    button: Button {
                        label: "Radial In".to_string(),
                    },
                    on_hold: Box::new(|| self.burn_radial_in = true),
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

/// Builds the manual-control simulation and hands off to the winit event
/// loop. Blocks until the window closes.
pub fn run() {
    // Seed satkit's global state (embedded ephemeris + EOP + IERS tables +
    // EGM96 gravity) before anything else: `new` parses a TLE and builds the
    // CelestialSphere, and every frame numerically propagates the orbit.
    simulation::init();

    application::run(ApplicationState::new(ManualControlSimulation::new()));
}
