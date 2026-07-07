//! Solar-system scene: a free tour of the whole solar system with no tracked
//! objects - the celestial sphere wound to a fixed past date, and a body
//! selector (one latching key per body) that flies the camera to and orbits any
//! of Terra, Luna, or the seven planets (CLI: `globe-experiment scene
//! solar_system`).
//! Like the eclipse scenes it carries no `Satellite` list and draws no
//! markers; unlike them it draws all seven planets, each at its true
//! geocentric position and scale.
//!
//! Because the outer planets sit billions of km from Terra - far past f32
//! precision in world-km - a planet target renders with a floating origin (the
//! scene is drawn relative to the orbited planet's center; see
//! `CameraTarget::render_origin`). Terra/Luna targets keep the origin at Terra.

use satkit::Instant;

use crate::engine::application::{self, ApplicationState};
use crate::engine::camera::{
    CameraControl, CameraView, CursorHint, PointerButton, PtzCamera, ScrollDelta,
};
use crate::engine::scene::celestial_sphere::CelestialSphere;
use crate::engine::scene::{self, BodySelector, CameraTarget, Clock, RenderState, Scene};
use crate::engine::ui::{
    Header, Instrument, InteractiveSlider, InteractiveToggle, PanelAnchor, Readout, Slider, Toggle,
    UIDrawable, UIDrawablePanel,
};

/// Empty solar-system simulation: the clock + celestial sphere held directly,
/// plus the body selector. No satellites.
pub struct SolarSystemScene {
    /// Simulation clock (datetime + play/paused + speed).
    clock: Clock,
    /// Ephemeris-driven celestial sphere, re-evaluated by `advance` while the
    /// clock runs.
    celestial_sphere: CelestialSphere,
    selector: BodySelector,
    /// The scene's interactive orbital camera (pan/tilt/zoom rig plus its
    /// animations); starts on the default whole-Terra view.
    camera: PtzCamera,
    /// The body the camera orbits this frame - owned by the scene, not the
    /// camera, and passed into every camera call that scales by or centers
    /// on it. Refreshed from the selector each frame; a genuine switch
    /// reframes the camera (`frame_state` compares via `same_kind`).
    camera_target: CameraTarget,
}

impl SolarSystemScene {
    fn new() -> Self {
        // A fixed recent past date, well inside the bundled EOP range
        // (1962-01-01 .. build date), so every body's position is accurate. The
        // clock auto-plays from here; the planets and their phases evolve.
        let epoch =
            Instant::from_datetime(2025, 6, 1, 0, 0, 0.0).expect("valid solar-system datetime");
        // `scene::init` must already have run (the celestial sphere reads
        // satkit globals).
        let clock = Clock::new(epoch);
        Self {
            celestial_sphere: CelestialSphere::at(&clock.now()),
            clock,
            selector: BodySelector::default(),
            camera: PtzCamera::default(),
            // Matches the selector default (Terra) and the whole-Terra camera
            // above, so the first frame does not reframe.
            camera_target: CameraTarget::terra(),
        }
    }
}

impl Scene for SolarSystemScene {
    fn advance(&mut self) -> bool {
        // Fold in any pending body-key press before the camera target is read.
        self.selector.apply_requests();
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

impl CameraControl for SolarSystemScene {
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
        self.camera
            .pointer_move(&self.camera_target, position, viewport_height)
    }

    fn scroll(&mut self, delta: ScrollDelta) -> bool {
        self.camera.scroll(&self.camera_target, delta)
    }

    fn tick(&mut self, viewport_height: f64) -> bool {
        self.camera.tick(&self.camera_target, viewport_height)
    }

    fn cursor_hint(&self) -> CursorHint {
        self.camera.cursor_hint()
    }
}

impl CameraView for SolarSystemScene {
    fn frame_state(&mut self) -> RenderState {
        // Refresh the scene-owned camera target from the selector; a genuine
        // body switch reframes the camera (full-frame distance, re-aim,
        // in-flight animations dropped). Then resolve the inertial rig into
        // the render frame (the selected body's moving center is re-resolved
        // from the ephemeris inside `world_rig`).
        let celestial_to_world = self.celestial_sphere.star_rot_inv.transpose();
        let target = self.selector.resolve();
        if !self.camera_target.same_kind(&target) {
            self.camera
                .reframe(&target, &self.celestial_sphere, celestial_to_world);
        }
        self.camera_target = target;
        let (eye, look_at, up) =
            self.camera
                .world_rig(&target, &self.celestial_sphere, celestial_to_world);

        // No satellites: an empty marker list. The renderer derives every
        // body's position from the frame's time and uses the camera target's
        // render origin (which must match the one the camera built its rig
        // against - both are the single `target` resolved above).
        RenderState {
            time: self.clock.now(),
            camera_target: target,
            camera_pos: eye,
            camera_look_at: look_at,
            camera_up: up,
            markers: Vec::new(),
        }
    }
}

impl UIDrawable for SolarSystemScene {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        // The Time panel (datetime + run/speed) plus the one-key-per-body
        // selector. The panels borrow disjoint fields (`clock` vs `selector`).
        // The panel builder is deliberately kept per-scene - scenes may
        // diverge in what they expose.
        //
        // Snapshot the displayed values up front (owned `String`/`f32`/`bool`),
        // so no shared borrow of the clock outlives into the mutable callback
        // captures below. The two control callbacks capture disjoint clock
        // fields (`paused` vs `multiplier`) via direct field assignment - a
        // `Clock` method would borrow the whole clock and collide.
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
        panels.push(self.selector.panel());
        panels
    }
}

/// Builds the solar-system scene and hands off to the winit event loop. Starts
/// on the default whole-Terra view; the body-selector keys then tour the
/// system.
pub fn run() {
    scene::init();
    application::run(ApplicationState::new(SolarSystemScene::new()));
}
