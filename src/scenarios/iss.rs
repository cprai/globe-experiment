//! ISS-only scenario: track the International Space Station from its
//! ~2024-001.5 TLE epoch. Same as `iss_and_hubble` but with Hubble omitted, so
//! a single marker renders (CLI: `globe-experiment scenario iss`).

use glam::{Mat3, Mat4, Vec3};

use crate::application::{self, ApplicationState};
use crate::simulation::satellite::Satellite;
use crate::simulation::{
    self, Clock, RenderState, SatelliteMarker, SatelliteTelemetry, ScenarioUIState, Simulation,
    SimulationState, SimulationUIState, marker_occluded,
};

// This scenario's tracked-object TLE, inlined as a source literal. Unlike the
// textures/ephemeris/EOP (build-downloaded straight into `OUT_DIR` and baked
// into the binary), this small element set lives directly in source so a fresh
// checkout needs no data file. The lines are column-sensitive TLE format (each
// element line is exactly 69 chars) - keep the exact spacing. `concat!` keeps
// source indentation out of the string. satkit parses by column and does not
// verify the trailing checksum digit. `new` below assembles the tracked array
// from this via `Satellite::from_tle`. (Deliberately duplicated from
// `iss_and_hubble.rs` - each scenario owns its own TLE data.)

/// The International Space Station (ISS / ZARYA), epoch 2024-001.5. Real
/// element set.
const ISS_TLE: &str = concat!(
    "ISS (ZARYA)\n",
    "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9003\n",
    "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299357\n",
);

/// ISS-only simulation: the shared core (clock + celestial sphere) via
/// composition, plus this scenario's single tracked satellite.
pub struct IssSimulation {
    simulation: SimulationState,
    satellites: Vec<Satellite>,
}

impl IssSimulation {
    fn new() -> Self {
        let satellites = vec![Satellite::from_tle(ISS_TLE)];
        let epoch = satellites.first().expect("TLE present").epoch();
        Self {
            simulation: SimulationState::new(epoch),
            satellites,
        }
    }
}

impl Simulation for IssSimulation {
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

/// Builds the ISS simulation and hands off to the winit event loop. Blocks
/// until the window closes.
pub fn run() {
    // Seed satkit's global state (embedded ephemeris + EOP table) before
    // anything else: IssSimulation::new below builds the CelestialSphere
    // (which reads the ephemeris) and the satellite parses a TLE. Doing it
    // here keeps satkit fully offline and data-dir-free.
    simulation::init();

    application::run(ApplicationState::new(IssSimulation::new()));
}
