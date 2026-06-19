mod application;
mod earth;
mod renderer;
mod simulation;
mod ui;

use application::ApplicationState;
use simulation::SimulationState;

fn main() {
    // Seed satkit's global state (embedded ephemeris + EOP table) before
    // anything else (SimulationState::new below builds the Sky, which reads the
    // ephemeris). Doing it here keeps satkit fully offline and data-dir-free.
    simulation::init();

    let simulation = SimulationState::new();
    application::run(ApplicationState::new(simulation));
}
