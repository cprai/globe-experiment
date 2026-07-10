//! Solar-eclipse scene: the 2024-04-08 total solar eclipse, with no tracked
//! objects - just the celestial sphere wound to the event, so Luna's shadow
//! sweeps across the daylit Terra (CLI: `globe-experiment scene
//! solar_eclipse`). Unlike the satellite scenes this carries no `Satellite`
//! list; its clock starts directly from the eclipse datetime rather than a TLE
//! epoch, and it draws no markers.

use satkit::Instant;

use crate::engine::application::{self, ApplicationState};
use crate::engine::camera::{
    CameraControl, CameraView, CursorHint, PointerButton, PtzCamera, ScrollDelta,
};
use crate::engine::scene::celestial_sphere::CelestialSphere;
use crate::engine::scene::{
    self, CameraTarget, CelestialBody, Clock, RenderState, Scene, TargetSelector,
};
use crate::engine::ui::{
    Header, Instrument, InteractiveSlider, InteractiveToggle, PanelAnchor, Readout, Slider, Toggle,
    UIDrawable, UIDrawablePanel,
};

/// Eye distance for the day-side framing (km): Terra fills most of the
/// frame with Luna's umbral shadow spot centered near the subsolar point.
const VIEW_DISTANCE_KM: f64 = 22000.0;

/// Empty solar-eclipse simulation: just the clock; no satellites. Carries a
/// [`TargetSelector`] so the view can be switched between orbiting Terra (the
/// default day-side framing) and orbiting Luna. No celestial sphere is
/// stored - `CelestialSphere::at` is a pure function of time, so
/// `frame_state` evaluates it fresh at each frame's clock instant (`new`
/// builds a throwaway one for the initial framing).
pub struct SolarEclipseScene {
    /// Simulation clock (datetime + play/paused + speed).
    clock: Clock,
    selector: TargetSelector,
    /// The scene's interactive orbital camera (pan/tilt/zoom rig plus its
    /// animations), seeded on the day-side framing.
    camera: PtzCamera,
    /// The body the camera orbits this frame - owned by the scene, not the
    /// camera, and passed into every camera call that scales by or centers
    /// on it. Refreshed from the selector each frame; a genuine switch
    /// reframes the camera (`frame_state` compares via `same_kind`).
    camera_target: CameraTarget,
}

impl SolarEclipseScene {
    fn new() -> Self {
        // ~30 min before greatest eclipse (18:17 UTC), so the auto-playing clock
        // runs into and through the umbra's crossing of North America. Well
        // inside the bundled EOP range (1962-01-01 .. build date), so the
        // ephemeris/Earth-orientation accuracy holds.
        let epoch =
            Instant::from_datetime(2024, 4, 8, 17, 47, 0.0).expect("valid solar-eclipse datetime");
        // `scene::init` must already have run (the celestial sphere reads
        // satkit globals).
        let clock = Clock::new(epoch);
        let celestial_sphere = CelestialSphere::at(&clock.now());

        // Frame the sunlit face (and Luna's shadow spot near the subsolar
        // point) by looking toward Sol, from the ephemeris at the start
        // instant. The celestial sphere is heliocentric, so the Terra->Sol
        // direction is Sol's position minus Terra's center (not just the Sol
        // position). The view stays fully interactive afterward.
        let terra_to_sol =
            celestial_sphere.sol_pos_world - celestial_sphere.center_world(CelestialBody::TERRA);
        // Matches the selector default below - the camera_target must start on
        // the body the framing above orbits, or the first frame would reframe.
        let camera_target = CameraTarget::terra();
        let camera = PtzCamera::looking_toward(
            &camera_target,
            celestial_sphere.star_rot_inv,
            -terra_to_sol.normalize(),
            VIEW_DISTANCE_KM,
        );

        Self {
            clock,
            // Default to orbiting Terra (the day-side framing above).
            selector: TargetSelector::new(false),
            camera,
            camera_target,
        }
    }
}

impl Scene for SolarEclipseScene {
    fn advance(&mut self) -> bool {
        // Fold in any pending target-selector key press before the camera target
        // is read this frame.
        self.selector.apply_requests();
        // Advance the clock. Returns whether it is running - an "animating"
        // source that keeps frames coming; when paused nothing advances and
        // the app can go idle. Nothing else to update: `frame_state`
        // re-derives the celestial sphere at the frame's clock instant.
        self.clock.tick()
    }
}

impl CameraControl for SolarEclipseScene {
    // The input methods forward to the embedded PtzCamera; the forwarding
    // block is deliberately duplicated per scene (like the Time panel) so
    // a scene can diverge - e.g. gate input or swap the camera kind.
    fn pointer_press(&mut self, button: PointerButton) -> bool {
        self.camera.pointer_press(button)
    }

    fn pointer_release(&mut self, button: PointerButton) -> bool {
        self.camera.pointer_release(button)
    }

    fn pointer_move(&mut self, position: (f64, f64), viewport_height: f64) -> bool {
        self.camera
            .pointer_move(&self.camera_target, position, viewport_height)
    }

