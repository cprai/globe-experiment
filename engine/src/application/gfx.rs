//! `Gfx`: the windowed presentation side of rendering - GPU surface,
//! swapchain configuration, per-frame present - around the shared
//! `SceneRenderer`. Lives in `application` (not `renderer`) because it is
//! winit-bound.

use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

use crate::renderer::{
    DEPTH_FORMAT, SceneRenderer, UiFrame, create_depth_view, depth_attachment,
    request_adapter_device,
};
use crate::scene::RenderState;

/// Owns the GPU surface/device/queue, the scene resources, and the egui
/// paint backend. Window visibility and redraw scheduling are the caller's
/// job, driven by the [`FrameOutcome`] each [`Gfx::update`] returns.
pub struct Gfx {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    scene: SceneRenderer,
    egui_renderer: egui_wgpu::Renderer,
    /// Reversed-Z depth buffer, recreated on resize to match the surface size.
    depth_view: wgpu::TextureView,
}

/// What happened when a frame was submitted, so the caller can drive window
/// visibility and redraw scheduling without the renderer touching the window.
pub enum FrameOutcome {
    /// The frame was presented to the surface.
    Presented,
    /// The surface was occluded (hidden/minimized); nothing was drawn.
    Occluded,
    /// Acquiring the surface texture failed (lost/outdated/timeout); the
    /// surface was reconfigured where needed. The caller should redraw.
    Reconfigured,
}

impl Gfx {
    /// Builds the GPU surface/device, the scene resources, and the egui
    /// paint backend.
    pub fn init(window: Arc<Window>, display: OwnedDisplayHandle) -> Self {
        // The display handle lets the GLES/EGL backend open its display
        // connection; without it, GL adapter enumeration fails on Wayland
        // (winit's Linux default, incl. WSL where Vulkan may be absent).
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(display),
        ));
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");

        // The surface is passed so the chosen adapter can present to it; the
        // offscreen renderer passes `None`.
        let (adapter, device, queue) = request_adapter_device(&instance, Some(&surface));

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface unsupported by adapter");
        // Non-sRGB on purpose: every shader look-tuning constant is
        // calibrated to a non-sRGB surface (linear output stored raw, read
        // as sRGB by the display); an sRGB surface renders visibly brighter.
        let caps = surface.get_capabilities(&adapter);
        if let Some(format) = caps.formats.iter().copied().find(|f| !f.is_srgb()) {
            config.format = format;
        }
        // The default present mode is Mailbox on DX12 - unpaced rendering
        // that makes scroll zoom and inertia judder; vsync paces the loop.
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);

        let scene = SceneRenderer::new(&device, &queue, config.format);

        // The egui overlay shares the frame pass, which has a depth
        // attachment, so its pipeline must declare the matching depth format
        // (egui builds it depth-test-off / no-write).
        let egui_renderer = egui_wgpu::Renderer::new(
            &device,
            config.format,
            egui_wgpu::RendererOptions {
                depth_stencil_format: Some(DEPTH_FORMAT),
                ..Default::default()
            },
        );

        let depth_view = create_depth_view(&device, config.width, config.height);

        Self {
            surface,
            device,
            queue,
            config,
            scene,
            egui_renderer,
            depth_view,
        }
    }

    /// Reconfigures the surface to a new size and resizes the depth buffer to
    /// match. The caller requests a redraw.
    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        self.config.width = size.width.max(1);
        self.config.height = size.height.max(1);
        self.surface.configure(&self.device, &self.config);
        self.depth_view = create_depth_view(&self.device, self.config.width, self.config.height);
    }

    /// The surface size in pixels (width, height). f64 to match the
    /// camera/renderer math (a pixel count is exact in either float width).
    pub fn viewport(&self) -> (f64, f64) {
        (f64::from(self.config.width), f64::from(self.config.height))
    }

    /// Renders one frame: applies egui's texture-set deltas, writes the
    /// uniforms from `render`, draws the scene plus the egui overlay in a
    /// single depth-buffered pass, and presents.
    pub fn update(&mut self, window: &Window, render: &RenderState, ui: UiFrame) -> FrameOutcome {
        // Texture-set deltas apply BEFORE the surface acquire: egui emits
        // each delta exactly once, so one dropped on an early-return acquire
        // (e.g. macOS's first-frame Occluded) is lost for good and a later
        // partial atlas update panics in egui-wgpu ("Tried to update a
        // texture that has not been allocated yet"). `update_texture` needs
        // only device/queue, so hoisting it is safe. The `free` deltas stay
        // after present (the frame may still reference those textures; a
        // dropped free is benign).
        for (id, delta) in &ui.textures_delta.set {
            self.egui_renderer
                .update_texture(&self.device, &self.queue, *id, delta);
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return FrameOutcome::Reconfigured;
            }
            wgpu::CurrentSurfaceTexture::Timeout => return FrameOutcome::Reconfigured,
            wgpu::CurrentSurfaceTexture::Occluded => return FrameOutcome::Occluded,
            wgpu::CurrentSurfaceTexture::Validation => {
                panic!("surface validation error on frame acquire")
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let viewport = self.viewport();
        self.scene
            .prepare(&self.device, &self.queue, render, viewport);

        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: ui.pixels_per_point,
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });
        let egui_commands = self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &ui.primitives,
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
                depth_stencil_attachment: Some(depth_attachment(&self.depth_view)),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // egui-wgpu needs a `RenderPass<'static>`; the pass still ends
            // at this scope's close, before the encoder is finished.
            let mut render_pass = render_pass.forget_lifetime();

            self.scene.render(&mut render_pass);
            self.egui_renderer
                .render(&mut render_pass, &ui.primitives, &screen);
        }

        self.queue
            .submit(egui_commands.into_iter().chain([encoder.finish()]));
        window.pre_present_notify();
        frame.present();

        for id in &ui.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }

        FrameOutcome::Presented
    }
}
