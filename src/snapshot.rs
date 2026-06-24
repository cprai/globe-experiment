//! Single-frame render mode (the `render` CLI subcommand): draws one frame to a
//! PNG and exits, with no winit window or input. The frame is positioned by an
//! explicit datetime (which fixes the celestial positions) and explicit camera
//! parameters, and written to a caller-specified path - intended for visual
//! debugging of rendering changes, e.g. by an agent that opens the image. This
//! is the headless analogue of a scenario's `run`.
//!
//! An optional `--ui` JSON argument overlays mock UI panels (see
//! [`crate::ui::UiPanelSpec`]) so an agent can debug UI layouts without a live
//! window. The mock is run through the same `ui::control_panel` path as the
//! live app and composited by [`HeadlessRenderer`]; this module is the headless
//! analogue of the windowed egui driving in `application`.
//!
//! IMPORTANT: unlike scenarios (see the "Scenarios & valid time range" rules in
//! `CLAUDE.md`), render mode does **not** range-check the datetime against the
//! bundled Earth-orientation (EOP) data. The caller owns the time, and an
//! out-of-range datetime silently degrades rather than erroring: before
//! ~1962-01-01 satkit falls back to zero EOP, and past the last bundled EOP
//! entry it constant-extrapolates. Choosing an in-range past datetime for an
//! accurate frame is the caller's responsibility. This deliberate deviation is
//! also documented in `.claude/rules/scenarios.md` and the `analyze-render`
//! skill.

use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use satkit::Instant;

use crate::application::Camera;
use crate::renderer::{HeadlessRenderer, MAX_FRAME_DIMENSION, UiFrame};
use crate::simulation::celestial_sphere::CelestialSphere;
use crate::simulation::{self, RenderState};
use crate::ui::{self, MockUi, UiPanelSpec};

/// Parameters for one single-frame render, built by `main` from the parsed CLI.
pub struct RenderParams {
    /// RFC3339 UTC instant for the celestial positions (e.g.
    /// `2024-01-15T12:30:00Z`).
    pub datetime: String,
    /// Inertial look longitude, degrees.
    pub longitude: f32,
    /// Inertial look latitude, degrees.
    pub latitude: f32,
    /// Eye distance to the look-at point, kilometers.
    pub distance_km: f32,
    /// Tilt off nadir, degrees (0 looks straight down).
    pub tilt: f32,
    /// Output image width, pixels.
    pub width: u32,
    /// Output image height, pixels.
    pub height: u32,
    /// Path to write the PNG.
    pub output: PathBuf,
    /// Optional JSON array of mock UI panels to overlay (see
    /// [`crate::ui::UiPanelSpec`]). Parsed in [`run`]; `None` renders the globe
    /// alone.
    pub ui: Option<String>,
}

/// Renders one frame per `params`, writes it to `params.output` as a PNG, and
/// prints a summary to stdout. Exits the process with a nonzero status on a
/// usage error (bad datetime, bad dimensions, or write failure).
pub fn run(params: RenderParams) {
    // Parse the datetime first. No EOP range check - see the module note.
    let time = match parse_rfc3339(&params.datetime) {
        Ok(time) => time,
        Err(message) => fail(&format!(
            "invalid --datetime '{}': {message} (expected RFC3339 UTC, e.g. 2024-01-15T12:30:00Z)",
            params.datetime
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
    // has no clock, no tracked satellites, and no SimulationState. The camera
    // math is identical to the windowed path (see `application`'s redraw).
    let celestial = CelestialSphere::at(&time);
    let celestial_to_world = celestial.star_rot_inv.transpose();

    let distance = Camera::clamp_distance(params.distance_km);
    let camera = Camera {
        longitude: params.longitude,
        latitude: params.latitude,
        distance,
        tilt: params.tilt,
    };
    let aspect = params.width as f32 / params.height.max(1) as f32;
    let eye = camera.eye(celestial_to_world);

    let render = RenderState {
        view_proj: camera.view_proj(aspect, celestial_to_world),
        camera_pos: eye,
        sun_dir: celestial.sun_dir,
        star_rot_inv: celestial.star_rot_inv,
        // Pure globe: render mode tracks no satellites, so no markers.
        markers: Vec::new(),
    };

    // Build the optional mock-UI overlay before touching the GPU, so a JSON
    // error fails cleanly without spinning up a device.
    let ui_frame = params
        .ui
        .as_deref()
        .map(|json| build_ui_frame(json, params.width, params.height));

    let image = HeadlessRenderer::new(params.width, params.height).render(&render, ui_frame);

    if let Err(error) = image.save(&params.output) {
        fail(&format!(
            "failed to write {}: {error}",
            params.output.display()
        ));
    }

    print_summary(&params, &time, &celestial, distance);
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

/// Parses the `--ui` JSON into mock panels and runs them through egui once to
/// produce a render-ready [`UiFrame`] - the headless analogue of the windowed
/// egui driving in `application`. The mock is rendered via the same
/// `ui::control_panel` the live app uses, so the overlay is faithful. The egui
/// screen is sized to the output in points at 1.0 pixels-per-point, so mock
/// positions are in output pixels. Exits with a clean error on bad JSON.
fn build_ui_frame(json: &str, width: u32, height: u32) -> UiFrame {
    let panels: Vec<UiPanelSpec> = serde_json::from_str(json)
        .unwrap_or_else(|error| fail(&format!("invalid --ui JSON: {error}")));
    let mut mock = MockUi { panels };

    let ctx = egui::Context::default();
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
    // load fonts/lay out, then a second pass for the real geometry. egui emits
    // each texture delta exactly once, so the font-atlas allocation arrives on
    // the *warmup* output; merge its texture deltas into the second pass's so
    // the renderer actually gets the atlas the glyph primitives reference.
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
/// datetime, the subsolar point (so the day side / terminator location is
/// known), the camera, and the output path. Informational only - render mode is
/// deliberately silent about EOP range (see the module note).
fn print_summary(
    params: &RenderParams,
    time: &Instant,
    celestial: &CelestialSphere,
    distance: f32,
) {
    let (year, month, day, hour, minute, second) = time.as_datetime();

    // Note if the supplied distance was clamped into the camera's valid range.
    let clamped = if (distance - params.distance_km).abs() > f32::EPSILON {
        format!(" (clamped from {:.1})", params.distance_km)
    } else {
        String::new()
    };

    println!("Rendered single frame:");
    println!(
        "  datetime:  {year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{:02} UTC",
        second as i32
    );
    println!(
        "  subsolar:  lat {:.3} lon {:.3} deg",
        celestial.subsolar_lat_deg, celestial.subsolar_lon_deg
    );
    println!(
        "  camera:    lon {:.3} lat {:.3} deg, distance {:.1} km{clamped}, tilt {:.3} deg",
        params.longitude, params.latitude, distance, params.tilt
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
