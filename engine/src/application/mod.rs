//! The application layer: windowing, the winit event loop, per-frame redraw
//! orchestration, and the windowed presenter (`gfx`). Generic over any
//! `S: Scene + CameraControl + CameraView + UIDrawable`. It keeps NO camera
//! or input state: each winit input event is translated statelessly into one
//! device-neutral `CameraControl` call (`translate_camera_event`).

mod gfx;

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{CursorIcon, Window, WindowId};

use crate::camera::{CameraControl, CameraView, CursorHint, PointerButton, ScrollDelta};
use crate::renderer::UiFrame;
use crate::scene::{Scene, SceneClock};
use crate::ui::{self, UIDrawable};
use gfx::{FrameOutcome, Gfx};

/// Runs the winit event loop to completion, driving `app`. Frames render
/// continuously (each presented frame requests the next, vsync-paced); only
/// an occluded window stops rendering until an expose event.
pub fn run<S: Scene + SceneClock + CameraControl + CameraView + UIDrawable>(
    mut app: ApplicationState<S>,
) {
    let event_loop = build_event_loop();
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut app).expect("run event loop");
}

/// Creates the winit event loop, forcing X11 on WSL: WSLg's Wayland
/// compositor drops EGL connections under GPU load (broken-pipe crash), so
/// `WSL_DISTRO_NAME` steers winit to the XCB/X11 backend.
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

/// The application: owns the window, the egui logic side, the simulation
/// (which carries its own camera), and the renderer. The window/egui_state/
/// gfx are created on `resumed`.
pub struct ApplicationState<S: Scene + SceneClock + CameraControl + CameraView + UIDrawable> {
    simulation: S,
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

impl<S: Scene + SceneClock + CameraControl + CameraView + UIDrawable> ApplicationState<S> {
    /// Builds the application around an already-constructed simulation; the
    /// window, egui platform state, and renderer come on the first `resumed`.
    pub fn new(simulation: S) -> Self {
        let egui_ctx = egui::Context::default();
        // The theme must be stamped onto each egui Context exactly once.
        ui::install_theme(&egui_ctx);
        Self {
            simulation,
            window: None,
            egui_ctx,
            egui_state: None,
            gfx: None,
            shown: false,
        }
    }
}

impl<S: Scene + SceneClock + CameraControl + CameraView + UIDrawable> ApplicationHandler
    for ApplicationState<S>
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }

        // Created hidden and revealed after the first presented frame, so
        // the window appears with the scene already rendered.
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
        window.set_cursor(cursor_icon(self.simulation.cursor_hint()));

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

        // Direct call, not request_redraw(): Windows delivers no
        // RedrawRequested to a hidden window, so waiting for one would
        // deadlock with the window never shown (redraw() does the reveal).
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

impl<S: Scene + SceneClock + CameraControl + CameraView + UIDrawable> ApplicationState<S> {
    /// Routes an input event: egui gets first claim; whatever it doesn't
    /// consume goes to the scene's `CameraControl` impl.
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
        if response.consumed {
            return;
        }

        translate_camera_event(&mut self.simulation, &event, gfx.viewport().1);
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

        // Camera animation, then clock + scene advance. (This frame's
        // play/pause and speed edits come from the previous frame's UI - a
        // one-frame, ~16 ms delay, imperceptible.)
        self.simulation.tick(gfx.viewport().1);
        self.simulation.tick_scene();

        let render_state = self.simulation.frame_state();

        // Panels are pulled ONCE per frame, outside run_ui - load-bearing:
        // egui's discard pass (max_passes = 2) re-runs the closure, and only
        // fixed build-time callback snapshots keep a twice-fired callback
        // idempotent. The panels are owned (they never borrow the scene), so
        // `&mut panels` and `&mut simulation` coexist in the closure.
        let raw_input = egui_state.take_egui_input(&window);
        let mut panels = self.simulation.get_drawables();
        let simulation = &mut self.simulation;
        let full_output = self.egui_ctx.run_ui(raw_input, |ui| {
            ui::control_panel(ui.ctx(), &mut panels, simulation)
        });
        egui_state.handle_platform_output(&window, full_output.platform_output);

        // egui resets the cursor icon every frame; restore the scene's
        // grab cursor whenever the pointer isn't on the panel.
        if !self.egui_ctx.is_pointer_over_egui() {
            window.set_cursor(cursor_icon(self.simulation.cursor_hint()));
        }

        let ui_frame = UiFrame {
            primitives: self
                .egui_ctx
                .tessellate(full_output.shapes, full_output.pixels_per_point),
            textures_delta: full_output.textures_delta,
            pixels_per_point: full_output.pixels_per_point,
        };

        match gfx.update(&window, &render_state, ui_frame) {
            FrameOutcome::Presented => {
                // First frame is on the surface - reveal the window.
                if !self.shown {
                    self.shown = true;
                    window.set_visible(true);
                }
            }
            // Hidden/minimized: skip the frame; the next expose event
            // requests a redraw. If the first frame lands here (macOS can
            // report a still-hidden window as occluded), show the window
            // and retry rather than deadlocking invisible.
            FrameOutcome::Occluded => {
                if !self.shown {
                    self.shown = true;
                    window.set_visible(true);
                    window.request_redraw();
                }
                return;
            }
            // Surface acquire failed; the renderer reconfigured where
            // needed - retry next frame.
            FrameOutcome::Reconfigured => {
                window.request_redraw();
                return;
            }
        }

        // Continuous render loop: every presented frame requests the next,
        // vsync-paced. The Occluded arm above is the one brake.
        window.request_redraw();
    }
}

/// Translates one winit window event into the matching device-neutral
/// `CameraControl` call, statelessly (cursor tracking lives in the camera).
fn translate_camera_event<C: CameraControl>(
    camera: &mut C,
    event: &WindowEvent,
    viewport_height: f64,
) {
    match event {
        WindowEvent::MouseInput { state, button, .. } => {
            // Only the buttons the camera vocabulary names; the rest are
            // dropped here.
            let button = match button {
                MouseButton::Left => PointerButton::Left,
                MouseButton::Right => PointerButton::Right,
                _ => return,
            };
            match state {
                ElementState::Pressed => camera.pointer_press(button),
                ElementState::Released => camera.pointer_release(button),
            }
        }
        WindowEvent::CursorMoved { position, .. } => {
            camera.pointer_move((position.x, position.y), viewport_height)
        }
        WindowEvent::MouseWheel { delta, .. } => {
            // Both variants are load-bearing (Windows/X11 wheels deliver
            // lines, macOS precision trackpads pixels).
            let delta = match delta {
                MouseScrollDelta::LineDelta(_, y) => ScrollDelta::Lines(f64::from(*y)),
                MouseScrollDelta::PixelDelta(position) => ScrollDelta::Pixels(position.y),
            };
            camera.scroll(delta)
        }
        _ => {}
    }
}

/// Maps the camera's winit-free cursor hint onto the winit icon set.
fn cursor_icon(hint: CursorHint) -> CursorIcon {
    match hint {
        CursorHint::Default => CursorIcon::Default,
        CursorHint::Grab => CursorIcon::Grab,
        CursorHint::Grabbing => CursorIcon::Grabbing,
    }
}
