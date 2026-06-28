//! Solar-eclipse scenario: the 2024-04-08 total solar eclipse, with no tracked
//! objects - just the celestial sphere wound to the event, so the Moon's shadow
//! sweeps across the daylit Earth (CLI: `globe-experiment scenario
//! solar_eclipse`). Unlike the satellite scenarios this carries no `Satellite`
//! list; its clock starts directly from the eclipse datetime rather than a TLE
//! epoch, and it draws no markers.

use glam::{Mat3, Mat4, Vec3};
use satkit::Instant;

use crate::application::{self, ApplicationState, Camera};
use crate::simulation::{self, RenderState, Simulation, SimulationState};
use crate::ui::{UIDrawable, UIDrawablePanel};

/// Eye distance for the day-side framing (km): the Earth fills most of the
/// frame with the Moon's umbral shadow spot centered near the subsolar point.
const VIEW_DISTANCE_KM: f32 = 22000.0;

/// Empty solar-eclipse simulation: just the shared core (clock + celestial
/// sphere); no satellites.
pub struct SolarEclipseSimulation {
    simulation: SimulationState,
}

impl SolarEclipseSimulation {
    fn new() -> Self {
        // ~30 min before greatest eclipse (18:17 UTC), so the auto-playing clock
        // runs into and through the umbra's crossing of North America. Well
        // inside the bundled EOP range (1962-01-01 .. build date), so the
        // ephemeris/Earth-orientation accuracy holds.
        let epoch =
            Instant::from_datetime(2024, 4, 8, 17, 47, 0.0).expect("valid solar-eclipse datetime");
        Self {
            simulation: SimulationState::new(epoch),
        }
    }
}

impl Simulation for SolarEclipseSimulation {
    fn advance(&mut self) -> bool {
        self.simulation.advance()
    }

    fn celestial_to_world(&self) -> Mat3 {
        self.simulation.celestial_to_world()
    }

    fn frame_state(&mut self, eye: Vec3, view_proj: Mat4) -> RenderState {
        // No satellites: an empty marker list, the celestial state straight from
        // the shared core.
        let celestial = &self.simulation.celestial_sphere;
        RenderState {
            view_proj,
            camera_pos: eye,
            sun_dir: celestial.sun_dir,
            star_rot_inv: celestial.star_tex_rot_inv,
            moon_pos_world: celestial.moon_pos_world,
            moon_rot: celestial.moon_rot,
            moon_radius_km: celestial.moon_radius_km,
            markers: Vec::new(),
        }
    }
}

impl UIDrawable for SolarEclipseSimulation {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        // Just the shared-core panel (datetime + subsolar + run/speed); there is
        // no satellite panel because nothing is tracked.
        self.simulation.get_drawables()
    }
}

/// Builds the solar-eclipse scene, framed on the daylit face so the Moon's
/// shadow spot is in view, and hands off to the winit event loop.
pub fn run() {
    // Seed satkit's globals (embedded ephemeris + EOP) before the celestial
    // sphere is built in `new` below.
    simulation::init();

    let sim = SolarEclipseSimulation::new();

    // Frame the sunlit face (and the Moon's shadow spot near the subsolar point)
    // by looking along -sun_dir, computed from the ephemeris at the start
    // instant. The view stays interactive afterward.
    let celestial = &sim.simulation.celestial_sphere;
    let camera =
        Camera::looking_toward(celestial.star_rot_inv, -celestial.sun_dir, VIEW_DISTANCE_KM);

    application::run(ApplicationState::with_camera(sim, camera));
}
