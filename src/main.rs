mod engine;
mod scenes;

use std::path::PathBuf;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, ValueEnum};

// Deriving `Parser` directly on an enum makes each variant a top-level
// subcommand - no wrapper struct or separate `Subcommand` enum needed. (Switch
// to a struct with a `#[command(subcommand)]` field only if global flags shared
// across all subcommands are ever wanted.) The `///` doc comment below is the
// user-facing `about` text, so keep it free of implementation notes like this.
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
    /// Run a named scene, e.g. `scene iss_and_hubble`. Omit the name to
    /// list the available scenes. The Python-paneled scenes additionally
    /// take the path to their panel script, e.g. `scene manual_control_py
    /// scenes/manual_control_py.py`.
    Scene {
        /// Which scene to simulate. If omitted, the available scenes are
        /// listed instead of running one. `Option` (not a required positional)
        /// is what makes the bare `scene` invocation valid.
        #[arg(value_enum)]
        name: Option<SceneName>,
        /// Path to the scene's Python panel script. Required by - and only
        /// valid for - the `*_py` scenes; clap cannot tie a positional to
        /// individual `ValueEnum` variants, so `Option` here and the pairing
        /// is enforced in `main` (with clap-styled errors).
        script: Option<PathBuf>,
    },
}

/// The set of available scenes. Each variant maps to a module under
/// `scenes`; the `value(name = ...)` keeps the CLI token snake_case so the
/// command reads `scene iss_and_hubble`. The `///` per-variant docs are the
/// help text clap shows (and that `list_scenes` prints).
#[derive(Clone, ValueEnum)]
enum SceneName {
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
    /// `manual_control` with its UI panels produced by the Python script at
    /// the given path, e.g. `scenes/manual_control_py.py` (edit + relaunch,
    /// no rebuild).
    #[value(name = "manual_control_py")]
    ManualControlPy,
    /// The 2024-04-08 total solar eclipse (no satellites; framed on the day
    /// side).
    #[value(name = "solar_eclipse")]
    SolarEclipse,
    /// The whole solar system: fly the camera to and orbit any of Terra, the
    /// Luna, or the seven planets (no satellites).
    #[value(name = "solar_system")]
    SolarSystem,
    /// `solar_system` with its UI panels produced by the Python script at
    /// the given path, e.g. `scenes/solar_system_py.py` (edit + relaunch,
    /// no rebuild).
    #[value(name = "solar_system_py")]
    SolarSystemPy,
}

fn main() {
    match Cli::parse() {
        Cli::Scene {
            name: Some(name),
            script,
        } => match name {
            // The Python-paneled scenes are the only consumers of the script
            // positional; everything else must run without one.
            SceneName::ManualControlPy => {
                scenes::manual_control_py::run(require_script(&name, script));
            }
            SceneName::SolarSystemPy => {
                scenes::solar_system_py::run(require_script(&name, script));
            }
            _ => {
                reject_script(&name, &script);
                match name {
                    SceneName::Iss => scenes::iss::run(),
                    SceneName::IssAndHubble => scenes::iss_and_hubble::run(),
                    SceneName::LunarEclipse => scenes::lunar_eclipse::run(),
                    SceneName::ManualControl => scenes::manual_control::run(),
                    SceneName::SolarEclipse => scenes::solar_eclipse::run(),
                    SceneName::SolarSystem => scenes::solar_system::run(),
                    SceneName::ManualControlPy | SceneName::SolarSystemPy => unreachable!(),
                }
            }
        },
        // Bare `scene` with no name: list what's available instead of
        // erroring. (A lone path can't reach here - positionals fill in
        // order, so a scriptless invocation is also nameless.)
        Cli::Scene { name: None, .. } => list_scenes(),
    }
}

/// The CLI token for a scene variant, for error messages.
fn scene_token(name: &SceneName) -> String {
    name.to_possible_value()
        .expect("scene variants are not skipped")
        .get_name()
        .to_owned()
}

/// Unwraps the script positional for a `*_py` scene, exiting with a
/// clap-styled missing-argument error (usage line included) when absent.
fn require_script(name: &SceneName, script: Option<PathBuf>) -> PathBuf {
    script.unwrap_or_else(|| {
        Cli::command()
            .error(
                ErrorKind::MissingRequiredArgument,
                format!(
                    "scene `{}` requires the path to its Python panel script \
                     (e.g. `scene {0} scenes/{0}.py`)",
                    scene_token(name)
                ),
            )
            .exit()
    })
}

/// Rejects a script positional handed to a non-Python scene - silently
/// ignoring it would look like the script was being used.
fn reject_script(name: &SceneName, script: &Option<PathBuf>) {
    if script.is_some() {
        Cli::command()
            .error(
                ErrorKind::ArgumentConflict,
                format!(
                    "scene `{}` is not Python-paneled and takes no script path",
                    scene_token(name)
                ),
            )
            .exit()
    }
}

/// Prints the available scenes (name + help) to stdout, driven off the
/// `SceneName` `ValueEnum` so it can never drift out of sync with the actual
/// variants.
fn list_scenes() {
    println!("Available scenes (run with `scene <name>`):");
    for variant in SceneName::value_variants() {
        // Every `ValueEnum` variant with a CLI token yields a `PossibleValue`.
        let value = variant
            .to_possible_value()
            .expect("scene variants are not skipped");
        match value.get_help() {
            Some(help) => println!("  {:<16} {help}", value.get_name()),
            None => println!("  {}", value.get_name()),
        }
    }
}
