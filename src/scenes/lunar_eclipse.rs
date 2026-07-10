//! Lunar-eclipse scene: the 2025-03-14 total lunar eclipse, with no tracked
//! objects - just the celestial sphere wound to the event, so Terra's
//! shadow falls on Luna and turns it a coppery "blood-red Luna" (CLI:
//! `globe-experiment scene lunar_eclipse`). Like `solar_eclipse` it carries
//! no `Satellite` list; its clock starts directly from the eclipse datetime,
//! and it draws no markers.

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

/// Eye distance for the Luna framing (km): ~2 lunar radii above the surface, so
/// the eclipsed disc fills the frame with a little margin (the camera orbits
/// Luna, so the distance is relative to its surface, not Terra's).
const VIEW_DISTANCE_KM: f64 = 3500.0;

/// Empty lunar-eclipse simulation: just the clock; no satellites. Its Camera
/// Target panel switches the view between orbiting Luna (the default
/// blood-red-Luna framing) and orbiting Terra by writing `camera_target`
/// directly. No celestial sphere is stored - `CelestialSphere::at` is a pure
/// function of time, so `frame_state` evaluates it fresh at each frame's
/// clock instant (`new` builds a throwaway one for the initial framing).
pub struct LunarEclipseScene {
    /// Simulation clock (datetime + play/paused + speed), reached only via
    /// the `SceneClock` API - the Time panel's Run-toggle/speed-slider
    /// callbacks included, which receive the scene at fire time and call the
    /// setters directly.
    clock: Clock,
    /// The body the camera orbits - owned by the scene, not the camera, and
    /// passed into every camera call that scales by or centers on it.
    /// Written directly by the Camera Target panel's key callbacks (via
    /// [`Self::set_camera_target`], which reframes the camera on a genuine
    /// Luna<->Terra switch).
    camera_target: CameraTarget,
    /// The scene's interactive orbital camera (pan/tilt/zoom rig plus its
    /// animations), seeded orbiting Luna with the eclipsed near side framed.
    camera: PtzCamera,
}

impl LunarEclipseScene {
    fn new() -> Self {
        // ~30 min before greatest eclipse (06:58 UTC) - the start of totality -
        // so the auto-playing clock runs through the deep umbral phase. Well
        // inside the bundled EOP range (1962-01-01 .. build date).
        let epoch =
            Instant::from_datetime(2025, 3, 14, 6, 28, 0.0).expect("valid lunar-eclipse datetime");
        // `scene::init` must already have run (the celestial sphere reads
        // satkit globals). Framed at the epoch - the clock has not ticked yet.
        let celestial_sphere = CelestialSphere::at(&epoch);

        // Orbit Luna, looking at its Terra-facing near side (which is the side
        // in Terra's shadow - the blood-red Luna). Looking *toward* Luna
        // places the eye on its Terra-facing side, so Terra is behind the
        // camera and never occludes the disc - no limb nudge needed. The
        // Terra->Luna direction: the celestial sphere is heliocentric, so this
        // is Luna's center minus Terra's center, not Luna's raw position.
        let center = celestial_sphere.luna().placement.pos_world
            - celestial_sphere.center_world(CelestialBody::TERRA);
        // Default to orbiting Luna - the whole point is the blood-red Luna;
        // the camera_target starts on the body the framing below orbits
        // (the Camera Target panel's Luna key).
        let camera_target = CameraTarget::Body(CelestialBody::LUNA);
        let camera = PtzCamera::looking_toward(
            &camera_target,
            celestial_sphere.star_rot_inv,
            center,
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

impl SceneClock for LunarEclipseScene {
    fn clock_mut(&mut self) -> &mut Clock {
        &mut self.clock
    }
}

impl Scene for LunarEclipseScene {
    fn advance(&mut self, _running: bool) {
        // Nothing scene-specific: the clock tick lives in `tick_scene` (any
        // Camera Target key press or Time-panel edit already landed directly
        // during the previous egui pass), and `frame_state` re-derives the
        // celestial sphere at the frame's clock instant.
    }
}

impl ScenePtzCamera for LunarEclipseScene {
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

impl CameraView for LunarEclipseScene {
    fn frame_state(&mut self) -> RenderState {
        let now = self.clock_now();
        // This frame's celestial sphere, evaluated on the spot: `at` is a
        // pure function of time, so it needs no stashing between frames (the
        // renderer re-derives the same sphere from `RenderState.time`).
        let sphere = CelestialSphere::at(&now);

        // Resolve the inertial rig into the render frame (the moving Luna
        // center is re-resolved from the ephemeris inside `world_rig`). Any
        // Luna<->Terra switch already landed - and reframed the camera -
        // directly in the Camera Target key's callback (`set_camera_target`)
        // during the previous egui pass, so the target here is simply the
        // scene-owned one. The target packed below is the same one the rig
        // was built for.
        let celestial_to_world = sphere.star_rot_inv.transpose();
        let target = self.camera_target;
        let (eye, look_at, up) = self.camera.world_rig(&target, &sphere, celestial_to_world);

        // No satellites: an empty marker list. The renderer derives the Terra
        // system from the frame's time; the target (Luna or Terra)
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

impl UIDrawable for LunarEclipseScene {
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

/// The `scene lunar_eclipse` CLI arguments - none today. Each scene
/// subcommand declares its own arguments, so a future flag for this scene is
/// added here, not in `main` (which only dispatches).
#[derive(clap::Args)]
pub struct Args {}

/// Builds the lunar-eclipse scene already orbiting Luna (the eclipsed
/// near-side disc centered - the camera is seeded in `new`) and hands off to
/// the winit event loop.
pub fn run(_args: Args) {
    scene::init();

    application::run(ApplicationState::new(LunarEclipseScene::new()));
}
