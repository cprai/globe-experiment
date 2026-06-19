//! Simulation state and astronomical math: the simulation clock, the tracked
//! satellite, and the ephemeris-driven sky. This module owns everything about
//! *what is being simulated* and computes the positions of the rendered
//! objects; it is deliberately free of any windowing (winit), GPU (wgpu), or UI
//! (egui) dependency, and never references the camera type (the camera lives in
//! `application`). See `REFACTOR_PLAN.md`.

pub mod clock;
pub mod satellite;
pub mod sky;

use glam::{Mat3, Mat4, Vec3};

use crate::earth;
use clock::Clock;
use satellite::Satellite;
use sky::Sky;

/// The finished, render-ready positions/matrices for one frame: everything the
/// renderer needs, as plain `glam` data (no GPU types). Produced by
/// [`SimulationState::render_state`] from the application-resolved camera.
#[derive(Clone, Copy, Debug)]
pub struct RenderState {
    /// World-frame view-projection matrix (built by the application from the
    /// camera + `celestial_to_world` + viewport aspect).
    pub view_proj: Mat4,
    /// Camera eye position in the Earth-fixed world frame (km).
    pub camera_pos: Vec3,
    /// Unit vector toward the Sun in the world frame.
    pub sun_dir: Vec3,
    /// World -> celestial rotation for the star-map lookup (uploaded as
    /// `star_rot_inv`).
    pub star_rot_inv: Mat3,
    /// Space-station marker position in the world frame (km).
    pub sat_pos: Vec3,
    /// Whether the marker is visible (false when the solid Earth occludes it).
    pub marker_visible: bool,
}

/// Seeds satkit's global state (embedded ephemeris + EOP table) for fully
/// offline, data-dir-free use. Must be called once at startup, before any
/// `Sky` is built. Thin wrapper over `sky::init_satkit` so callers (e.g.
/// `main`) need not know about the `sky` submodule.
pub fn init() {
    sky::init_satkit();
}

/// All simulation state: the clock (datetime + play/paused + speed), the
/// tracked satellite (TLE + last propagated state), and the ephemeris-driven
/// sky (Sun direction + star-map orientation). Held by composition so each
/// subsystem keeps its own tuned logic.
///
/// This struct owns *what is being simulated* and the astronomical math over
/// it; it does not own the camera (that lives in `application`) and carries no
/// windowing/GPU/UI types.
pub struct SimulationState {
    pub clock: Clock,
    pub satellite: Satellite,
    pub sky: Sky,
}

impl SimulationState {
    /// Builds the initial state: the satellite is loaded first, the clock
    /// starts at its TLE epoch, and the sky is evaluated at that same time.
    /// `init` must already have run (the satellite/sky read satkit globals).
    pub fn new() -> Self {
        let satellite = Satellite::load();
        let clock = Clock::new(satellite.epoch());
        let sky = Sky::at(&clock.now());
        Self {
            clock,
            satellite,
            sky,
        }
    }

    /// Advances the simulation by the wall-clock delta since the last call:
    /// the clock steps, and while it is running the satellite is re-propagated
    /// and the ephemeris-driven sky re-evaluated at the new time. Returns
    /// whether the clock is running - an "animating" source that keeps frames
    /// coming; when paused nothing advances and the app can go idle.
    pub fn advance(&mut self) -> bool {
        let running = self.clock.tick();
        if running {
            let now = self.clock.now();
            self.satellite.update_to(&now);
            self.sky = Sky::at(&now);
        }
        running
    }

    /// Rotation from the inertial (star-fixed) frame the camera rig lives in
    /// to the Earth-fixed world frame the scene is drawn in - the inverse of
    /// the sky's world -> celestial rotation. The application applies this to
    /// resolve the camera into the world frame before building its view; see
    /// `REFACTOR_PLAN.md`. (Orthonormal, so transpose = inverse.)
    pub fn celestial_to_world(&self) -> Mat3 {
        self.sky.star_rot_inv.transpose()
    }

    /// Produces the frame's [`RenderState`] from the application-resolved
    /// camera: the world-frame `eye` and `view_proj`. The astronomical fields
    /// (Sun, star rotation, satellite position) come from the current sky and
    /// satellite, and the marker's visibility is the Earth-occlusion test of
    /// the line of sight from `eye` to the station.
    pub fn render_state(&self, eye: Vec3, view_proj: Mat4) -> RenderState {
        RenderState {
            view_proj,
            camera_pos: eye,
            sun_dir: self.sky.sun_dir,
            star_rot_inv: self.sky.star_rot_inv,
            sat_pos: self.satellite.position_km,
            marker_visible: !marker_occluded(eye, self.satellite.position_km),
        }
    }
}

impl Default for SimulationState {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether the solid Earth blocks the line of sight from `eye` to `target`
/// (both world-space km). Approximates the planet as a sphere of mean Earth
/// radius - slightly conservative against the WGS84 ellipsoid, which is fine
/// for deciding whether to hide the marker.
fn marker_occluded(eye: Vec3, target: Vec3) -> bool {
    let to_target = target - eye;
    let distance = to_target.length();
    if distance <= 1e-3 {
        return false;
    }
    let dir = to_target / distance;

    // Ray-sphere intersection of the line of sight with the Earth sphere.
    let b = dir.dot(eye);
    let c = eye.length_squared() - earth::MEAN_RADIUS_KM * earth::MEAN_RADIUS_KM;
    let disc = b * b - c;
    if disc < 0.0 {
        return false; // line of sight misses the Earth entirely
    }
    let t = -b - disc.sqrt(); // nearest intersection along the ray
    t > 0.0 && t < distance // Earth sits between the eye and the station
}
