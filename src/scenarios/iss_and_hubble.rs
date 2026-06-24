//! ISS + Hubble scenario: track the International Space Station and the Hubble
//! Space Telescope from their shared ~2024-001.5 TLE epoch. This is the
//! original default scene, now expressed as a named scenario (CLI:
//! `globe-experiment scenario iss_and_hubble`).

use glam::{Mat3, Mat4, Vec3};

use crate::application::{self, ApplicationState};
use crate::simulation::satellite::Satellite;
use crate::simulation::{
    self, Clock, RenderState, SatelliteMarker, SatelliteTelemetry, ScenarioUIState, Simulation,
    SimulationState, SimulationUIState, marker_occluded,
};

// This scenario's tracked-object TLEs, inlined as source literals. Unlike the
// textures/ephemeris/EOP (build-downloaded straight into `OUT_DIR` and baked
// into the binary), these small element sets live directly in source so a fresh
// checkout needs no data file. The lines are column-sensitive TLE format (each
// element line is exactly 69 chars) - keep the exact spacing. `concat!` keeps
// source indentation out of the string. satkit parses by column and does not
// verify the trailing checksum digit. `new` below assembles the tracked array
// from these via `Satellite::from_tle`.

/// The International Space Station (ISS / ZARYA), epoch 2024-001.5. Real
/// element set.
const ISS_TLE: &str = concat!(
    "ISS (ZARYA)\n",
    "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9003\n",
    "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299357\n",
);

/// The Hubble Space Telescope (HST), epoch ~2024-001.5. NOTE: the orbit shape
/// is realistic (inclination 28.47 deg, ~540 km / ~15.1 rev/day), but this set
/// was assembled from memory - the RAAN/anomaly/epoch-fraction phase is
/// approximate. Replace with a freshly fetched TLE for true positional
/// accuracy. Included as a second tracked object so multiple markers render.
const HST_TLE: &str = concat!(
    "HST\n",
    "1 20580U 90037B   24001.49473380  .00002000  00000-0  10000-3 0  9990\n",
    "2 20580  28.4690  85.5400 0002600 310.0000  50.0000 15.09600000123456\n",
);

/// ISS + Hubble simulation: the shared core (clock + celestial sphere) via
/// composition, plus this scenario's two tracked satellites.
pub struct IssAndHubbleSimulation {
    simulation: SimulationState,
    satellites: Vec<Satellite>,
}

impl IssAndHubbleSimulation {
    fn new() -> Self {
        // The clock starts at the first satellite's TLE epoch, so order
        // matters: the primary object (ISS) goes first.
        let satellites = vec![Satellite::from_tle(ISS_TLE), Satellite::from_tle(HST_TLE)];
        let epoch = satellites.first().expect("TLE present").epoch();
        Self {
            simulation: SimulationState::new(epoch),
            satellites,
        }
    }
}

impl Simulation for IssAndHubbleSimulation {
    fn advance(&mut self) -> bool {
        self.simulation.advance()
    }

    fn celestial_to_world(&self) -> Mat3 {
        self.simulation.celestial_to_world()
    }

    fn clock_mut(&mut self) -> &mut Clock {
        &mut self.simulation.clock
    }

    fn frame_state(
        &mut self,
        eye: Vec3,
        view_proj: Mat4,
    ) -> (RenderState, SimulationUIState, ScenarioUIState) {
        let now = self.simulation.clock.now();

        let mut markers = Vec::with_capacity(self.satellites.len());
        let mut sat_telemetry = Vec::with_capacity(self.satellites.len());
        for sat in &mut self.satellites {
            let state = sat.state_at(&now);
            markers.push(SatelliteMarker {
                position_km: state.position_km,
                visible: !marker_occluded(eye, state.position_km),
            });
            sat_telemetry.push(SatelliteTelemetry {
                name: sat.name.clone(),
                latitude_deg: state.latitude_deg,
                longitude_deg: state.longitude_deg,
                altitude_km: state.altitude_km,
            });
        }

        let render = RenderState {
            view_proj,
            camera_pos: eye,
            sun_dir: self.simulation.celestial_sphere.sun_dir,
            star_rot_inv: self.simulation.celestial_sphere.star_rot_inv,
            markers,
        };
        let sim_ui = SimulationUIState {
            subsolar_lat_deg: self.simulation.celestial_sphere.subsolar_lat_deg,
            subsolar_lon_deg: self.simulation.celestial_sphere.subsolar_lon_deg,
            datetime_label: self.simulation.clock.datetime_label(),
        };
        let scenario_ui = ScenarioUIState {
            satellites: sat_telemetry,
        };
        (render, sim_ui, scenario_ui)
    }
}

/// Builds the ISS + Hubble simulation and hands off to the winit event loop.
/// Blocks until the window closes.
pub fn run() {
    // Seed satkit's global state (embedded ephemeris + EOP table) before
    // anything else: IssAndHubbleSimulation::new below builds the
    // CelestialSphere (which reads the ephemeris) and the satellites parse
    // TLEs. Doing it here keeps satkit fully offline and data-dir-free.
    simulation::init();

    application::run(ApplicationState::new(IssAndHubbleSimulation::new()));
}
