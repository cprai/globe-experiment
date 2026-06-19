//! The application layer: windowing, the winit event loop, per-frame redraw
//! orchestration, and the camera (rig + all input/animation). It owns the
//! `SimulationState` and the `Gfx` renderer, updates the camera from window
//! input, advances the simulation, and drives each render.
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
use crate::simulation::SimulationState;
use crate::ui;
use camera::Camera;
use input::Controller;

/// Runs the winit event loop to completion, driving `app`. Frames are driven by
/// explicit redraw requests (input, inertia, egui repaints); idle means zero
/// GPU work.
pub fn run(mut app: ApplicationState) {
    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut app).expect("run event loop");
}

/// The application: owns the window, the egui logic side (`Context` +
/// `egui_winit::State`), the camera and its input controller, the simulation,
/// and the renderer. The window/egui_state/gfx are created on `resumed`.
pub struct ApplicationState {
    camera: Camera,
    /// All simulation state: clock, tracked satellite, ephemeris-driven sky.
    simulation: SimulationState,
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

impl ApplicationState {
    /// Builds the application around an already-constructed simulation. The
    /// window, egui platform state, and renderer are created later, on the
    /// first `resumed`.
    pub fn new(simulation: SimulationState) -> Self {
        Self {
            camera: Camera::default(),
            simulation,
            controller: Controller::default(),
            window: None,
            egui_ctx: egui::Context::default(),
            egui_state: None,
            gfx: None,
            shown: false,
        }
    }
}

impl ApplicationHandler for ApplicationState {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }

        // The window stays hidden through device setup, texture upload,
        // and pipeline creation; it's shown after the first frame is
        // presented, so it appears with the globe already rendered
        // instead of sitting blank while loading.
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Globe")
                        .with_visible(false),
                )
                .expect("create window"),
        );

        let gfx = Gfx::init(window.clone());
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

impl ApplicationState {
    /// Routes an input event: egui gets first claim (sliders, panel
    /// hover); whatever it doesn't consume drives the globe camera.
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
        // ephemeris-driven sky is re-evaluated at the new time. (This frame's
        // play/pause and speed edits come from the previous frame's UI, applied
        // here - a one-frame, ~16 ms delay, imperceptible.) A running clock is
        // another "animating" source - it keeps requesting frames; when paused
        // nothing advances and the app can go idle.
        animating |= self.simulation.advance();

        // Resolve the inertial-frame camera into the Earth-fixed world frame
        // using the sky's current orientation, then hand the finished view to
        // the simulation to produce this frame's RenderState *and* the UI's
        // TelemetryState in one shot (a single satellite propagation feeds
        // both). All the camera math lives here (with the camera); the
        // simulation only consumes the resolved eye/view and fills in the
        // astronomical positions.
        let (width, height) = gfx.viewport();
        let aspect = width / height.max(1.0);
        let celestial_to_world = self.simulation.celestial_to_world();
        let eye = self.camera.eye(celestial_to_world);
        let view_proj = self.camera.view_proj(aspect, celestial_to_world);
        let (render_state, telemetry) = self.simulation.frame_state(eye, view_proj);

        // Run the egui UI from that telemetry snapshot (so the readout matches
        // the rendered marker exactly); it mutates only the Clock. The logic
        // and tessellation live here (the renderer only draws the primitives).
        let raw_input = egui_state.take_egui_input(&window);
        let full_output = self.egui_ctx.run_ui(raw_input, |ui| {
            ui::control_panel(ui.ctx(), &telemetry, &mut self.simulation.clock)
        });
        egui_state.handle_platform_output(&window, full_output.platform_output);

        // egui resets the cursor icon every frame; restore the globe's
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
