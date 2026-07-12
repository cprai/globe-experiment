//! The `headless` binary: renders one frame to a PNG and exits. The whole
//! scene is a single `--scene` JSON ([`SceneSpec`]): `simulation` (the
//! datetime), `camera`, and an optional `ui` section of mock panels - for
//! visually debugging rendering and UI layouts without a live window. The
//! output target (width/height/path) stays on the CLI.
//!
//! IMPORTANT: unlike scenes, this binary does NOT range-check the datetime
//! against the bundled EOP data - deliberate: the caller owns the time, and
//! an out-of-range datetime silently degrades (zero EOP before ~1962,
//! constant extrapolation past the last entry) rather than erroring.

use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use clap::Parser;
use satkit::Instant;

use engine::camera::PtzCamera;
use engine::offscreen::{MAX_FRAME_DIMENSION, OffscreenRenderer};
use engine::renderer::UiFrame;
use engine::scene::celestial_sphere::CelestialSphere;
use engine::scene::{self, CameraTarget, CelestialBody, RenderState};
use engine::ui::{self, PanelSet, UIDrawable, UiPanel};

/// Renders a single frame of the astronomically-accurate solar system to a PNG
/// and exits (no window, no interactivity). The `--scene` JSON fixes the
/// celestial positions (its `simulation.datetime`) and the view (its `camera`),
/// and may carry mock `ui` panels to overlay; the frame is written to --output.
///
/// NOTE: the datetime is NOT range-checked against the bundled
/// Earth-orientation (EOP) data - times outside the bundled range silently
/// degrade rather than erroring. Use a past, in-range datetime for an accurate
/// frame.
//
// `name` is set explicitly: the derive default is CARGO_PKG_NAME
// ("engine"), which would mislabel the help text.
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

/// The full render scene, deserialized from the `--scene` JSON.
/// `deny_unknown_fields` so a misspelled key fails loudly rather than being
/// silently dropped - the scene is hand-authored by agents.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneSpec {
    simulation: SimulationSpec,
    camera: CameraSpec,
    /// Mock UI panels to overlay; omit (or empty) for a body-only frame.
    #[serde(default)]
    ui: Vec<UiPanel>,
}

/// The `simulation` section: just the datetime today; a struct so it can grow.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SimulationSpec {
    /// RFC3339 UTC instant for the celestial positions (e.g.
    /// `2024-01-15T12:30:00Z`).
    datetime: String,
}

/// The `camera` section: the orbital camera placement, mirroring the
/// `PtzCamera` pose fields.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CameraSpec {
    /// Inertial look longitude, degrees.
    longitude: f64,
    /// Inertial look latitude, degrees.
    latitude: f64,
    /// Eye distance to the look-at point, kilometers.
    distance: f64,
    /// Tilt off nadir, degrees (0 looks straight down).
    tilt: f64,
    /// Which body the camera orbits. Distance/tilt are relative to the chosen
    /// body's surface, so framing Luna usually wants a much smaller
    /// `distance` than Terra.
    #[serde(default)]
    target: CameraTargetSpec,
}

/// The orbit body for the render camera, center-free (the body's center is
/// filled from the ephemeris at render time). Lowercase JSON tokens;
/// defaults to Terra so existing scenes are unchanged.
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
/// PNG, and prints a summary. Exits nonzero on a usage error.
fn run(params: Cli) {
    let scene: SceneSpec = serde_json::from_str(&params.scene)
        .unwrap_or_else(|error| fail(&format!("invalid --scene JSON: {error}")));

    // No EOP range check - see the module note.
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
    // celestial-sphere or instant math, exactly like a scene.
    scene::init();

    // No clock, no tracked bodies, no scene struct: the frame is built directly
    // from the celestial sphere + camera.
    let celestial = CelestialSphere::at(&time);
    // Camera rig uses the equatorial frame (`star_rot_inv`); the star
    // texture uses the galactic-corrected `star_tex_rot_inv`.
    let celestial_to_world = celestial.star_rot_inv.transpose();

    let target = CameraTarget::Body(scene.camera.target.body());
    let camera = PtzCamera::new(
        &target,
        scene.camera.longitude,
        scene.camera.latitude,
        scene.camera.distance,
        scene.camera.tilt,
    );
    let distance = camera.distance;
    let (eye, look_at, up) = camera.world_rig(&target, &celestial, celestial_to_world);

    let render = RenderState {
        time,
        camera_target: target,
        camera_pos: eye,
        camera_look_at: look_at,
        camera_up: up,
        // Celestial bodies only: render mode tracks no bodies.
        tracked_bodies: Vec::new(),
    };

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
    // `from_unixtime` is leap-second-correct (it re-adds the leap seconds
    // Unix time omits), so this lands on the intended UTC instant.
    Ok(Instant::from_unixtime(unix_seconds))
}

/// Runs the mock `panels` through egui once to produce a render-ready
/// [`UiFrame`], via the same `ui::control_panel` and theme the live app
/// uses. The egui screen is sized at 1.0 pixels-per-point, so mock panel
/// sizes land in output pixels.
fn build_ui_frame(panels: Vec<UiPanel>, width: u32, height: u32) -> UiFrame {
    let mut mock = PanelSet { panels };
    let mut drawables = mock.get_drawables();

    let ctx = egui::Context::default();
    ui::install_theme(&ctx);
    let raw_input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(width as f32, height as f32),
        )),
        ..Default::default()
    };

    // Two passes are required: egui builds its font atlas lazily, so a single
    // pass tessellates to nothing - a throwaway warmup pass loads fonts and
    // seeds egui_taffy's layout cache, then the second pass emits the real
    // geometry. egui emits each texture delta exactly once, so the font-atlas
    // allocation arrives on the warmup output; merge its deltas into the
    // second pass's so the renderer gets the atlas the glyphs reference.
    let warmup = ctx.run_ui(raw_input.clone(), |ui| {
        ui::control_panel(ui.ctx(), &mut drawables, &mut mock)
    });
    let full_output = ctx.run_ui(raw_input, |ui| {
        ui::control_panel(ui.ctx(), &mut drawables, &mut mock)
    });

    let mut textures_delta = warmup.textures_delta;
    textures_delta.set.extend(full_output.textures_delta.set);
    textures_delta.free.extend(full_output.textures_delta.free);

    UiFrame {
        primitives: ctx.tessellate(full_output.shapes, full_output.pixels_per_point),
        textures_delta,
        pixels_per_point: full_output.pixels_per_point,
    }
}

/// Prints a concise summary of the rendered frame to stdout.
fn print_summary(params: &Cli, camera: &CameraSpec, time: &Instant, distance: f64) {
    let (year, month, day, hour, minute, second) = time.as_datetime();

    // Note if the supplied distance was clamped into the camera's valid range.
    let clamped = if (distance - camera.distance).abs() > f64::EPSILON {
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
