mod application;
mod earth;
mod moon;
mod renderer;
mod scenarios;
mod simulation;
mod snapshot;
mod ui;

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

// Deriving `Parser` directly on an enum makes each variant a top-level
// subcommand - no wrapper struct or separate `Subcommand` enum needed. (Switch
// to a struct with a `#[command(subcommand)]` field only if global flags shared
// across all subcommands are ever wanted.) The `///` doc comment below is the
// user-facing `about` text, so keep it free of implementation notes like this.
/// Globe: an astronomically-accurate satellite simulation tool. The CLI either
/// runs a past scenario in an interactive window (`scenario`) or renders a
/// single frame to an image file (`render`); the actual setup lives in the
/// `scenarios` / `snapshot` modules. `main` does nothing but parse args and
/// dispatch.
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

    /// Render a single frame to an image file and exit (no window, no UI). The
    /// `--scene` JSON fixes the celestial positions (its `simulation.datetime`)
    /// and the view (its `camera`), and may carry mock `ui` panels to overlay;
    /// the frame is written to --output as a PNG. Intended for visually
    /// debugging rendering and UI changes.
    ///
    /// NOTE: unlike `scenario`, the datetime is NOT range-checked against the
    /// bundled Earth-orientation (EOP) data - times outside the bundled range
    /// silently degrade rather than erroring. Use a past, in-range datetime for
    /// an accurate frame.
    Render {
        /// JSON scene: `{"simulation": {"datetime": ...}, "camera":
        /// {"longitude", "latitude", "distance" (km), "tilt"}, "ui":
        /// [panels]}`. `ui` is optional (omit for a globe-only frame).
        /// See `snapshot::SceneSpec` / `ui::UiPanel`. Unknown keys
        /// are rejected.
        #[arg(long)]
        scene: String,
        /// Output image width in pixels.
        #[arg(long, default_value_t = 1920)]
        width: u32,
        /// Output image height in pixels.
        #[arg(long, default_value_t = 1080)]
        height: u32,
        /// Path to write the PNG.
        #[arg(long)]
        output: PathBuf,
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
    /// The 2025-03-14 total lunar eclipse (no satellites; framed on the Moon).
    #[value(name = "lunar_eclipse")]
    LunarEclipse,
    /// The 2024-04-08 total solar eclipse (no satellites; framed on the day
    /// side).
    #[value(name = "solar_eclipse")]
    SolarEclipse,
}

fn main() {
    match Cli::parse() {
        Cli::Scenario { name: Some(name) } => match name {
            ScenarioName::Iss => scenarios::iss::run(),
            ScenarioName::IssAndHubble => scenarios::iss_and_hubble::run(),
            ScenarioName::LunarEclipse => scenarios::lunar_eclipse::run(),
            ScenarioName::SolarEclipse => scenarios::solar_eclipse::run(),
        },
        // Bare `scenario` with no name: list what's available instead of erroring.
        Cli::Scenario { name: None } => list_scenarios(),
        Cli::Render {
            scene,
            width,
            height,
            output,
        } => snapshot::run(snapshot::RenderParams {
            scene,
            width,
            height,
            output,
        }),
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
