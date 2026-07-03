//! Lunar-eclipse scenario: the 2025-03-14 total lunar eclipse, with no tracked
//! objects - just the celestial sphere wound to the event, so Terra's
//! shadow falls on Luna and turns it a coppery "blood-red Luna" (CLI:
//! `globe-experiment scenario lunar_eclipse`). Like `solar_eclipse` it carries
//! no `Satellite` list; its clock starts directly from the eclipse datetime,
//! and it draws no markers.

use glam::Vec3;
use satkit::Instant;

use crate::engine::application::{self, ApplicationState};
use crate::engine::camera::Camera;
use crate::engine::simulation::celestial_sphere::CelestialSphere;
use crate::engine::simulation::{
    self, CameraTarget, CelestialBody, Clock, RenderState, Simulation, TargetSelector,
};
use crate::engine::ui::{
    Header, Instrument, InteractiveSlider, InteractiveToggle, PanelAnchor, Readout, Slider, Toggle,
    UIDrawable, UIDrawablePanel,
};

/// Eye distance for the Luna framing (km): ~2 lunar radii above the surface, so
/// the eclipsed disc fills the frame with a little margin (the camera orbits
/// Luna, so the distance is relative to its surface, not Terra's).
const VIEW_DISTANCE_KM: f32 = 3500.0;

/// Empty lunar-eclipse simulation: just the clock + celestial sphere; no
/// satellites. Carries a [`TargetSelector`] so the view can be switched
/// between orbiting Luna (the default blood-red-Luna framing) and orbiting
/// Terra.
pub struct LunarEclipseSimulation {
    /// Simulation clock (datetime + play/paused + speed).
    clock: Clock,
    /// Ephemeris-driven celestial sphere, re-evaluated by `advance` while the
    /// clock runs.
    celestial_sphere: CelestialSphere,
    selector: TargetSelector,
}

impl LunarEclipseSimulation {
    fn new() -> Self {
        // ~30 min before greatest eclipse (06:58 UTC) - the start of totality -
        // so the auto-playing clock runs through the deep umbral phase. Well
        // inside the bundled EOP range (1962-01-01 .. build date).
        let epoch =
            Instant::from_datetime(2025, 3, 14, 6, 28, 0.0).expect("valid lunar-eclipse datetime");
        // `simulation::init` must already have run (the celestial sphere reads
        // satkit globals).
        let clock = Clock::new(epoch);
        Self {
            celestial_sphere: CelestialSphere::at(&clock.now()),
            clock,
            // Default to orbiting Luna - the whole point is the blood-red Luna.
            selector: TargetSelector::new(true),
        }
    }
}

impl Simulation for LunarEclipseSimulation {
    fn advance(&mut self) -> bool {
        // Fold in any pending target-selector key press before the camera target
        // is read this frame.
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

    fn celestial(&self) -> &CelestialSphere {
        &self.celestial_sphere
    }

    fn camera_target(&self) -> CameraTarget {
        self.selector.resolve()
    }

    fn frame_state(&mut self, camera_pos: Vec3, look_at: Vec3, up: Vec3) -> RenderState {
        // No satellites: an empty marker list. The renderer derives the Terra
        // system from the frame's time; the selector's target (Luna or Terra)
        // keeps the origin at Terra either way.
        RenderState {
            time: self.clock.now(),
            camera_target: self.camera_target(),
            camera_pos,
            camera_look_at: look_at,
            camera_up: up,
            markers: Vec::new(),
        }
    }
}

impl UIDrawable for LunarEclipseSimulation {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        // The Time panel (datetime + run/speed) plus the Terra / Luna
        // camera-target selector. The panels borrow disjoint fields (`clock`
        // vs `selector`). The panel builder is deliberately kept per-scenario
        // - scenarios may diverge in what they expose.
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

/// Builds the lunar-eclipse scene already orbiting Luna (the eclipsed
/// near-side disc centered) and hands off to the winit event loop.
pub fn run() {
    simulation::init();

    let sim = LunarEclipseSimulation::new();

    // Orbit Luna, looking at its Terra-facing near side (which is the side
    // in Terra's shadow - the blood-red Luna). Looking *toward* Luna places the
    // eye on its Terra-facing side, so Terra is behind the camera and never
    // occludes the disc - no limb nudge needed.
    let celestial = &sim.celestial_sphere;
    let center = celestial.luna().placement.pos_world;
    let camera = Camera::looking_toward(
        CameraTarget::Body(CelestialBody::LUNA),
        celestial.star_rot_inv,
        center,
        VIEW_DISTANCE_KM,
    );

    application::run(ApplicationState::with_camera(sim, camera));
}
