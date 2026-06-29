//! The application layer: windowing, the winit event loop, per-frame redraw
//! orchestration, and the camera (rig + all input/animation). It is generic
//! over any `S: Simulation`, owns the `Gfx` renderer, updates the camera from
//! window input, advances the simulation, and drives each render.
//!
//! The camera and its input controller live here so that swapping the input
//! scheme (e.g. adding touch controls) stays local to this module; the
//! simulation and renderer only ever consume a resolved camera position/view.

mod camera;
mod input;

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::renderer::{FrameOutcome, Gfx, UiFrame};
use crate::simulation::Simulation;
use crate::ui::{self, UIDrawable};
// Re-exported crate-wide so the headless `render` mode (`crate::snapshot`) can
// build the same camera rig; only the input `Controller` stays private here.
pub(crate) use camera::Camera;
use input::Controller;

/// Runs the winit event loop to completion, driving `app`. Frames are driven by
/// explicit redraw requests (input, inertia, egui repaints); idle means zero
/// GPU work.
pub fn run<S: Simulation + UIDrawable>(mut app: ApplicationState<S>) {
    let event_loop = build_event_loop();
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut app).expect("run event loop");
}

/// Creates the winit event loop, forcing X11 on WSL.
///
/// WSLg's native Wayland compositor drops EGL connections under GPU load,
/// causing broken-pipe crashes. Checking `WSL_DISTRO_NAME` (set by WSL2 for
/// every distro) lets us steer winit toward the XCB/X11 backend before the
/// compositor can break the connection. On all other platforms the default
/// backend selection is unchanged.
fn build_event_loop() -> EventLoop<()> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WSL_DISTRO_NAME").is_some() {
        use winit::platform::x11::EventLoopBuilderExtX11;
        return EventLoop::builder()
            .with_x11()
            .build()
            .expect("create event loop");
    }
    EventLoop::new().expect("create event loop")
}

/// The application: owns the window, the egui logic side (`Context` +
/// `egui_winit::State`), the camera and its input controller, the simulation,
/// and the renderer. The window/egui_state/gfx are created on `resumed`.
/// Generic over `S: Simulation` so any scenario can be plugged in without
/// changing the application layer.
pub struct ApplicationState<S: Simulation + UIDrawable> {
    camera: Camera,
    simulation: S,
    controller: Controller,
    /// The window, created on `resumed`. Shared with the renderer's surface.
    window: Option<Arc<Window>>,
    /// egui's logic/platform side (the GPU side lives in `Gfx`).
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    gfx: Option<Gfx>,
    /// Whether the window has been made visible; flips on the first
    /// presented frame.
    shown: bool,
}

impl<S: Simulation + UIDrawable> ApplicationState<S> {
    /// Builds the application around an already-constructed simulation. The
    /// window, egui platform state, and renderer are created later, on the
    /// first `resumed`.
    pub fn new(simulation: S) -> Self {
        let egui_ctx = egui::Context::default();
        // Stamp the Apollo-panel theme onto the context once; the live UI and a
        // headless mock share the same `install_theme`, so they look identical.
        ui::install_theme(&egui_ctx);
        Self {
            camera: Camera::default(),
            simulation,
            controller: Controller::default(),
            window: None,
            egui_ctx,
            egui_state: None,
            gfx: None,
            shown: false,
        }
    }

    /// Builds the application with a specific initial camera instead of the
    /// default whole-body view. Used by scenarios that want to frame a specific
    /// event on launch (e.g. the eclipse scenarios aim at the Sol/Luna); the
    /// camera is fully interactive afterward.
    pub fn with_camera(simulation: S, camera: Camera) -> Self {
        Self {
            camera,
            ..Self::new(simulation)
        }
    }
}

impl<S: Simulation + UIDrawable> ApplicationHandler for ApplicationState<S> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }

        // The window stays hidden through device setup, texture upload,
        // and pipeline creation; it's shown after the first frame is
        // presented, so it appears with the scene already rendered
        // instead of sitting blank while loading.
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Solar System")
                        .with_visible(false),
                )
                .expect("create window"),
        );

        let gfx = Gfx::init(window.clone(), event_loop.owned_display_handle());
        window.set_cursor(self.controller.cursor_icon());

        // egui's platform side needs the window; the GPU side already went
        // into Gfx. The context is shared with the input/redraw paths.
        let egui_state = egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );

        self.window = Some(window);
        self.egui_state = Some(egui_state);
        self.gfx = Some(gfx);

        // The first frame is rendered directly rather than via
        // request_redraw(): a hidden window receives no RedrawRequested
        // on Windows (paint events are only generated for visible
        // windows), so waiting for one would deadlock with the window
        // never shown. redraw() reveals the window after presenting.
        self.redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let (Some(gfx), Some(window)) = (self.gfx.as_mut(), self.window.as_ref()) {
                    gfx.resize(size);
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            event => self.handle_input(event),
        }
    }
}

