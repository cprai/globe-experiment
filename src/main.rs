mod globe;
mod ui;

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use globe::camera::Camera;
use globe::clock::Clock;
use globe::input::Controller;
use globe::renderer::GlobeRenderer;
use globe::satellite::Satellite;
use globe::sky::Sky;

fn main() {
    // Point satkit at the build-downloaded ephemeris data before anything
    // (App::default below builds the Sky, which reads the ephemeris).
    globe::sky::init_data_dir();

    let event_loop = EventLoop::new().expect("create event loop");
    // Frames are driven by explicit redraw requests (input, inertia,
    // egui repaints); idle means zero GPU work.
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App::default();
    event_loop.run_app(&mut app).expect("run event loop");
}

struct App {
    camera: Camera,
    /// Ephemeris-driven Sun direction + star-map orientation for the current
    /// clock time.
    sky: Sky,
    /// The tracked space station, re-propagated as the clock advances.
    satellite: Satellite,
    /// Simulation clock that drives the satellite and sky.
    clock: Clock,
    controller: Controller,
    gfx: Option<Gfx>,
}

impl Default for App {
    fn default() -> Self {
        // Build the satellite first; the clock starts at its TLE epoch, and
        // the sky is evaluated at that same time.
        let satellite = Satellite::load();
        let clock = Clock::new(satellite.epoch());
        let sky = Sky::at(&clock.now());
        Self {
            camera: Camera::default(),
            sky,
            satellite,
            clock,
            controller: Controller::default(),
            gfx: None,
        }
    }
}

/// Everything tied to the window and GPU, created once on `resumed`.
struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    globe: GlobeRenderer,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    /// Whether the window has been made visible; flips on the first
    /// presented frame.
    shown: bool,
}

impl Gfx {
    fn new(window: Arc<Window>) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("request adapter");

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("globe device"),
            // The earth/star textures are BC7-compressed at build time.
            // BC support is universal on desktop GPUs.
            required_features: wgpu::Features::TEXTURE_COMPRESSION_BC,
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .expect("request device");

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface unsupported by adapter");
        // A non-sRGB surface stores the shader's linear output raw, which
        // the display then reads as sRGB-encoded. That is what iced's
        // default `web-colors` feature did in phase 1, and every shader
        // look-tuning constant is calibrated to it; an sRGB surface
        // (hardware encode) renders visibly brighter.
        let caps = surface.get_capabilities(&adapter);
        if let Some(format) = caps.formats.iter().copied().find(|f| !f.is_srgb()) {
            config.format = format;
        }
        // The default config takes the first advertised present mode, which
        // is Mailbox on DX12 - unpaced rendering that makes scroll zoom and
        // inertia judder. Vsync paces the animation loop to the refresh
        // rate, matching iced's AutoVsync.
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);

        // Shader-module compilation, texture upload, and pipeline
        // creation happen here, before the first frame; decoding and the
        // LUT bake already ran at build time (build.rs). This work is
        // parallelized internally with rayon (see GlobeRenderer::new).
        let globe = GlobeRenderer::new(&device, &queue, config.format);

        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let egui_renderer = egui_wgpu::Renderer::new(
            &device,
            config.format,
            egui_wgpu::RendererOptions::default(),
        );

        Self {
            window,
            surface,
            device,
            queue,
            config,
            globe,
            egui_ctx,
            egui_state,
            egui_renderer,
            shown: false,
        }
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.config.width = size.width.max(1);
        self.config.height = size.height.max(1);
        self.surface.configure(&self.device, &self.config);
    }
}

impl ApplicationHandler for App {
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

        let gfx = Gfx::new(window);
        gfx.window.set_cursor(self.controller.cursor_icon());
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
                if let Some(gfx) = self.gfx.as_mut() {
                    gfx.resize(size);
                    gfx.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            event => self.handle_input(event),
        }
    }
}

