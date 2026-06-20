//! ISS-only scenario: track the International Space Station from its
//! ~2024-001.5 TLE epoch. Same as `iss_and_hubble` but with Hubble omitted, so
//! a single marker renders (CLI: `globe-experiment scenario iss`).

use crate::application::{self, ApplicationState};
use crate::simulation::satellite::Satellite;
use crate::simulation::{self, SimulationState};

// This scenario's tracked-object TLE, inlined as a source literal. Unlike the
// textures/ephemeris/EOP (build-downloaded straight into `OUT_DIR` and baked
// into the binary), this small element set lives directly in source so a fresh
// checkout needs no data file. The lines are column-sensitive TLE format (each
// element line is exactly 69 chars) - keep the exact spacing. `concat!` keeps
// source indentation out of the string. satkit parses by column and does not
// verify the trailing checksum digit. `run` below assembles the tracked array
// from this via `Satellite::from_tle`. (Deliberately duplicated from
// `iss_and_hubble.rs` - each scenario owns its own TLE data.)

/// The International Space Station (ISS / ZARYA), epoch 2024-001.5. Real
/// element set.
const ISS_TLE: &str = concat!(
    "ISS (ZARYA)\n",
    "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9003\n",
    "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299357\n",
);

/// Builds the tracked-object list, the simulation state, and the application,
/// then hands off to the winit event loop. Blocks until the window closes.
pub fn run() {
    // Seed satkit's global state (embedded ephemeris + EOP table) before
    // anything else: SimulationState::new below builds the Sky (which reads the
    // ephemeris) and the satellites parse TLEs. Doing it here keeps satkit fully
    // offline and data-dir-free.
    simulation::init();

    // The single tracked object; the clock starts at its TLE epoch.
    let satellites = vec![Satellite::from_tle(ISS_TLE)];

    let simulation = SimulationState::new(satellites);
    application::run(ApplicationState::new(simulation));
}
