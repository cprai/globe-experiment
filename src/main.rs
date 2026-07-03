mod application;
mod camera;
mod luna;
mod planet;
mod renderer;
mod scenarios;
mod simulation;
mod terra;
mod ui;

use clap::{Parser, ValueEnum};

// Deriving `Parser` directly on an enum makes each variant a top-level
// subcommand - no wrapper struct or separate `Subcommand` enum needed. (Switch
// to a struct with a `#[command(subcommand)]` field only if global flags shared
// across all subcommands are ever wanted.) The `///` doc comment below is the
// user-facing `about` text, so keep it free of implementation notes like this.
/// Solar System: an astronomically-accurate solar-system renderer with
/// satellite tracking (past scenarios only). This CLI runs a past scenario in
/// an interactive window (`scenario`); the actual setup lives in the
/// `scenarios` modules. `main` does nothing but parse args and dispatch.
/// (Single-frame image rendering lives in the separate `headless` binary.)
//
// No explicit `name`: clap defaults the command name to `CARGO_PKG_NAME` (the
// Cargo.toml package name, "globe-experiment"), so there's no string to keep in
// sync. `version`/`about` likewise come from the package metadata / doc comment.
#[derive(Parser)]
#[command(version, about)]
enum Cli {
    /// Run a named scenario, e.g. `scenario iss_and_hubble`. Omit the name to
    /// list the available scenarios.
    Scenario {
        /// Which scenario to simulate. If omitted, the available scenarios are
        /// listed instead of running one. `Option` (not a required positional)
        /// is what makes the bare `scenario` invocation valid.
        #[arg(value_enum)]
        name: Option<ScenarioName>,
    },
}

/// The set of available scenarios. Each variant maps to a module under
/// `scenarios`; the `value(name = ...)` keeps the CLI token snake_case so the
/// command reads `scenario iss_and_hubble`. The `///` per-variant docs are the
/// help text clap shows (and that `list_scenarios` prints).
#[derive(Clone, ValueEnum)]
enum ScenarioName {
    /// The International Space Station only.
    #[value(name = "iss")]
    Iss,
    /// The International Space Station and the Hubble Space Telescope.
    #[value(name = "iss_and_hubble")]
    IssAndHubble,
    /// The 2025-03-14 total lunar eclipse (no satellites; framed on Luna).
    #[value(name = "lunar_eclipse")]
    LunarEclipse,
    /// A manually-controlled satellite starting from the ISS orbit: hold the
    /// Burns panel keys to thrust and reshape the orbit.
    #[value(name = "manual_control")]
    ManualControl,
    /// The 2024-04-08 total solar eclipse (no satellites; framed on the day
    /// side).
    #[value(name = "solar_eclipse")]
    SolarEclipse,
    /// The whole solar system: fly the camera to and orbit any of Terra, the
    /// Luna, or the seven planets (no satellites).
    #[value(name = "solar_system")]
    SolarSystem,
}

fn main() {
    match Cli::parse() {
        Cli::Scenario { name: Some(name) } => match name {
            ScenarioName::Iss => scenarios::iss::run(),
            ScenarioName::IssAndHubble => scenarios::iss_and_hubble::run(),
            ScenarioName::LunarEclipse => scenarios::lunar_eclipse::run(),
            ScenarioName::ManualControl => scenarios::manual_control::run(),
            ScenarioName::SolarEclipse => scenarios::solar_eclipse::run(),
            ScenarioName::SolarSystem => scenarios::solar_system::run(),
        },
        // Bare `scenario` with no name: list what's available instead of erroring.
        Cli::Scenario { name: None } => list_scenarios(),
    }
}

/// Prints the available scenarios (name + help) to stdout, driven off the
/// `ScenarioName` `ValueEnum` so it can never drift out of sync with the actual
/// variants.
fn list_scenarios() {
    println!("Available scenarios (run with `scenario <name>`):");
    for variant in ScenarioName::value_variants() {
        // Every `ValueEnum` variant with a CLI token yields a `PossibleValue`.
        let value = variant
            .to_possible_value()
            .expect("scenario variants are not skipped");
        match value.get_help() {
            Some(help) => println!("  {:<16} {help}", value.get_name()),
            None => println!("  {}", value.get_name()),
        }
    }
}