impl App {
    /// Routes an input event: egui gets first claim (sliders, panel
    /// hover); whatever it doesn't consume drives the globe camera.
    fn handle_input(&mut self, event: WindowEvent) {
        let Some(gfx) = self.gfx.as_mut() else {
            return;
        };

        let response = gfx.egui_state.on_window_event(&gfx.window, &event);
        if response.repaint {
            gfx.window.request_redraw();
        }
        if response.consumed {
            return;
        }

        if self
            .controller
            .handle_event(&event, &mut self.camera, gfx.config.height as f32)
        {
            gfx.window.request_redraw();
        }
    }

    fn redraw(&mut self) {
        let Some(gfx) = self.gfx.as_mut() else {
            return;
        };

        // Step flick inertia with real frame time; while it's coasting,
        // each frame requests the next one.
        let mut animating = self
            .controller
            .tick(&mut self.camera, gfx.config.height as f32);

        let raw_input = gfx.egui_state.take_egui_input(&gfx.window);
        let full_output = gfx.egui_ctx.run_ui(raw_input, |ui| {
            ui::control_panel(ui.ctx(), &self.sky, &self.satellite, &mut self.clock)
        });
        gfx.egui_state
            .handle_platform_output(&gfx.window, full_output.platform_output);

        // Advance the simulation clock (after the UI, so this frame's
        // play/pause and speed changes apply) and re-evaluate the satellite
        // and the ephemeris-driven sky at the new time. A running clock is
        // another "animating" source: it keeps requesting frames; when paused
        // it advances nothing and the app can go idle.
        let clock_running = self.clock.tick();
        if clock_running {
            let now = self.clock.now();
            self.satellite.update_to(&now);
            self.sky = Sky::at(&now);
        }
        animating |= clock_running;

        // egui resets the cursor icon every frame; restore the globe's
        // grab cursor whenever the pointer isn't on the panel.
        if !gfx.egui_ctx.is_pointer_over_egui() {
            gfx.window.set_cursor(self.controller.cursor_icon());
        }

        let frame = match gfx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                gfx.surface.configure(&gfx.device, &gfx.config);
                gfx.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout => {
                gfx.window.request_redraw();
                return;
            }
            // Hidden/minimized: skip the frame; the next expose event
            // requests a redraw. If the first frame lands here (some
            // backends report the still-hidden window as occluded), show
            // the window and retry rather than deadlocking invisible.
            wgpu::CurrentSurfaceTexture::Occluded => {
                if !gfx.shown {
                    gfx.shown = true;
                    gfx.window.set_visible(true);
                    gfx.window.request_redraw();
                }
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                panic!("surface validation error on frame acquire")
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let viewport = (gfx.config.width as f32, gfx.config.height as f32);
        gfx.globe.prepare(
            &gfx.queue,
            &self.camera,
            &self.sky,
            &self.satellite,
            viewport,
        );

        let primitives = gfx
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [gfx.config.width, gfx.config.height],
            pixels_per_point: full_output.pixels_per_point,
        };

        for (id, delta) in &full_output.textures_delta.set {
            gfx.egui_renderer
                .update_texture(&gfx.device, &gfx.queue, *id, delta);
        }

        let mut encoder = gfx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });
        let egui_commands = gfx.egui_renderer.update_buffers(
            &gfx.device,
            &gfx.queue,
            &mut encoder,
            &primitives,
            &screen,
        );

        {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // egui-wgpu needs a `RenderPass<'static>`; the pass still ends
            // at this scope's close, before the encoder is finished.
            let mut render_pass = render_pass.forget_lifetime();

            gfx.globe.render(&mut render_pass);
            gfx.egui_renderer
                .render(&mut render_pass, &primitives, &screen);
        }

        gfx.queue
            .submit(egui_commands.into_iter().chain([encoder.finish()]));
        gfx.window.pre_present_notify();
        frame.present();

        // First frame is on the surface - reveal the window.
        if !gfx.shown {
            gfx.shown = true;
            gfx.window.set_visible(true);
        }

        for id in &full_output.textures_delta.free {
            gfx.egui_renderer.free_texture(id);
        }

        let egui_repaint = full_output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|output| output.repaint_delay.is_zero());

        if animating || egui_repaint {
            gfx.window.request_redraw();
        }
    }
}
