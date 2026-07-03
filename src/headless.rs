//! The `headless` binary: renders one frame to a PNG and exits, with no winit
//! window or input. The frame is positioned by an explicit datetime (which
//! fixes the celestial positions) and explicit camera parameters, and written
//! to a caller-specified path - intended for visual debugging of rendering
//! changes, e.g. by an agent that opens the image. This is the headless
//! analogue of a scenario's `run` in the main binary.
//!
//! This bin root declares its own module tree (the two binaries share source
//! files, not a lib crate): only the winit-free shared modules plus
//! `offscreen`, and none of the windowed ones (`application`, `scenarios`).
//! The shared modules also carry items only the main binary calls, hence the
//! crate-level `allow(dead_code)` - the main binary's tree keeps full
//! dead-code checking for them.
//!
//! The whole scene is a single `--scene` JSON ([`SceneSpec`]): a `simulation`
//! section (the datetime), a `camera` section, and an optional `ui` section of
//! mock panels (see [`crate::ui::UiPanel`]) to overlay - so an agent can
//! debug rendering *and* UI layouts without a live window. The output target
//! (width/height/path) stays on the CLI, not in the JSON. When `ui` is present
//! the mock is run through the same `ui::control_panel` path as the live app
//! and composited by [`OffscreenRenderer`]; this binary is the headless
//! analogue of the windowed egui driving in the main binary's `application`.
//!
//! IMPORTANT: unlike scenarios (see the "Scenarios & valid time range" rules in
//! `CLAUDE.md`), the headless binary does **not** range-check the datetime
//! against the bundled Earth-orientation (EOP) data. The caller owns the time,
//! and an out-of-range datetime silently degrades rather than erroring: before
//! ~1962-01-01 satkit falls back to zero EOP, and past the last bundled EOP
//! entry it constant-extrapolates. Choosing an in-range past datetime for an
//! accurate frame is the caller's responsibility. This deliberate deviation is
//! also documented in `.claude/rules/scenarios.md` and the `analyze-render`
//! skill.

// Shared modules included by both bin trees; scenario/windowed-only items in
// them are intentionally unused here (see the module doc above).
#![allow(dead_code)]

mod camera;
mod luna;
mod offscreen;
mod planet;
mod renderer;
mod simulation;
mod terra;
mod ui;

use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use clap::Parser;
use satkit::Instant;

use crate::camera::Camera;
use crate::offscreen::{MAX_FRAME_DIMENSION, OffscreenRenderer};
use crate::renderer::UiFrame;
use crate::simulation::celestial_sphere::CelestialSphere;
use crate::simulation::{CameraTarget, CelestialBody, RenderState};
use crate::ui::{PanelSet, UiPanel};

/// Renders a single frame of the astronomically-accurate solar system to a PNG
/// and exits (no window, no interactivity). The `--scene` JSON fixes the
/// celestial positions (its `simulation.datetime`) and the view (its `camera`),
/// and may carry mock `ui` panels to overlay; the frame is written to --output.
/// Intended for visually debugging rendering and UI changes.
///
/// NOTE: the datetime is NOT range-checked against the bundled
/// Earth-orientation (EOP) data - times outside the bundled range silently
/// degrade rather than erroring. Use a past, in-range datetime for an accurate
/// frame.
//
// The binary does exactly one thing, so the CLI is flat flags (no subcommand).
// `name` is set explicitly: the derive default is CARGO_PKG_NAME
// ("globe-experiment"), which would mislabel the help text.
#[derive(Parser)]
#[command(name = "headless", version, about)]
struct Cli {
    /// JSON scene: `{"simulation": {"datetime": ...}, "camera":
    /// {"longitude", "latitude", "distance" (km), "tilt", "target":
    /// "terra"|"luna"|"mercury"|...|"neptune"}, "ui": [panels]}`.
    /// `camera.target` and `ui` are optional (target defaults to "terra";
    /// omit `ui` for a body-only frame). See `SceneSpec` / `ui::UiPanel`.
    /// Unknown keys are rejected.
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
}

fn main() {
    run(Cli::parse());
}

/// The full render scene, deserialized from the `--scene` JSON. Divides the
/// celestial/simulation state from the camera and the optional UI overlay.
/// `deny_unknown_fields` so a misspelled key (e.g. `latitde`) fails loudly
/// rather than being silently dropped - the scene is hand-authored by agents.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneSpec {
    simulation: SimulationSpec,
    camera: CameraSpec,
    /// Mock UI panels to overlay; omit (or empty) for a body-only frame.
    #[serde(default)]
    ui: Vec<UiPanel>,
}

/// The `simulation` section: the celestial-state driver. Just the datetime
/// today (camera lives in its own section); a struct so it can grow.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SimulationSpec {
    /// RFC3339 UTC instant for the celestial positions (e.g.
    /// `2024-01-15T12:30:00Z`).
    datetime: String,
}

