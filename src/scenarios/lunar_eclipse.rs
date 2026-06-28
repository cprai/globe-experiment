//! Lunar-eclipse scenario: the 2025-03-14 total lunar eclipse, with no tracked
//! objects - just the celestial sphere wound to the event, so the Earth's
//! shadow falls on the Moon and turns it a coppery "blood moon" (CLI:
//! `globe-experiment scenario lunar_eclipse`). Like `solar_eclipse` it carries
//! no `Satellite` list; its clock starts directly from the eclipse datetime,
//! and it draws no markers.

use glam::{Mat3, Mat4, Vec3};
use satkit::Instant;

use crate::application::{self, ApplicationState, Camera};
use crate::simulation::{self, RenderState, Simulation, SimulationState};
use crate::ui::{UIDrawable, UIDrawablePanel};

/// Eye distance for the framing (km): near the maximum, so the Earth is small
/// and leaves room for the eclipsed Moon beside it.
const VIEW_DISTANCE_KM: f32 = 63000.0;

/// Degrees to nudge the look axis off the Moon so the Earth's limb does not
/// occlude the eclipsed disc. From a geocentric camera that looks straight at
/// the Moon, the Moon sits almost directly behind the Earth; this offset (a bit
/// more than the Earth's apparent radius at `VIEW_DISTANCE_KM`) clears it.
const LIMB_OFFSET_DEG: f32 = 13.0;

/// Empty lunar-eclipse simulation: just the shared core (clock + celestial
/// sphere); no satellites.
pub struct LunarEclipseSimulation {
    simulation: SimulationState,
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
        }
    }
}

impl Simulation for LunarEclipseSimulation {
    fn advance(&mut self) -> bool {
        self.simulation.advance()
    }

    fn celestial_to_world(&self) -> Mat3 {
        self.simulation.celestial_to_world()
    }

    fn frame_state(&mut self, eye: Vec3, view_proj: Mat4) -> RenderState {
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

impl UIDrawable for LunarEclipseSimulation {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        // Just the shared-core panel; nothing is tracked.
        self.simulation.get_drawables()
    }
}

/// Builds the lunar-eclipse scene, framed with the eclipsed Moon clear of the
/// Earth's limb, and hands off to the winit event loop.
pub fn run() {
    simulation::init();

    let sim = LunarEclipseSimulation::new();

    // Aim at the Moon, then nudge off it so the Earth's limb does not occlude
    // the eclipsed disc.
    let celestial = &sim.simulation.celestial_sphere;
    let mut camera = Camera::looking_toward(
        celestial.star_rot_inv,
        celestial.moon_pos_world,
        VIEW_DISTANCE_KM,
    );
    camera.latitude = (camera.latitude + LIMB_OFFSET_DEG).clamp(-89.0, 89.0);

    application::run(ApplicationState::with_camera(sim, camera));
}