impl<S: Simulation + UIDrawable> ApplicationState<S> {
    /// Routes an input event: egui gets first claim (sliders, panel
    /// hover); whatever it doesn't consume drives the orbital camera.
    fn handle_input(&mut self, event: WindowEvent) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let Some(egui_state) = self.egui_state.as_mut() else {
            return;
        };
        let Some(gfx) = self.gfx.as_ref() else {
            return;
        };

        let response = egui_state.on_window_event(&window, &event);
        if response.repaint {
            window.request_redraw();
        }
        if response.consumed {
            return;
        }

        if self
            .controller
            .handle_event(&event, &mut self.camera, gfx.viewport().1)
        {
            window.request_redraw();
        }
    }

    fn redraw(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let Some(gfx) = self.gfx.as_mut() else {
            return;
        };
        let Some(egui_state) = self.egui_state.as_mut() else {
            return;
        };

        // Step flick inertia with real frame time; while it's coasting,
        // each frame requests the next one.
        let mut animating = self.controller.tick(&mut self.camera, gfx.viewport().1);

        // Advance the simulation: the clock steps and, while it runs, the
        // ephemeris-driven celestial sphere is re-evaluated at the new time.
        // (This frame's play/pause and speed edits come from the previous
        // frame's UI, applied here - a one-frame, ~16 ms delay, imperceptible.)
        // A running clock is another "animating" source - it keeps requesting
        // frames; when paused nothing advances and the app can go idle.
        animating |= self.simulation.advance();

        // Resolve the inertial-frame camera into the Earth-fixed world frame
        // using the celestial sphere's current orientation, then hand the
        // finished view to the simulation to produce this frame's RenderState
        // *and* the UI snapshots in one shot (a single satellite propagation
        // feeds both). All the camera math lives here (with the camera); the
        // simulation only consumes the resolved eye/view and fills in the
        // astronomical positions.
        let (width, height) = gfx.viewport();
        let aspect = width / height.max(1.0);
        let celestial_to_world = self.simulation.celestial_to_world();

        // Re-aim the orbital camera at this frame's target (the scenario's
        // chosen body, with Luna's center refreshed from the ephemeris). On
        // a genuine body switch the reframe invalidates any in-flight zoom/flick
        // (they target the old body's scale), so cancel them.
        let target = self.simulation.camera_target();
        if self.camera.retarget(target, celestial_to_world) {
            self.controller.reset_animation();
        }

        // The eye in the floating-origin (render) frame; the renderer works in
        // that frame so far planet targets stay f32-precise.
        let eye = self.camera.eye_relative(celestial_to_world);
        let view_proj = self.camera.view_proj(aspect, celestial_to_world);
        let render_state = self.simulation.frame_state(eye, view_proj);

        // Run the egui UI: the panel pulls the scenario's drawable elements
        // (read from the propagation just done above, so the readout matches the
        // rendered markers) and renders them, firing the elements' callbacks for
        // any interaction. The logic and tessellation live here (the renderer
        // only draws the primitives). `self.simulation` and `self.egui_ctx` are
        // disjoint fields, so the panel's `&mut self.simulation` borrow coexists
        // with the `run_ui` receiver borrow.
        let raw_input = egui_state.take_egui_input(&window);
        let simulation = &mut self.simulation;
        let full_output = self
            .egui_ctx
            .run_ui(raw_input, |ui| ui::control_panel(ui.ctx(), simulation));
        egui_state.handle_platform_output(&window, full_output.platform_output);

        // egui resets the cursor icon every frame; restore the scene's
        // grab cursor whenever the pointer isn't on the panel.
        if !self.egui_ctx.is_pointer_over_egui() {
            window.set_cursor(self.controller.cursor_icon());
        }

        // Tessellate egui into render-ready primitives for the renderer.
        let ui_frame = UiFrame {
            primitives: self
                .egui_ctx
                .tessellate(full_output.shapes, full_output.pixels_per_point),
            textures_delta: full_output.textures_delta,
            pixels_per_point: full_output.pixels_per_point,
        };

        let egui_repaint = full_output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|output| output.repaint_delay.is_zero());

        match gfx.update(&window, &render_state, ui_frame) {
            FrameOutcome::Presented => {
                // First frame is on the surface - reveal the window.
                if !self.shown {
                    self.shown = true;
                    window.set_visible(true);
                }
            }
            // Hidden/minimized: skip the frame; the next expose event requests
            // a redraw. If the first frame lands here (some backends report the
            // still-hidden window as occluded), show the window and retry
            // rather than deadlocking invisible.
            FrameOutcome::Occluded => {
                if !self.shown {
                    self.shown = true;
                    window.set_visible(true);
                    window.request_redraw();
                }
                return;
            }
            // Surface acquire failed (lost/outdated/timeout); the renderer
            // reconfigured where needed - retry next frame.
            FrameOutcome::Reconfigured => {
                window.request_redraw();
                return;
            }
        }

        if animating || egui_repaint {
            window.request_redraw();
        }
    }
}
