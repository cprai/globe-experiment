//! Lunar-eclipse scenario: the 2025-03-14 total lunar eclipse, with no tracked
//! objects - just the celestial sphere wound to the event, so Terra's
//! shadow falls on Luna and turns it a coppery "blood-red Luna" (CLI:
//! `globe-experiment scenario lunar_eclipse`). Like `solar_eclipse` it carries
//! no `Satellite` list; its clock starts directly from the eclipse datetime,
//! and it draws no markers.

use glam::Vec3;
use satkit::Instant;

use crate::application::{self, ApplicationState, Camera};
use crate::simulation::celestial_sphere::CelestialSphere;
use crate::simulation::{
    self, CameraTarget, CelestialBody, RenderState, Simulation, SimulationState, TargetSelector,
};
use crate::ui::{UIDrawable, UIDrawablePanel};

/// Eye distance for the Luna framing (km): ~2 lunar radii above the surface, so
/// the eclipsed disc fills the frame with a little margin (the camera orbits
/// Luna, so the distance is relative to its surface, not Terra's).
const VIEW_DISTANCE_KM: f32 = 3500.0;

/// Empty lunar-eclipse simulation: just the shared core (clock + celestial
/// sphere); no satellites. Carries a [`TargetSelector`] so the view can be
/// switched between orbiting Luna (the default blood-red-Luna framing) and
/// orbiting Terra.
pub struct LunarEclipseSimulation {
    simulation: SimulationState,
    selector: TargetSelector,
}

impl LunarEclipseSimulation {
    fn new() -> Self {
        // ~30 min before greatest eclipse (06:58 UTC) - the start of totality -
        // so the auto-playing clock runs through the deep umbral phase. Well
        // inside the bundled EOP range (1962-01-01 .. build date).
        let epoch =
            Instant::from_datetime(2025, 3, 14, 6, 28, 0.0).expect("valid lunar-eclipse datetime");
        Self {
            simulation: SimulationState::new(epoch),
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
        self.simulation.advance()
    }

    fn celestial(&self) -> &CelestialSphere {
        &self.simulation.celestial_sphere
    }

    fn camera_target(&self) -> CameraTarget {
        self.selector.resolve()
    }

    fn frame_state(&mut self, camera_pos: Vec3, look_at: Vec3, up: Vec3) -> RenderState {
        // No satellites: an empty marker list. The renderer derives the Terra
        // system from the frame's time; the selector's target (Luna or Terra)
        // keeps the origin at Terra either way.
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

impl UIDrawable for LunarEclipseSimulation {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        // The shared-core panel plus the TERRA / LUNA camera-target selector.
        // The two panels borrow disjoint fields (`simulation` vs `selector`).
        let mut panels = self.simulation.get_drawables();
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
    let celestial = &sim.simulation.celestial_sphere;
    let center = celestial.luna().placement.pos_world;
    let camera = Camera::looking_toward(
        CameraTarget::Body(CelestialBody::LUNA),
        celestial.star_rot_inv,
        center,
        VIEW_DISTANCE_KM,
    );

    application::run(ApplicationState::with_camera(sim, camera));
}
