//! ISS + Hubble scenario: track the International Space Station and the Hubble
//! Space Telescope from their shared ~2024-001.5 TLE epoch. This is the original
//! default scene, now expressed as a named scenario (CLI:
//! `globe-experiment scenario iss_and_hubble`).

use crate::application::{self, ApplicationState};
use crate::simulation::satellite::Satellite;
use crate::simulation::{self, SimulationState};

// This scenario's tracked-object TLEs, inlined as source literals. Unlike the
// textures/ephemeris/EOP (build-downloaded straight into `OUT_DIR` and baked
// into the binary), these small element sets live directly in source so a fresh
// checkout needs no data file. The lines are column-sensitive TLE format (each
// element line is exactly 69 chars) - keep the exact spacing. `concat!` keeps
// source indentation out of the string. satkit parses by column and does not
// verify the trailing checksum digit. `run` below assembles the tracked array
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

/// Builds the tracked-object list, the simulation state, and the application,
/// then hands off to the winit event loop. Blocks until the window closes.
pub fn run() {
    // Seed satkit's global state (embedded ephemeris + EOP table) before
    // anything else: SimulationState::new below builds the Sky (which reads the
    // ephemeris) and the satellites parse TLEs. Doing it here keeps satkit fully
    // offline and data-dir-free.
    simulation::init();

    // Assemble the tracked objects and hand them to the simulation. The clock
    // starts at the first satellite's TLE epoch, so order matters: the primary
    // object (ISS) goes first.
    let satellites = vec![Satellite::from_tle(ISS_TLE), Satellite::from_tle(HST_TLE)];

    let simulation = SimulationState::new(satellites);
    application::run(ApplicationState::new(simulation));
}
