//! Solar-eclipse scene: the 2024-04-08 total solar eclipse, with no tracked
//! objects - just the celestial sphere wound to the event, so Luna's shadow
//! sweeps across the daylit Terra (CLI: `globe-experiment scene
//! solar_eclipse`). Unlike the satellite scenes this carries no `Satellite`
//! list; its clock starts directly from the eclipse datetime rather than a TLE
//! epoch, and it draws no markers.

use satkit::Instant;

use crate::engine::application::{self, ApplicationState};
use crate::engine::camera::{CameraView, PtzCamera, ScenePtzCamera};
use crate::engine::scene::celestial_sphere::CelestialSphere;
use crate::engine::scene::{
    self, CameraTarget, CelestialBody, Clock, RenderState, Scene, SceneClock,
};
use crate::engine::ui::{
    Header, Instrument, InteractiveSlider, InteractiveToggle, PanelAnchor, Readout, Slider, Toggle,
    UIDrawable, UIDrawablePanel,
};

/// Eye distance for the day-side framing (km): Terra fills most of the
/// frame with Luna's umbral shadow spot centered near the subsolar point.
const VIEW_DISTANCE_KM: f64 = 22000.0;

/// Empty solar-eclipse simulation: just the clock; no satellites. Its Camera
/// Target panel switches the view between orbiting Terra (the default
/// day-side framing) and orbiting Luna by writing `camera_target` directly.
/// No celestial sphere is stored - `CelestialSphere::at` is a pure function
/// of time, so `frame_state` evaluates it fresh at each frame's clock
/// instant (`new` builds a throwaway one for the initial framing).
pub struct SolarEclipseScene {
    /// Simulation clock (datetime + play/paused + speed), reached only via
    /// the `SceneClock` API - the Time panel's Run-toggle/speed-slider
    /// callbacks included, which receive the scene at fire time and call the
    /// setters directly.
    clock: Clock,
    /// The body the camera orbits - owned by the scene, not the camera, and
    /// passed into every camera call that scales by or centers on it.
    /// Written directly by the Camera Target panel's key callbacks (via
    /// [`Self::set_camera_target`], which reframes the camera on a genuine
    /// Terra<->Luna switch).
    camera_target: CameraTarget,
    /// The scene's interactive orbital camera (pan/tilt/zoom rig plus its
    /// animations), seeded on the day-side framing.
    camera: PtzCamera,
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
        // satkit globals). Framed at the epoch - the clock has not ticked yet.
        let celestial_sphere = CelestialSphere::at(&epoch);

        // Frame the sunlit face (and Luna's shadow spot near the subsolar
        // point) by looking toward Sol, from the ephemeris at the start
        // instant. The celestial sphere is heliocentric, so the Terra->Sol
        // direction is Sol's position minus Terra's center (not just the Sol
        // position). The view stays fully interactive afterward.
        let terra_to_sol =
            celestial_sphere.sol_pos_world - celestial_sphere.center_world(CelestialBody::TERRA);
        // Default to orbiting Terra - the camera_target starts on the body
        // the framing below orbits (the Camera Target panel's Terra key).
        let camera_target = CameraTarget::terra();
        let camera = PtzCamera::looking_toward(
            &camera_target,
            celestial_sphere.star_rot_inv,
            -terra_to_sol.normalize(),
            VIEW_DISTANCE_KM,
        );

        Self {
            clock: Clock::new(epoch),
            camera_target,
            camera,
        }
    }

    /// Switches the orbited body - the Camera Target panel keys' fire-time
    /// callback. A genuine switch reframes the camera on the spot
    /// (full-frame distance, re-aim, in-flight animations dropped) against
    /// the sphere at the current clock instant; re-selecting the already
    /// orbited body is a no-op, which also keeps the callback idempotent
    /// under egui's discard-pass double fire.
    fn set_camera_target(&mut self, target: CameraTarget) {
        if self.camera_target.same_kind(&target) {
            return;
        }
        let sphere = CelestialSphere::at(&self.clock_now());
        self.camera
            .reframe(&target, &sphere, sphere.star_rot_inv.transpose());
        self.camera_target = target;
    }
}

impl SceneClock for SolarEclipseScene {
    fn clock_mut(&mut self) -> &mut Clock {
        &mut self.clock
    }
}

impl Scene for SolarEclipseScene {
    fn advance(&mut self) -> bool {
        // Advance the clock (any Camera Target key press or Time-panel edit
        // already landed directly during the previous egui pass). Returns
        // whether it is running - an "animating" source that keeps frames
        // coming; when
        // paused nothing advances and the app can go idle. Nothing else to
        // update: `frame_state` re-derives the celestial sphere at the
        // frame's clock instant.
        self.tick_clock()
    }
}

impl ScenePtzCamera for SolarEclipseScene {
    // The accessors behind the blanket `CameraControl` impl, which forwards
    // every input event into the embedded camera: where the camera and the
    // scene-owned orbit target live in this struct. A scene that needs to
    // diverge (gate input, swap the camera kind) implements `CameraControl`
    // directly instead.
    fn camera(&self) -> &PtzCamera {
        &self.camera
    }

