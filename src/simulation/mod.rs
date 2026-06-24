//! Simulation state and astronomical math: the simulation clock and the
//! ephemeris-driven celestial sphere. This module owns the shared simulation
//! core and defines the `Simulation` trait that every scenario implements. It
//! is deliberately free of any windowing (winit), GPU (wgpu), or UI (egui)
//! dependency, and never references the camera type (the camera lives in
//! `application`).

pub mod celestial_sphere;
pub mod clock;
pub mod satellite;

use glam::{Mat3, Mat4, Vec3};
use satkit::Instant;

pub use clock::Clock;

use crate::earth;
use celestial_sphere::CelestialSphere;

/// The interface every scenario implements. `ApplicationState` is generic over
/// `S: Simulation` and calls only these methods, so adding or swapping a
/// scenario requires no changes to the application layer.
///
/// This trait is UI-agnostic. The egui panel reads/drives a scenario through a
/// separate `crate::ui::UIDrawable` impl (no `clock_mut`, no UI snapshots from
/// `frame_state`); keeping that trait in `ui` is what lets `simulation` stay
/// free of any UI dependency.
pub trait Simulation {
    /// Advance the clock and re-evaluate the celestial sphere. Returns whether
    /// the clock is running, i.e. the app should keep requesting frames.
    fn advance(&mut self) -> bool;

    /// Rotation from the inertial (star-fixed) camera rig frame to the
    /// Earth-fixed world frame. The application uses this to resolve the camera
    /// before computing eye and view_proj for `frame_state`.
    fn celestial_to_world(&self) -> Mat3;

    /// Produce this frame's render state from the application-resolved camera.
    /// Satellite propagation happens here, once per frame per satellite. The
    /// per-satellite readout produced by the same propagation is stashed on the
    /// scenario so the immediately-following
    /// `crate::ui::UIDrawable::get_drawables` call (the egui panel) reports
    /// values matching the rendered markers.
    fn frame_state(&mut self, eye: Vec3, view_proj: Mat4) -> RenderState;
}

/// The finished, render-ready positions/matrices for one frame: everything the
/// renderer needs, as plain `glam` data (no GPU types). Returned by
/// [`Simulation::frame_state`] from the application-resolved camera; the UI
/// readout for the same frame is pulled separately via `crate::ui::UIDrawable`.
#[derive(Clone, Debug)]
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
    /// One marker per tracked satellite, in the same order as the scenario's
    /// satellite list. The renderer draws them instanced.
    pub markers: Vec<SatelliteMarker>,
}

/// A single satellite's on-screen marker for one frame: where to draw it and
/// whether it is visible. Element of [`RenderState::markers`].
#[derive(Clone, Copy, Debug)]
pub struct SatelliteMarker {
    /// Marker position in the world frame (km).
    pub position_km: Vec3,
    /// Whether the marker is visible (false when the solid Earth occludes it).
    pub visible: bool,
}

/// One tracked satellite's readout for the UI panel. A scenario stashes a
/// `Vec<SatelliteTelemetry>` each frame in [`Simulation::frame_state`], built
/// from the same propagation that fills [`RenderState::markers`] (so readout
/// and marker can never disagree, and the orbit is propagated once per frame),
/// and turns it into `crate::ui::UIDrawableElement`s in its
/// `crate::ui::UIDrawable` impl.
#[derive(Clone, Debug)]
pub struct SatelliteTelemetry {
    /// Object name (e.g. "ISS (ZARYA)").
    pub name: String,
    /// Sub-satellite geodetic latitude, degrees.
    pub latitude_deg: f32,
    /// Sub-satellite geodetic longitude, degrees.
    pub longitude_deg: f32,
    /// Height above the WGS84 ellipsoid, kilometers.
    pub altitude_km: f32,
}

/// Seeds satkit's global state (embedded ephemeris + EOP table) for fully
/// offline, data-dir-free use. Must be called once at startup, before any
/// `CelestialSphere` is built. Thin wrapper over
/// `celestial_sphere::init_satkit` so callers (e.g. `main`) need not know
/// about the `celestial_sphere` submodule.
pub fn init() {
    celestial_sphere::init_satkit();
}

/// The shared simulation core: the clock (datetime + play/paused + speed) and
/// the ephemeris-driven celestial sphere (Sun direction + star-map
/// orientation). Held by composition inside each scenario's simulation struct,
/// which adds its own satellite list and implements [`Simulation`].
///
/// Does not own satellites - those live in the scenario struct so each scenario
/// can choose its own tracked objects without changing this shared core. This
/// struct owns the astronomical infrastructure; the `Simulation` trait owns the
/// per-scenario policy.
pub struct SimulationState {
    pub clock: Clock,
    pub celestial_sphere: CelestialSphere,
}

impl SimulationState {
    /// Builds the core state starting at `start_epoch`. `init` must already
    /// have run (the celestial sphere reads satkit globals).
    pub fn new(start_epoch: Instant) -> Self {
        let clock = Clock::new(start_epoch);
        let celestial_sphere = CelestialSphere::at(&clock.now());
        Self {
            clock,
            celestial_sphere,
        }
    }

    /// Advances the clock and, while it is running, re-evaluates the
    /// ephemeris-driven celestial sphere at the new time. Returns whether the
    /// clock is running - an "animating" source that keeps frames coming; when
    /// paused nothing advances and the app can go idle.
    pub fn advance(&mut self) -> bool {
        let running = self.clock.tick();
        if running {
            self.celestial_sphere = CelestialSphere::at(&self.clock.now());
        }
        running
    }

    /// Rotation from the inertial (star-fixed) frame the camera rig lives in
    /// to the Earth-fixed world frame the scene is drawn in - the inverse of
    /// the celestial sphere's world -> celestial rotation. (Orthonormal, so
    /// transpose = inverse.)
    pub fn celestial_to_world(&self) -> Mat3 {
        self.celestial_sphere.star_rot_inv.transpose()
    }
}

/// Whether the solid Earth blocks the line of sight from `eye` to `target`
/// (both world-space km). Approximates the planet as a sphere of mean Earth
/// radius - slightly conservative against the WGS84 ellipsoid, which is fine
/// for deciding whether to hide the marker.
pub(crate) fn marker_occluded(eye: Vec3, target: Vec3) -> bool {
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
