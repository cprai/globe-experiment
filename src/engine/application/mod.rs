//! The application layer: windowing, the winit event loop, per-frame redraw
//! orchestration, and the windowed presenter. It is generic over any
//! `S: Scene + CameraControl + CameraView + UIDrawable`, owns the
//! windowed `Gfx` renderer (the `gfx` submodule), advances the simulation,
//! and drives each render.
//!
//! The application keeps NO camera or input state: each winit input event is
//! translated **statelessly** into one device-neutral `CameraControl`-trait
//! call (see `translate_camera_event`), so all camera state - the rig, drag/
//! flick/zoom animation, even cursor tracking - lives behind the scene's
//! `CameraControl`/`CameraView` impls (usually a `PtzCamera`). Swapping or
//! extending the input scheme (gamepad, touch) means a new trait method plus
//! a translation arm here, nothing more. The simulation and renderer only
//! ever consume the resolved `RenderState`; the application never touches
//! the `CelestialSphere`.

mod gfx;

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{CursorIcon, Window, WindowId};

use crate::engine::camera::{CameraControl, CameraView, CursorHint, PointerButton, ScrollDelta};
use crate::engine::renderer::UiFrame;
use crate::engine::scene::{Scene, SceneClock};
use crate::engine::ui::{self, UIDrawable};
use gfx::{FrameOutcome, Gfx};

/// Runs the winit event loop to completion, driving `app`. Frames are driven by
/// explicit redraw requests (input, inertia, egui repaints); idle means zero
/// GPU work.
pub fn run<S: Scene + SceneClock + CameraControl + CameraView + UIDrawable>(
    mut app: ApplicationState<S>,
) {
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
/// `egui_winit::State`), the simulation (which carries its own camera behind
/// the `CameraControl`/`CameraView` traits), and the renderer. The
/// window/egui_state/gfx are created on `resumed`. Generic over
/// `S: Scene + CameraControl + CameraView + UIDrawable` so any scene
/// can be plugged in without changing the application layer.
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
    /// Builds the application around an already-constructed simulation (which
    /// carries its own camera - a scene that frames a specific event on
    /// launch seeds its `PtzCamera` in its `new()`). The window, egui platform
    /// state, and renderer are created later, on the first `resumed`.
    pub fn new(simulation: S) -> Self {
        let egui_ctx = egui::Context::default();
        // Stamp the Apollo-panel theme onto the context once; the live UI and a
        // headless mock share the same `install_theme`, so they look identical.
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
        window.set_cursor(cursor_icon(self.simulation.cursor_hint()));

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

impl<S: Scene + SceneClock + CameraControl + CameraView + UIDrawable> ApplicationState<S> {
    /// Routes an input event: egui gets first claim (sliders, panel
    /// hover); whatever it doesn't consume is translated into the
    /// scene's `CameraControl` impl.
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

        if translate_camera_event(&mut self.simulation, &event, gfx.viewport().1) {
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

        // Step camera animation (flick inertia, zoom glide) with real frame
        // time; while something is coasting, each frame requests the next one.
        let mut animating = self.simulation.tick(gfx.viewport().1);

        // Advance the simulation: tick_scene steps the clock, then runs the
        // scene's own advance (the shared tick lives in the Scene trait's
        // default, so scenes handle only what is unique to them).
        // (This frame's play/pause and speed edits come from the previous
        // frame's UI, applied here - a one-frame, ~16 ms delay, imperceptible.)
        // A running clock is another "animating" source - it keeps requesting
        // frames; when paused nothing advances and the app can go idle.
        animating |= self.simulation.tick_scene();

        // Produce this frame's RenderState. The scene's CameraView impl
        // resolves its own view - it re-aims at the frame's target and
        // builds the rig against its own celestial sphere - so the
        // application never touches the ephemeris; the renderer rebuilds the
        // projection from the rig in the returned state.
        let render_state = self.simulation.frame_state();

        // Run the egui UI: pull the scene's drawable panels ONCE per frame
        // (readouts re-derived at the same clock instant as the frame_state
        // above, so they match the rendered markers), then render them inside
        // run_ui, firing the elements' callbacks - each receives
        // `&mut simulation` at fire time - for any interaction. Building the
        // panels outside run_ui is load-bearing: egui's discard pass
        // (max_passes = 2) re-runs the closure, and only fixed build-time
        // callback snapshots keep a twice-fired callback idempotent (see
        // ui::control_panel). The logic and tessellation live here (the
        // renderer only draws the primitives). The panels are owned (they
        // never borrow the scene), so `&mut panels` and `&mut simulation`
        // coexist in the closure, and both are disjoint from the `run_ui`
        // receiver borrow of `self.egui_ctx`.
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

/// Translates one winit window event into the matching device-neutral
/// `CameraControl`-trait call, statelessly - raw positions/deltas pass
/// straight through and nothing is remembered here (cursor tracking lives in
/// the camera, which is why presses carry no position: winit gives them
/// none). Returns whether the camera changed and a redraw is needed.
fn translate_camera_event<C: CameraControl>(
    camera: &mut C,
    event: &WindowEvent,
    viewport_height: f64,
) -> bool {
    match event {
        WindowEvent::MouseInput { state, button, .. } => {
            // Only the buttons the camera vocabulary names; middle/back/
            // forward are dropped here (no camera has a gesture for them).
            let button = match button {
                MouseButton::Left => PointerButton::Left,
                MouseButton::Right => PointerButton::Right,
                _ => return false,
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
            // lines, macOS precision trackpads pixels); the per-variant zoom
            // feel lives with the camera.
            let delta = match delta {
                MouseScrollDelta::LineDelta(_, y) => ScrollDelta::Lines(f64::from(*y)),
                MouseScrollDelta::PixelDelta(position) => ScrollDelta::Pixels(position.y),
            };
            camera.scroll(delta)
        }
        _ => false,
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