    fn camera_mut(&mut self) -> &mut PtzCamera {
        &mut self.camera
    }

    fn camera_target(&self) -> &CameraTarget {
        &self.camera_target
    }
}

impl CameraView for SolarEclipseScene {
    fn frame_state(&mut self) -> RenderState {
        let now = self.clock_now();
        // This frame's celestial sphere, evaluated on the spot: `at` is a
        // pure function of time, so it needs no stashing between frames (the
        // renderer re-derives the same sphere from `RenderState.time`).
        let sphere = CelestialSphere::at(&now);

        // Resolve the inertial rig into the render frame (the moving Luna
        // center is re-resolved from the ephemeris inside `world_rig`). Any
        // Terra<->Luna switch already landed - and reframed the camera -
        // directly in the Camera Target key's callback (`set_camera_target`)
        // during the previous egui pass, so the target here is simply the
        // scene-owned one. The target packed below is the same one the rig
        // was built for.
        let celestial_to_world = sphere.star_rot_inv.transpose();
        let target = self.camera_target;
        let (eye, look_at, up) = self.camera.world_rig(&target, &sphere, celestial_to_world);

        // No satellites: an empty marker list. The renderer derives the Terra
        // system from the frame's time; the target (Terra or Luna)
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
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<Self>> {
        // The Time panel (datetime + run/speed) plus the Terra / Luna
        // Camera Target panel. The panel builders are deliberately kept
        // per-scene - scenes may diverge in what they expose.
        //
        // Snapshot the displayed values up front (owned `String`/`f32`/`bool`)
        // - the panels are owned and never borrow the scene. The control
        // callbacks receive the scene as `&mut Self` at fire time and call
        // the SceneClock setters directly; each stays idempotent under
        // egui's discard-pass double fire by writing snapshot-derived values
        // (the Run toggle sets the pre-click `running`, never a re-read
        // flip).
        let datetime = self.clock_datetime_label();
        // Padded to the widest value (MAX_MULTIPLIER "100.0" = 5 chars): the
        // font is monospace, so a fixed-width value keeps the digit window
        // from resizing as the speed changes.
        let speed = format!("{:>5.1}", self.clock_multiplier());
        let running = !self.clock_paused();

        // Exponential (base e) speed: the slider edits the exponent, so
        // multiplier = e^exp - real time (e^0 = 1x) at the left, 100x at the
        // right, 10x at the midpoint. The mapping lives here, not in the panel.
        let speed_exp = self.clock_multiplier().ln();
        let exp_range = Clock::MIN_MULTIPLIER.ln()..=Clock::MAX_MULTIPLIER.ln();

        // The producer groups instruments into rows + picks content only; all
        // styling and every metric live in the instrument modules / theme
        // (taffy bottom-aligns the Run key with the speed window beside it).
        let time_rows: Vec<Vec<Box<dyn Instrument<Self>>>> = vec![
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
                    // Pausing = setting the snapshotted pre-click `running`.
                    on_toggle: Box::new(move |scene: &mut Self| scene.set_clock_paused(running)),
                }),
            ],
            vec![Box::new(InteractiveSlider {
                slider: Slider {
                    value: speed_exp,
                    range: exp_range,
                },
                on_change: Box::new(|scene: &mut Self, exp| scene.set_clock_multiplier(exp.exp())),
            })],
        ];
        let mut panels = vec![UIDrawablePanel {
            anchor: PanelAnchor::TopLeft,
            rows: time_rows,
        }];

        // The Terra / Luna Camera Target panel: two latching keys splitting
        // one row, the orbited body lit. Each key's callback receives the
        // scene at fire time and writes the camera target directly through
        // `set_camera_target` (which reframes on a genuine switch and
        // no-ops on the already-orbited body - idempotent) - the scene holds
        // no selection state beyond `camera_target` itself.
        let luna_active = self
            .camera_target
            .same_kind(&CameraTarget::Body(CelestialBody::LUNA));
        let target_rows: Vec<Vec<Box<dyn Instrument<Self>>>> = vec![
            vec![Box::new(Header {
                title: "Camera Target".to_string(),
            })],
            vec![
                Box::new(InteractiveToggle {
                    toggle: Toggle {
                        label: "Terra".to_string(),
                        active: !luna_active,
                    },
                    on_toggle: Box::new(|scene: &mut Self| {
                        scene.set_camera_target(CameraTarget::terra())
                    }),
                }),
                Box::new(InteractiveToggle {
                    toggle: Toggle {
                        label: "Luna".to_string(),
                        active: luna_active,
                    },
                    on_toggle: Box::new(|scene: &mut Self| {
                        scene.set_camera_target(CameraTarget::Body(CelestialBody::LUNA))
                    }),
                }),
            ],
        ];
        panels.push(UIDrawablePanel {
            anchor: PanelAnchor::TopRight,
            rows: target_rows,
        });
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