    fn scroll(&mut self, delta: ScrollDelta) -> bool {
        self.camera.scroll(&self.camera_target, delta)
    }

    fn tick(&mut self, viewport_height: f64) -> bool {
        self.camera.tick(&self.camera_target, viewport_height)
    }

    fn cursor_hint(&self) -> CursorHint {
        self.camera.cursor_hint()
    }
}

impl CameraView for SolarEclipseScene {
    fn frame_state(&mut self) -> RenderState {
        let now = self.clock.now();
        // This frame's celestial sphere, evaluated on the spot: `at` is a
        // pure function of time, so it needs no stashing between frames (the
        // renderer re-derives the same sphere from `RenderState.time`).
        let sphere = CelestialSphere::at(&now);

        // Refresh the scene-owned camera target from the selector; a genuine
        // Terra<->Luna switch reframes the camera (full-frame distance,
        // re-aim, in-flight animations dropped). Then resolve the inertial
        // rig into the render frame (the moving Luna center is re-resolved
        // from the ephemeris inside `world_rig`). The target packed below is
        // the same one the rig was built for.
        let celestial_to_world = sphere.star_rot_inv.transpose();
        let target = self.selector.resolve();
        if !self.camera_target.same_kind(&target) {
            self.camera.reframe(&target, &sphere, celestial_to_world);
        }
        self.camera_target = target;
        let (eye, look_at, up) = self.camera.world_rig(&target, &sphere, celestial_to_world);

        // No satellites: an empty marker list. The renderer derives the Terra
        // system from the frame's time; the selector's target (Terra or Luna)
        // keeps the origin at Terra either way.
        RenderState {
            time: now,
            camera_target: target,
            camera_pos: eye,
            camera_look_at: look_at,
            camera_up: up,
            markers: Vec::new(),
        }
    }
}

impl UIDrawable for SolarEclipseScene {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        // The Time panel (datetime + run/speed) plus the Terra / Luna
        // camera-target selector. The panels borrow disjoint fields (`clock`
        // vs `selector`), so both can be live at once. The panel builder is
        // deliberately kept per-scene - scenes may diverge in what they
        // expose.
        //
        // Snapshot the displayed values up front (owned `String`/`f32`/`bool`),
        // so no shared borrow of the clock outlives into the mutable callback
        // captures below. The two control callbacks capture disjoint clock
        // fields (`paused` vs `multiplier`) via direct field assignment - a
        // `Clock` method would borrow the whole clock and collide.
        let datetime = self.clock.datetime_label();
        // Padded to the widest value (MAX_MULTIPLIER "100.0" = 5 chars): the
        // font is monospace, so a fixed-width value keeps the digit window
        // from resizing as the speed changes.
        let speed = format!("{:>5.1}", self.clock.multiplier);
        let running = !self.clock.paused;

        // Exponential (base e) speed: the slider edits the exponent, so
        // multiplier = e^exp - real time (e^0 = 1x) at the left, 100x at the
        // right, 10x at the midpoint. The mapping lives here, not in the panel.
        let speed_exp = self.clock.multiplier.ln();
        let exp_range = Clock::MIN_MULTIPLIER.ln()..=Clock::MAX_MULTIPLIER.ln();

        // The producer groups instruments into rows + picks content only; all
        // styling and every metric live in the instrument modules / theme
        // (taffy bottom-aligns the Run key with the speed window beside it).
        let time_rows: Vec<Vec<Box<dyn Instrument + '_>>> = vec![
            vec![Box::new(Header {
                title: "Time".to_string(),
            })],
            vec![Box::new(Readout {
                label: "UTC".to_string(),
                value: datetime,
                unit: String::new(),
            })],
            vec![
                Box::new(Readout {
                    label: "Speed".to_string(),
                    value: speed,
                    unit: "x".to_string(),
                }),
                Box::new(InteractiveToggle {
                    toggle: Toggle {
                        label: "Run".to_string(),
                        active: running,
                    },
                    on_toggle: Box::new(|| self.clock.paused = !self.clock.paused),
                }),
            ],
            vec![Box::new(InteractiveSlider {
                slider: Slider {
                    value: speed_exp,
                    range: exp_range,
                },
                on_change: Box::new(|exp| self.clock.multiplier = exp.exp()),
            })],
        ];
        let mut panels = vec![UIDrawablePanel {
            anchor: PanelAnchor::TopLeft,
            rows: time_rows,
        }];
        panels.push(self.selector.panel());
        panels
    }
}

/// The `scene solar_eclipse` CLI arguments - none today. Each scene
/// subcommand declares its own arguments, so a future flag for this scene is
/// added here, not in `main` (which only dispatches).
#[derive(clap::Args)]
pub struct Args {}

/// Builds the solar-eclipse scene (framed on the daylit face so Luna's
/// shadow spot is in view - the camera is seeded in `new`) and hands off to
/// the winit event loop.
pub fn run(_args: Args) {
    // Seed satkit's globals (embedded ephemeris + EOP) before the celestial
    // sphere is built in `new` below.
    scene::init();

    application::run(ApplicationState::new(SolarEclipseScene::new()));
}
