mod engine;
mod scenes;

use clap::{Parser, Subcommand};

/// Solar System: an astronomically-accurate solar-system renderer with
/// satellite tracking (past scenes only). This CLI runs a past scene in
/// an interactive window (`scene`). (Single-frame image rendering lives
/// in the separate `headless` binary.)
#[derive(Parser)]
#[command(version, about)]
enum Cli {
    /// Run a named scene, e.g. `scene iss_and_hubble`. Each scene is its own
    /// subcommand declaring exactly the arguments it takes (`scene <name>
    /// --help` shows them). Omit the name to list the available scenes.
    // arg_required_else_help: bare `scene` prints the scene list instead of
    // clap's terse missing-subcommand error.
    #[command(arg_required_else_help = true)]
    Scene {
        #[command(subcommand)]
        scene: SceneCommand,
    },
}

/// One subcommand per scene, so each declares its own arguments (the
/// per-scene `Args` structs live beside their scenes). Explicit
/// `command(name = ...)` keeps the CLI tokens snake_case (clap would
/// kebab-case the variant names).
#[derive(Subcommand)]
enum SceneCommand {
    /// The International Space Station only.
    #[command(name = "iss")]
    Iss(scenes::iss::Args),
    /// The International Space Station and the Hubble Space Telescope.
    #[command(name = "iss_and_hubble")]
    IssAndHubble(scenes::iss_and_hubble::Args),
    /// The 2025-03-14 total lunar eclipse (no satellites; framed on Luna).
    #[command(name = "lunar_eclipse")]
    LunarEclipse(scenes::lunar_eclipse::Args),
    /// A manually-controlled satellite starting from the ISS orbit: hold the
    /// Burns panel keys to thrust and reshape the orbit.
    #[command(name = "manual_control")]
    ManualControl(scenes::manual_control::Args),
    /// The 2024-04-08 total solar eclipse (no satellites; framed on the day
    /// side).
    #[command(name = "solar_eclipse")]
    SolarEclipse(scenes::solar_eclipse::Args),
    /// The whole solar system: fly the camera to and orbit any of Terra, the
    /// Luna, or the seven planets (no satellites).
    #[command(name = "solar_system")]
    SolarSystem(scenes::solar_system::Args),
}

fn main() {
    match Cli::parse() {
        Cli::Scene { scene } => match scene {
            SceneCommand::Iss(args) => scenes::iss::run(args),
            SceneCommand::IssAndHubble(args) => scenes::iss_and_hubble::run(args),
            SceneCommand::LunarEclipse(args) => scenes::lunar_eclipse::run(args),
            SceneCommand::ManualControl(args) => scenes::manual_control::run(args),
            SceneCommand::SolarEclipse(args) => scenes::solar_eclipse::run(args),
            SceneCommand::SolarSystem(args) => scenes::solar_system::run(args),
        },
    }
}