/// The `camera` section: the orbital camera placement, mirroring the `Camera`
/// fields the windowed path drives.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CameraSpec {
    /// Inertial look longitude, degrees.
    longitude: f32,
    /// Inertial look latitude, degrees.
    latitude: f32,
    /// Eye distance to the look-at point, kilometers.
    distance: f32,
    /// Tilt off nadir, degrees (0 looks straight down).
    tilt: f32,
    /// Which body the camera orbits: `"terra"` (default) or `"luna"`. The
    /// distance/tilt are relative to the chosen body's surface, so framing the
    /// Luna usually wants a much smaller `distance` than Terra.
    #[serde(default)]
    target: CameraTargetSpec,
}

/// The orbit body for the render camera. Mirrors the runtime [`CameraTarget`]
/// kinds, but center-free: a body's world center is filled from the ephemeris
/// at render time (the JSON only names the body). Lowercase JSON tokens
/// (`"terra"`, `"luna"`, `"mars"`, ...); defaults to Terra so existing scenes
/// are unchanged. A planet target renders with a floating origin (see
/// `CameraTarget::render_origin`).
#[derive(serde::Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum CameraTargetSpec {
    #[default]
    Terra,
    Luna,
    Mercury,
    Venus,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
}

impl CameraTargetSpec {
    /// The celestial-body identity this token names.
    fn body(self) -> CelestialBody {
        match self {
            CameraTargetSpec::Terra => CelestialBody::TERRA,
            CameraTargetSpec::Luna => CelestialBody::LUNA,
            CameraTargetSpec::Mercury => CelestialBody::Mercury,
            CameraTargetSpec::Venus => CelestialBody::Venus,
            CameraTargetSpec::Mars => CelestialBody::Mars,
            CameraTargetSpec::Jupiter => CelestialBody::Jupiter,
            CameraTargetSpec::Saturn => CelestialBody::Saturn,
            CameraTargetSpec::Uranus => CelestialBody::Uranus,
            CameraTargetSpec::Neptune => CelestialBody::Neptune,
        }
    }

    /// Lowercase body name, for the summary line.
    fn name(self) -> &'static str {
        match self {
            CameraTargetSpec::Terra => "terra",
            CameraTargetSpec::Luna => "luna",
            CameraTargetSpec::Mercury => "mercury",
            CameraTargetSpec::Venus => "venus",
            CameraTargetSpec::Mars => "mars",
            CameraTargetSpec::Jupiter => "jupiter",
            CameraTargetSpec::Saturn => "saturn",
            CameraTargetSpec::Uranus => "uranus",
            CameraTargetSpec::Neptune => "neptune",
        }
    }
}

/// Renders one frame per the parsed CLI, writes it to `params.output` as a
/// PNG, and prints a summary to stdout. Exits the process with a nonzero
/// status on a usage error (bad datetime, bad dimensions, or write failure).
fn run(params: Cli) {
    // Parse the scene JSON first (strict: a misspelled key errors, see
    // SceneSpec). Everything below reads from it.
    let scene: SceneSpec = serde_json::from_str(&params.scene)
        .unwrap_or_else(|error| fail(&format!("invalid --scene JSON: {error}")));

    // Parse the datetime. No EOP range check - see the module note.
    let time = match parse_rfc3339(&scene.simulation.datetime) {
        Ok(time) => time,
        Err(message) => fail(&format!(
            "invalid simulation.datetime '{}': {message} (expected RFC3339 UTC, e.g. 2024-01-15T12:30:00Z)",
            scene.simulation.datetime
        )),
    };

    if params.width == 0 || params.height == 0 {
        fail("--width and --height must be greater than 0");
    }
    if params.width > MAX_FRAME_DIMENSION || params.height > MAX_FRAME_DIMENSION {
        fail(&format!(
            "--width/--height must be <= {MAX_FRAME_DIMENSION}, got {}x{}",
            params.width, params.height
        ));
    }

    // Seed satkit's global state (embedded ephemeris + EOP) before any
    // celestial-sphere or instant math, exactly like a scenario.
    simulation::init();

    // Build the frame directly from the celestial sphere + camera: render mode
    // has no clock, no tracked satellites, and no scenario struct. The camera
    // math is identical to the windowed path (see `application`'s redraw).
    let celestial = CelestialSphere::at(&time);
    // Camera rig uses the equatorial frame (`star_rot_inv`); the star texture
    // is sampled with the galactic-corrected `star_tex_rot_inv` below.
    let celestial_to_world = celestial.star_rot_inv.transpose();

    // Resolve the orbit body by identity; its center (and the render origin) is
    // looked up from the celestial sphere where needed. The distance clamp uses
    // the chosen target's radius-scaled limits.
    let target = CameraTarget::Body(scene.camera.target.body());
    let camera = Camera {
        longitude: scene.camera.longitude,
        latitude: scene.camera.latitude,
        distance: scene.camera.distance,
        tilt: scene.camera.tilt,
        target,
    };
    let distance = camera.clamp_distance(scene.camera.distance);
    let camera = Camera { distance, ..camera };
    let (eye, look_at, up) = camera.world_rig(&celestial, celestial_to_world);

    let render = RenderState {
        time,
        camera_target: target,
        camera_pos: eye,
        camera_look_at: look_at,
        camera_up: up,
        // Bodies only: render mode tracks no satellites, so no markers. The
        // renderer derives Sol/Luna/planets from `time`.
        markers: Vec::new(),
    };

    // Build the optional mock-UI overlay (empty `ui` = body-only frame). The
    // panels were already validated by the scene parse above.
    let ui_frame =
        (!scene.ui.is_empty()).then(|| build_ui_frame(scene.ui, params.width, params.height));

    let image = OffscreenRenderer::new(params.width, params.height).render(&render, ui_frame);

    if let Err(error) = image.save(&params.output) {
        fail(&format!(
            "failed to write {}: {error}",
            params.output.display()
        ));
    }

    print_summary(&params, &scene.camera, &time, distance);
}

