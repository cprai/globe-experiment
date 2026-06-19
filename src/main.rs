mod application;
mod earth;
mod renderer;
mod simulation;
mod ui;

use application::ApplicationState;
use simulation::SimulationState;
use simulation::satellite::{self, Satellite};

fn main() {
    // Seed satkit's global state (embedded ephemeris + EOP table) before
    // anything else (SimulationState::new below builds the Sky, which reads the
    // ephemeris, and the satellites parse TLEs). Doing it here keeps satkit
    // fully offline and data-dir-free.
    simulation::init();

    // Assemble the tracked objects here and hand them to the simulation. The
    // clock starts at the first satellite's TLE epoch, so order matters: the
    // primary object goes first.
    let satellites = vec![
        Satellite::from_tle(satellite::ISS_TLE),
        Satellite::from_tle(satellite::HST_TLE),
    ];

    let simulation = SimulationState::new(satellites);
    application::run(ApplicationState::new(simulation));
}
