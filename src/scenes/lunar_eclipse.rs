//! Lunar-eclipse scene: the 2025-03-14 total lunar eclipse, with no tracked
//! objects - just the celestial sphere wound to the event, so Terra's
//! shadow falls on Luna and turns it a coppery "blood-red Luna" (CLI:
//! `globe-experiment scene lunar_eclipse`). Like `solar_eclipse` it carries
//! no `Satellite` list; its clock starts directly from the eclipse datetime,
//! and it draws no markers.

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

/// Eye distance for the Luna framing (km): ~2 lunar radii above the surface, so
/// the eclipsed disc fills the frame with a little margin (the camera orbits
/// Luna, so the distance is relative to its surface, not Terra's).
const VIEW_DISTANCE_KM: f64 = 3500.0;

/// Empty lunar-eclipse simulation: just the clock; no satellites. Carries a
/// [`TargetSelector`] so the view can be switched between orbiting Luna (the
/// default blood-red-Luna framing) and orbiting Terra. No celestial sphere is
/// stored - `CelestialSphere::at` is a pure function of time, so
/// `frame_state` evaluates it fresh at each frame's clock instant (`new`
/// builds a throwaway one for the initial framing).
pub struct LunarEclipseScene {
    /// Simulation clock (datetime + play/paused + speed).
    clock: Clock,
    selector: TargetSelector,
    /// The scene's interactive orbital camera (pan/tilt/zoom rig plus its
    /// animations), seeded orbiting Luna with the eclipsed near side framed.
    camera: PtzCamera,
    /// The body the camera orbits this frame - owned by the scene, not the
    /// camera, and passed into every camera call that scales by or centers
    /// on it. Refreshed from the selector each frame; a genuine switch
    /// reframes the camera (`frame_state` compares via `same_kind`).
    camera_target: CameraTarget,
}

impl LunarEclipseScene {
    fn new() -> Self {
        // ~30 min before greatest eclipse (06:58 UTC) - the start of totality -
        // so the auto-playing clock runs through the deep umbral phase. Well
        // inside the bundled EOP range (1962-01-01 .. build date).
        let epoch =
            Instant::from_datetime(2025, 3, 14, 6, 28, 0.0).expect("valid lunar-eclipse datetime");
        // `scene::init` must already have run (the celestial sphere reads
        // satkit globals).
        let clock = Clock::new(epoch);
        let celestial_sphere = CelestialSphere::at(&clock.now());

        // Orbit Luna, looking at its Terra-facing near side (which is the side
        // in Terra's shadow - the blood-red Luna). Looking *toward* Luna
        // places the eye on its Terra-facing side, so Terra is behind the
        // camera and never occludes the disc - no limb nudge needed. The
        // Terra->Luna direction: the celestial sphere is heliocentric, so this
        // is Luna's center minus Terra's center, not Luna's raw position.
        let center = celestial_sphere.luna().placement.pos_world
            - celestial_sphere.center_world(CelestialBody::TERRA);
        // Matches the selector default below - the camera_target must start on
        // the body the framing above orbits, or the first frame would reframe.
        let camera_target = CameraTarget::Body(CelestialBody::LUNA);
        let camera = PtzCamera::looking_toward(
            &camera_target,
            celestial_sphere.star_rot_inv,
            center,
            VIEW_DISTANCE_KM,
        );

        Self {
            clock,
            // Default to orbiting Luna - the whole point is the blood-red Luna.
            selector: TargetSelector::new(true),
            camera,
            camera_target,
        }
    }
}

impl Scene for LunarEclipseScene {
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

impl CameraControl for LunarEclipseScene {
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

impl CameraView for LunarEclipseScene {
    fn frame_state(&mut self) -> RenderState {
        let now = self.clock.now();
        // This frame's celestial sphere, evaluated on the spot: `at` is a
        // pure function of time, so it needs no stashing between frames (the
        // renderer re-derives the same sphere from `RenderState.time`).
        let sphere = CelestialSphere::at(&now);

        // Refresh the scene-owned camera target from the selector; a genuine
        // Luna<->Terra switch reframes the camera (full-frame distance,
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
        // system from the frame's time; the selector's target (Luna or Terra)
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
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        // The Time panel (datetime + run/speed) plus the Terra / Luna
        // camera-target selector. The panels borrow disjoint fields (`clock`
        // vs `selector`). The panel builder is deliberately kept per-scene
        // - scenes may diverge in what they expose.
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

/// Builds the lunar-eclipse scene already orbiting Luna (the eclipsed
/// near-side disc centered - the camera is seeded in `new`) and hands off to
/// the winit event loop.
pub fn run() {
    scene::init();

    application::run(ApplicationState::new(LunarEclipseScene::new()));
}
