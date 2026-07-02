//! Solar-system scenario: a free tour of the whole solar system with no tracked
//! objects - the celestial sphere wound to a fixed past date, and a body
//! selector (one latching key per body) that flies the camera to and orbits any
//! of Terra, Luna, or the seven planets (CLI: `globe-experiment scenario
//! solar_system`).
//! Like the eclipse scenarios it carries no `Satellite` list and draws no
//! markers; unlike them it draws all seven planets, each at its true
//! geocentric position and scale.
//!
//! Because the outer planets sit billions of km from Terra - far past f32
//! precision in world-km - a planet target renders with a floating origin (the
//! scene is drawn relative to the orbited planet's center; see
//! `CameraTarget::render_origin`). Terra/Luna targets keep the origin at Terra.

use glam::Vec3;
use satkit::Instant;

use crate::application::{self, ApplicationState};
use crate::simulation::celestial_sphere::CelestialSphere;
use crate::simulation::{
    self, BodySelector, CameraTarget, RenderState, Simulation, SimulationState,
};
use crate::ui::{UIDrawable, UIDrawablePanel};

/// Empty solar-system simulation: the shared core (clock + celestial sphere)
/// plus the body selector. No satellites.
pub struct SolarSystemSimulation {
    simulation: SimulationState,
    selector: BodySelector,
}

impl SolarSystemSimulation {
    fn new() -> Self {
        // A fixed recent past date, well inside the bundled EOP range
        // (1962-01-01 .. build date), so every body's position is accurate. The
        // clock auto-plays from here; the planets and their phases evolve.
        let epoch =
            Instant::from_datetime(2025, 6, 1, 0, 0, 0.0).expect("valid solar-system datetime");
        Self {
            simulation: SimulationState::new(epoch),
            selector: BodySelector::default(),
        }
    }
}

impl Simulation for SolarSystemSimulation {
    fn advance(&mut self) -> bool {
        // Fold in any pending body-key press before the camera target is read.
        self.selector.apply_requests();
        self.simulation.advance()
    }

    fn celestial(&self) -> &CelestialSphere {
        &self.simulation.celestial_sphere
    }

    fn camera_target(&self) -> CameraTarget {
        self.selector.resolve()
    }

    fn frame_state(&mut self, camera_pos: Vec3, look_at: Vec3, up: Vec3) -> RenderState {
        // No satellites: an empty marker list. The renderer derives every
        // body's position from the frame's time and uses the camera target's
        // render origin (which must match the one the camera built its rig
        // against - both come from `camera_target()` this frame).
        RenderState {
            time: self.simulation.clock.now(),
            camera_target: self.camera_target(),
            camera_pos,
            camera_look_at: look_at,
            camera_up: up,
            markers: Vec::new(),
        }
    }
}

impl UIDrawable for SolarSystemSimulation {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        // The shared-core panel plus the one-key-per-body selector. The two
        // panels borrow disjoint fields (`simulation` vs `selector`).
        let mut panels = self.simulation.get_drawables();
        panels.push(self.selector.panel());
        panels
    }
}

/// Builds the solar-system scene and hands off to the winit event loop. Starts
/// on the default whole-Terra view; the body-selector keys then tour the
/// system.
pub fn run() {
    simulation::init();
    application::run(ApplicationState::new(SolarSystemSimulation::new()));
}