/// Parses an RFC3339 UTC datetime into a satkit [`Instant`] via `humantime`.
fn parse_rfc3339(text: &str) -> Result<Instant, String> {
    let system_time = humantime::parse_rfc3339(text).map_err(|error| error.to_string())?;
    let unix_seconds = system_time
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "datetime is before the Unix epoch (1970-01-01)".to_string())?
        .as_secs_f64();
    // `from_unixtime` is leap-second-correct (it re-adds the leap seconds Unix
    // time omits), so this lands on the intended UTC instant.
    Ok(Instant::from_unixtime(unix_seconds))
}

/// Runs the mock `panels` (the scene's `ui` section) through egui once to
/// produce a render-ready [`UiFrame`] - the headless analogue of the windowed
/// egui driving in `application`. The mock is rendered via the same
/// `ui::control_panel` the live app uses (taffy lays out the rows; the JSON
/// carries no pixel coordinates), so the overlay is faithful. The egui screen
/// is sized to the output in points at 1.0 pixels-per-point, so mock panel
/// sizes land in output pixels. The panels were already validated by the scene
/// parse in [`run`].
fn build_ui_frame(panels: Vec<UiPanel>, width: u32, height: u32) -> UiFrame {
    let mut mock = PanelSet { panels };

    let ctx = egui::Context::default();
    // Same theme the windowed app installs, so a mock overlay is faithful.
    ui::install_theme(&ctx);
    let raw_input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(width as f32, height as f32),
        )),
        ..Default::default()
    };

    // egui builds its font atlas and measures text lazily on the first pass, so
    // a single pass tessellates to nothing. (The live app never sees this - it
    // runs continuously, settling by frame two.) Run a throwaway warmup pass to
    // load fonts/lay out (this also seeds egui_taffy's layout cache; each
    // run_ui below may internally add a discard pass - install_theme sets
    // max_passes = 2 - so the taffy layout is settled by the second run), then
    // a second pass for the real geometry. egui emits each texture delta
    // exactly once, so the font-atlas allocation arrives on the *warmup*
    // output; merge its texture deltas into the second pass's so the renderer
    // actually gets the atlas the glyph primitives reference.
    let warmup = ctx.run_ui(raw_input.clone(), |ui| {
        ui::control_panel(ui.ctx(), &mut mock)
    });
    let full_output = ctx.run_ui(raw_input, |ui| ui::control_panel(ui.ctx(), &mut mock));

    let mut textures_delta = warmup.textures_delta;
    textures_delta.set.extend(full_output.textures_delta.set);
    textures_delta.free.extend(full_output.textures_delta.free);

    UiFrame {
        primitives: ctx.tessellate(full_output.shapes, full_output.pixels_per_point),
        textures_delta,
        pixels_per_point: full_output.pixels_per_point,
    }
}

/// Prints a concise summary of the rendered frame to stdout: the resolved
/// datetime, the camera, and the output path. Informational only - the
/// headless binary is deliberately silent about EOP range (see the module
/// note).
fn print_summary(params: &Cli, camera: &CameraSpec, time: &Instant, distance: f32) {
    let (year, month, day, hour, minute, second) = time.as_datetime();

    // Note if the supplied distance was clamped into the camera's valid range.
    let clamped = if (distance - camera.distance).abs() > f32::EPSILON {
        format!(" (clamped from {:.1})", camera.distance)
    } else {
        String::new()
    };

    println!("Rendered single frame:");
    println!(
        "  datetime:  {year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{:02} UTC",
        second as i32
    );
    let target = camera.target.name();
    println!(
        "  camera:    orbit {target}, lon {:.3} lat {:.3} deg, distance {:.1} km{clamped}, tilt {:.3} deg",
        camera.longitude, camera.latitude, distance, camera.tilt
    );
    println!(
        "  output:    {} ({}x{})",
        params.output.display(),
        params.width,
        params.height
    );
}

/// Prints a usage error to stderr and exits with a nonzero status.
fn fail(message: &str) -> ! {
    eprintln!("error: {message}");
    std::process::exit(2);
}
