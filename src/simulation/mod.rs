//! Simulation state and astronomical math: the simulation clock, the tracked
//! satellite, and the ephemeris-driven celestial sphere. This module owns
//! everything about *what is being simulated* and computes the positions of the
//! rendered objects; it is deliberately free of any windowing (winit), GPU
//! (wgpu), or UI (egui) dependency, and never references the camera type (the
//! camera lives in `application`).

pub mod celestial_sphere;
pub mod clock;
pub mod satellite;

use glam::{Mat3, Mat4, Vec3};

use crate::earth;
use celestial_sphere::CelestialSphere;
use clock::Clock;
use satellite::Satellite;

/// The finished, render-ready positions/matrices for one frame: everything the
/// renderer needs, as plain `glam` data (no GPU types). Produced together with
/// [`TelemetryState`] by [`SimulationState::frame_state`] from the
/// application-resolved camera.
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
    /// One marker per tracked satellite, in the same order as
    /// `SimulationState::satellites`. The renderer draws them instanced.
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

/// Everything the UI readout displays for one frame, as a plain owned snapshot
/// (no GPU/winit/egui types). Produced together with [`RenderState`] by
/// [`SimulationState::frame_state`] so the on-screen lat/lon/altitude come from
/// the *same* satellite propagation the marker uses - they can never disagree,
/// and the orbit is propagated only once per frame. The clock's own controls
/// (play/pause + speed) are *not* here: the UI mutates the `Clock` directly, so
/// it reads those from the live clock rather than a snapshot.
#[derive(Clone, Debug)]
pub struct TelemetryState {
    /// Subsolar geodetic latitude, degrees (ephemeris-derived).
    pub subsolar_lat_deg: f32,
    /// Subsolar geodetic longitude, degrees (ephemeris-derived).
    pub subsolar_lon_deg: f32,
    /// Current simulation datetime, formatted for display (UTC).
    pub datetime_label: String,
    /// One readout per tracked satellite, in the same order as
    /// `SimulationState::satellites`.
    pub satellites: Vec<SatelliteTelemetry>,
}

/// One tracked satellite's readout for the UI panel. Element of
/// [`TelemetryState::satellites`].
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
/// `celestial_sphere::init_satkit` so callers (e.g. `main`) need not know about
/// the `celestial_sphere` submodule.
pub fn init() {
    celestial_sphere::init_satkit();
}

/// All simulation state: the clock (datetime + play/paused + speed), the
/// tracked satellites (each a TLE; positions propagated on demand), and the
/// ephemeris-driven celestial sphere (Sun direction + star-map orientation).
/// Held by composition so each subsystem keeps its own tuned logic.
///
/// This struct owns *what is being simulated* and the astronomical math over
/// it; it does not own the camera (that lives in `application`) and carries no
/// windowing/GPU/UI types.
pub struct SimulationState {
    pub clock: Clock,
    /// The tracked objects, assembled by the caller (`main`) and passed to
    /// [`new`](Self::new). Rendered as markers in this order.
    pub satellites: Vec<Satellite>,
    pub celestial_sphere: CelestialSphere,
}

impl SimulationState {
    /// Builds the initial state from a caller-provided list of satellites: the
    /// clock starts at the first satellite's TLE epoch and the celestial sphere
    /// is evaluated at that same time. `init` must already have run (the
    /// satellites/celestial sphere read satkit globals). Panics if `satellites`
    /// is empty - the clock needs a start epoch and the tool always tracks at
    /// least one object.
    pub fn new(satellites: Vec<Satellite>) -> Self {
        let epoch = satellites
            .first()
            .expect("at least one satellite to track")
            .epoch();
        let clock = Clock::new(epoch);
        let celestial_sphere = CelestialSphere::at(&clock.now());
        Self {
            clock,
            satellites,
            celestial_sphere,
        }
    }

    /// Advances the simulation by the wall-clock delta since the last call:
    /// the clock steps, and while it is running the ephemeris-driven celestial
    /// sphere is re-evaluated at the new time. The satellite carries no stored
    /// state - it is propagated on demand from the clock's time in
    /// `frame_state`, which feeds the result to both the renderer and the UI.
    /// Returns whether the clock is running - an "animating" source that keeps
    /// frames coming; when paused nothing advances and the app can go idle.
    pub fn advance(&mut self) -> bool {
        let running = self.clock.tick();
        if running {
            self.celestial_sphere = CelestialSphere::at(&self.clock.now());
        }
        running
    }

    /// Rotation from the inertial (star-fixed) frame the camera rig lives in
    /// to the Earth-fixed world frame the scene is drawn in - the inverse of
    /// the celestial sphere's world -> celestial rotation. The application
    /// applies this to resolve the camera into the world frame before building
    /// its view. (Orthonormal, so transpose = inverse.)
    pub fn celestial_to_world(&self) -> Mat3 {
        self.celestial_sphere.star_rot_inv.transpose()
    }

    /// Produces this frame's [`RenderState`] (for the renderer) and
    /// [`TelemetryState`] (for the UI readout) together, from the
    /// application-resolved camera (world-frame `eye` and `view_proj`). Each
    /// tracked satellite is propagated *once* at the clock's current time and
    /// that single result feeds both its marker and its readout, so they always
    /// agree and every orbit is propagated only once per frame. The marker's
    /// visibility is the Earth-occlusion test of the line of sight from `eye`
    /// to the object. Takes `&mut self` because the satellites are
    /// propagated on demand (satkit's `sgp4` needs `&mut` to cache its
    /// initialization).
    pub fn frame_state(&mut self, eye: Vec3, view_proj: Mat4) -> (RenderState, TelemetryState) {
        let now = self.clock.now();

        let mut markers = Vec::with_capacity(self.satellites.len());
        let mut satellites = Vec::with_capacity(self.satellites.len());
        for satellite in &mut self.satellites {
            let sat = satellite.state_at(&now);
            markers.push(SatelliteMarker {
                position_km: sat.position_km,
                visible: !marker_occluded(eye, sat.position_km),
            });
            satellites.push(SatelliteTelemetry {
                name: satellite.name.clone(),
                latitude_deg: sat.latitude_deg,
                longitude_deg: sat.longitude_deg,
                altitude_km: sat.altitude_km,
            });
        }

        let render = RenderState {
            view_proj,
            camera_pos: eye,
            sun_dir: self.celestial_sphere.sun_dir,
            star_rot_inv: self.celestial_sphere.star_rot_inv,
            markers,
        };
        let telemetry = TelemetryState {
            subsolar_lat_deg: self.celestial_sphere.subsolar_lat_deg,
            subsolar_lon_deg: self.celestial_sphere.subsolar_lon_deg,
            datetime_label: self.clock.datetime_label(),
            satellites,
        };
        (render, telemetry)
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
