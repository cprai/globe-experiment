mod engine;
mod scenes;

use clap::{Parser, Subcommand};

// Deriving `Parser` directly on an enum makes each variant a top-level
// subcommand - no wrapper struct or separate top-level `Subcommand` enum
// needed. (Switch to a struct with a `#[command(subcommand)]` field only if
// global flags shared across all subcommands are ever wanted.) The `///` doc
// comment below is the user-facing `about` text, so keep it free of
// implementation notes like this.
/// Solar System: an astronomically-accurate solar-system renderer with
/// satellite tracking (past scenes only). This CLI runs a past scene in
/// an interactive window (`scene`); the actual setup lives in the
/// `scenes` modules. `main` does nothing but parse args and dispatch.
/// (Single-frame image rendering lives in the separate `headless` binary.)
//
// No explicit `name`: clap defaults the command name to `CARGO_PKG_NAME` (the
// Cargo.toml package name, "globe-experiment"), so there's no string to keep in
// sync. `version`/`about` likewise come from the package metadata / doc comment.
#[derive(Parser)]
#[command(version, about)]
enum Cli {
    /// Run a named scene, e.g. `scene iss_and_hubble`. Each scene is its own
    /// subcommand declaring exactly the arguments it takes (`scene <name>
    /// --help` shows them; only the Python-paneled scenes take `--script`).
    /// Omit the name to list the available scenes.
    // `arg_required_else_help` makes the bare `scene` invocation print this
    // help - the scene list - instead of clap's terse missing-subcommand
    // error.
    #[command(arg_required_else_help = true)]
    Scene {
        #[command(subcommand)]
        scene: SceneCommand,
    },
}

/// The set of available scenes, one subcommand per scene so each declares
/// its own arguments: the per-scene `Args` structs live beside their scenes
/// in `scenes/*` (only the `*_py` scenes have a `--script`, and clap itself
/// enforces the pairing - a non-Python scene rejects it as an unknown
/// argument). The `command(name = ...)` keeps the CLI tokens snake_case
/// (clap would kebab-case the variant names) so the command reads
/// `scene iss_and_hubble`. The `///` per-variant docs are the help text
/// clap shows.
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
    /// `manual_control` with its UI panels produced by the Python script at
    /// `--script` (edit + relaunch, no rebuild).
    #[command(name = "manual_control_py")]
    ManualControlPy(scenes::manual_control_py::Args),
    /// The 2024-04-08 total solar eclipse (no satellites; framed on the day
    /// side).
    #[command(name = "solar_eclipse")]
    SolarEclipse(scenes::solar_eclipse::Args),
    /// The whole solar system: fly the camera to and orbit any of Terra, the
    /// Luna, or the seven planets (no satellites).
    #[command(name = "solar_system")]
    SolarSystem(scenes::solar_system::Args),
    /// `solar_system` with its UI panels produced by the Python script at
    /// `--script` (edit + relaunch, no rebuild).
    #[command(name = "solar_system_py")]
    SolarSystemPy(scenes::solar_system_py::Args),
}

fn main() {
    match Cli::parse() {
        Cli::Scene { scene } => match scene {
            SceneCommand::Iss(args) => scenes::iss::run(args),
            SceneCommand::IssAndHubble(args) => scenes::iss_and_hubble::run(args),
            SceneCommand::LunarEclipse(args) => scenes::lunar_eclipse::run(args),
            SceneCommand::ManualControl(args) => scenes::manual_control::run(args),
            SceneCommand::ManualControlPy(args) => scenes::manual_control_py::run(args),
            SceneCommand::SolarEclipse(args) => scenes::solar_eclipse::run(args),
            SceneCommand::SolarSystem(args) => scenes::solar_system::run(args),
            SceneCommand::SolarSystemPy(args) => scenes::solar_system_py::run(args),
        },
    }
}
