//! Lunar-eclipse scenario: the 2025-03-14 total lunar eclipse, with no tracked
//! objects - just the celestial sphere wound to the event, so the Earth's
//! shadow falls on the Moon and turns it a coppery "blood moon" (CLI:
//! `globe-experiment scenario lunar_eclipse`). Like `solar_eclipse` it carries
//! no `Satellite` list; its clock starts directly from the eclipse datetime,
//! and it draws no markers.

use glam::{Mat3, Mat4, Vec3};
use satkit::Instant;

use crate::application::{self, ApplicationState, Camera};
use crate::simulation::{
    self, CameraTarget, RenderState, Simulation, SimulationState, TargetSelector,
};
use crate::ui::{UIDrawable, UIDrawablePanel};

/// Eye distance for the Moon framing (km): ~2 lunar radii above the surface, so
/// the eclipsed disc fills the frame with a little margin (the camera orbits
/// the Moon, so the distance is relative to its surface, not the Earth's).
const VIEW_DISTANCE_KM: f32 = 3500.0;

/// Empty lunar-eclipse simulation: just the shared core (clock + celestial
/// sphere); no satellites. Carries a [`TargetSelector`] so the view can be
/// switched between orbiting the Moon (the default blood-moon framing) and
/// orbiting the Earth.
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
            // Default to orbiting the Moon - the whole point is the blood moon.
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

    fn celestial_to_world(&self) -> Mat3 {
        self.simulation.celestial_to_world()
    }

    fn camera_target(&self) -> CameraTarget {
        self.selector
            .resolve(self.simulation.celestial_sphere.moon_pos_world)
    }

    fn frame_state(&mut self, eye: Vec3, view_proj: Mat4) -> RenderState {
        let celestial = &self.simulation.celestial_sphere;
        RenderState {
            view_proj,
            camera_pos: eye,
            // Earth/Moon targets keep the origin at Earth (planet-free scene).
            render_origin: Vec3::ZERO,
            sun_dir: celestial.sun_dir,
            sun_pos_world: celestial.sun_pos_world,
            star_rot_inv: celestial.star_tex_rot_inv,
            moon_pos_world: celestial.moon_pos_world,
            moon_rot: celestial.moon_rot,
            moon_radius_km: celestial.moon_radius_km,
            planets: Vec::new(),
            markers: Vec::new(),
        }
    }
}

impl UIDrawable for LunarEclipseSimulation {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        // The shared-core panel plus the EARTH / MOON camera-target selector.
        // The two panels borrow disjoint fields (`simulation` vs `selector`).
        let mut panels = self.simulation.get_drawables();
        panels.push(self.selector.panel());
        panels
    }
}

/// Builds the lunar-eclipse scene already orbiting the Moon (the eclipsed
/// near-side disc centered) and hands off to the winit event loop.
pub fn run() {
    simulation::init();

    let sim = LunarEclipseSimulation::new();

    // Orbit the Moon, looking at its Earth-facing near side (which is the side
    // in Earth's shadow - the blood moon). Looking *toward* the Moon places the
    // eye on its Earth-facing side, so the Earth is behind the camera and never
    // occludes the disc - no limb nudge needed.
    let celestial = &sim.simulation.celestial_sphere;
    let center = celestial.moon_pos_world;
    let camera = Camera::looking_toward(
        CameraTarget::Moon {
            center_world: center,
        },
        celestial.star_rot_inv,
        center,
        VIEW_DISTANCE_KM,
    );

    application::run(ApplicationState::with_camera(sim, camera));
}
