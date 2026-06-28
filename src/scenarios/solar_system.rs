//! Solar-system scenario: a free tour of the whole solar system with no tracked
//! objects - the celestial sphere wound to a fixed past date, and a body
//! selector (PREV / NEXT) that flies the camera to and orbits any of Earth, the
//! Moon, or the seven planets (CLI: `globe-experiment scenario solar_system`).
//! Like the eclipse scenarios it carries no `Satellite` list and draws no
//! markers; unlike them it draws all seven planets, each at its true
//! geocentric position and scale.
//!
//! Because the outer planets sit billions of km from Earth - far past f32
//! precision in world-km - a planet target renders with a floating origin (the
//! scene is drawn relative to the orbited planet's center; see
//! `RenderState::render_origin`). Earth/Moon targets keep the origin at Earth.

use glam::{Mat3, Mat4, Vec3};
use satkit::Instant;

use crate::application::{self, ApplicationState};
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
        // Fold in any pending PREV/NEXT press before the camera target is read.
        self.selector.apply_requests();
        self.simulation.advance()
    }

    fn celestial_to_world(&self) -> Mat3 {
        self.simulation.celestial_to_world()
    }

    fn camera_target(&self) -> CameraTarget {
        self.selector.resolve(&self.simulation.celestial_sphere)
    }

    fn frame_state(&mut self, eye: Vec3, view_proj: Mat4) -> RenderState {
        // The render origin must match the camera's target this frame (the
        // camera builds view_proj against the same origin).
        let target = self.selector.resolve(&self.simulation.celestial_sphere);
        let celestial = &self.simulation.celestial_sphere;
        RenderState {
            view_proj,
            camera_pos: eye,
            render_origin: target.render_origin(),
            sun_dir: celestial.sun_dir,
            sun_pos_world: celestial.sun_pos_world,
            star_rot_inv: celestial.star_tex_rot_inv,
            moon_pos_world: celestial.moon_pos_world,
            moon_rot: celestial.moon_rot,
            moon_radius_km: celestial.moon_radius_km,
            // All seven planets, drawn at their true positions.
            planets: celestial.planets.to_vec(),
            markers: Vec::new(),
        }
    }
}

impl UIDrawable for SolarSystemSimulation {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        // The shared-core panel plus the PREV / NEXT body selector. The two
        // panels borrow disjoint fields (`simulation` vs `selector`).
        let mut panels = self.simulation.get_drawables();
        panels.push(self.selector.panel());
        panels
    }
}

/// Builds the solar-system scene and hands off to the winit event loop. Starts
/// on the default full-globe Earth view; PREV/NEXT then tour the system.
pub fn run() {
    simulation::init();
    application::run(ApplicationState::new(SolarSystemSimulation::new()));
}
