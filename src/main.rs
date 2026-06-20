mod application;
mod earth;
mod renderer;
mod scenarios;
mod simulation;
mod ui;

use clap::{Parser, Subcommand, ValueEnum};

/// Globe: an astronomically-accurate satellite simulation tool. The CLI selects
/// which past scenario to run; all the actual setup lives in the `scenarios`
/// module. `main` does nothing but parse args and dispatch.
#[derive(Parser)]
#[command(name = "globe-experiment", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a named scenario, e.g. `scenario iss_and_hubble`.
    Scenario {
        /// Which scenario to simulate.
        #[arg(value_enum)]
        name: ScenarioName,
    },
}

/// The set of available scenarios. Each variant maps to a module under
/// `scenarios`; the `value(name = ...)` keeps the CLI token snake_case so the
/// command reads `scenario iss_and_hubble`.
#[derive(Clone, ValueEnum)]
enum ScenarioName {
    #[value(name = "iss_and_hubble")]
    IssAndHubble,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Scenario { name } => match name {
            ScenarioName::IssAndHubble => scenarios::iss_and_hubble::run(),
        },
    }
}
