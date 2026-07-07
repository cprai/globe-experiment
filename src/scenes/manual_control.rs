//! Manually-controlled satellite scene: one object seeded from the ISS TLE
//! but propagated **numerically** (satkit `orbitprop`, no TLE retained), so
//! the user can reshape the orbit at runtime. A bottom-center "Burns" panel
//! offers the six orbital-frame thrust keys (prograde / retrograde, normal /
//! anti-normal, radial out / radial in); while a key is held the thrust
//! acceleration integrates into the GCRF velocity, and the marker, predicted
//! orbit path, and apsis readouts all follow the post-burn state
//! (CLI: `globe-experiment scene manual_control`).

use glam::DVec3;
use satkit::Instant;

use crate::engine::application::{self, ApplicationState};
use crate::engine::camera::{
    CameraControl, CameraView, CursorHint, PointerButton, PtzCamera, ScrollDelta,
};
use crate::engine::scene::celestial_sphere::CelestialSphere;
use crate::engine::scene::satellite::{self, OrbitState, Propagation, Satellite};
use crate::engine::scene::{
    self, CameraTarget, Clock, RenderState, SatelliteMarker, Scene, marker_occluded,
};
use crate::engine::ui::{
    Button, DualReadout, Header, Instrument, InteractiveHoldButton, InteractiveSlider,
    InteractiveToggle, PanelAnchor, Readout, Slider, Toggle, UIDrawable, UIDrawablePanel,
};

// This scene's seed TLE, inlined as a source literal - see `iss.rs` for the
// format notes. (Deliberately duplicated per scene.) Unlike the tracking
// scenes the element set is used ONCE, to bootstrap the initial GCRF state
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

/// Manually-controlled simulation: the clock + celestial sphere held directly,
/// plus the satellite's live GCRF state vector (re-anchored to the clock every
/// frame) and the six burn request flags.
pub struct ManualControlScene {
    /// Simulation clock (datetime + play/paused + speed).
    clock: Clock,
    /// Ephemeris-driven celestial sphere, re-evaluated by `advance` while the
    /// clock runs.
    celestial_sphere: CelestialSphere,
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
    /// The scene's interactive orbital camera (pan/tilt/zoom rig plus its
    /// animations); the default whole-Terra view.
    camera: PtzCamera,
}

impl ManualControlScene {
    fn new() -> Self {
        // The TLE lives exactly long enough to produce the initial conditions:
        // one SGP4 sample at its own epoch, converted to a GCRF state vector.
        let mut seed = Satellite::from_tle(ISS_TLE);
        let epoch = seed.epoch();
        let orbit = seed.state_at(&epoch).orbit;
        // `scene::init` must already have run (the celestial sphere reads
        // satkit globals).
        let clock = Clock::new(epoch);
        Self {
            celestial_sphere: CelestialSphere::at(&clock.now()),
            clock,
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

impl Scene for ManualControlScene {
    fn advance(&mut self) -> bool {
        // Advance the clock and, while it is running, re-evaluate the
        // ephemeris-driven celestial sphere at the new time (paused = nothing
        // advances and the app can go idle).
        let running = self.clock.tick();
        if running {
            self.celestial_sphere = CelestialSphere::at(&self.clock.now());
        }
        let now = self.clock.now();

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
}

impl CameraControl for ManualControlScene {
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

impl CameraView for ManualControlScene {
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

        // `advance` just re-anchored the state to `now`, so this is a pure
        // frame change (GCRF -> the world-frame marker), no propagation.
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
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        // The Time panel first (its callbacks capture disjoint `self.clock`
        // fields), then the telemetry panel, then the Burns panel whose key
        // callbacks each capture one disjoint `burn_*` flag - all coexisting
        // borrows of separate fields. The panel builder is deliberately kept
        // per-scene (like the propagation loop) - scenes may diverge in what
        // they expose.
        //
        // Snapshot the displayed values up front (owned values only), so no
        // shared borrow of the clock or the orbit outlives into the mutable
        // callback captures below. The two control callbacks capture disjoint
        // clock fields (`paused` vs `multiplier`) via direct field assignment
        // - a `Clock` method would borrow the whole clock and collide.
        //
        // The readout re-derives from the live state at the same instant
        // `frame_state` used (`Clock::now()` is pure, `advance` anchored
        // `self.orbit` to it earlier this redraw, and nothing mutated either
        // since), so this pure frame change matches the rendered marker with
        // no stashed state. `shape` is `None` after a burn to escape (e >= 1:
        // no apsides).
        let state = satellite::resolve_orbit(&self.orbit, &self.clock.now());
        let shape = satellite::orbit_shape(&self.orbit);

        let datetime = self.clock.datetime_label();
        // Padded to the widest value (MAX_MULTIPLIER "100.0" = 5 chars): the
        // font is monospace, so a fixed-width value keeps the digit window
        // from resizing as the speed changes.
        let clock_speed = format!("{:>5.1}", self.clock.multiplier);
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
                    value: clock_speed,
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

        // Values padded to their widest form (monospace font), so the
        // digit windows keep their size as the numbers move - see iss.rs.
        // Apsis windows show dashes on an escape orbit (no apsides).
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
        let rows: Vec<Vec<Box<dyn Instrument + '_>>> = vec![
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
    scene::init();

    application::run(ApplicationState::new(ManualControlScene::new()));
}
