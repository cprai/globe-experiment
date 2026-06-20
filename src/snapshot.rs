//! Single-frame render mode (the `render` CLI subcommand): draws one frame to a
//! PNG and exits, with no winit window, input, or egui. The frame is positioned
//! by an explicit datetime (which fixes the celestial positions) and explicit
//! camera parameters, and written to a caller-specified path - intended for
//! visual debugging of rendering changes, e.g. by an agent that opens the
//! image. This is the headless analogue of a scenario's `run`.
//!
//! IMPORTANT: unlike scenarios (see the "Scenarios & valid time range" rules in
//! `CLAUDE.md`), render mode does **not** range-check the datetime against the
//! bundled Earth-orientation (EOP) data. The caller owns the time, and an
//! out-of-range datetime silently degrades rather than erroring: before
//! ~1962-01-01 satkit falls back to zero EOP, and past the last bundled EOP
//! entry it constant-extrapolates. Choosing an in-range past datetime for an
//! accurate frame is the caller's responsibility. This deliberate deviation is
//! also documented in `CLAUDE.md`/`MEMORY.md` and the `analyze-render` skill.

use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use satkit::Instant;

use crate::application::Camera;
use crate::renderer::{HeadlessRenderer, MAX_FRAME_DIMENSION};
use crate::simulation::celestial_sphere::CelestialSphere;
use crate::simulation::{self, RenderState};

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

    let image = HeadlessRenderer::new(params.width, params.height).render(&render);

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
