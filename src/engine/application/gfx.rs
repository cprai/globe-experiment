//! The windowed presentation side of rendering: `Gfx` owns the GPU surface,
//! swapchain configuration, and per-frame present, wrapping the shared scene
//! core (`crate::engine::renderer::SceneRenderer`). It lives in `application`
//! (not `renderer`) because it is winit-bound - the surface is built from the
//! window and every frame outcome drives window visibility/redraw scheduling -
//! and because the `headless` binary must compile the renderer without any
//! winit code (its offscreen twin is `crate::offscreen::OffscreenRenderer`).

use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

use crate::engine::renderer::{
    DEPTH_FORMAT, SceneRenderer, UiFrame, create_depth_view, depth_attachment,
    request_adapter_device,
};
use crate::engine::simulation::RenderState;

/// The renderer: owns the GPU surface/device/queue, the scene resources
/// (pipelines, buffers, bind group), and the egui paint backend. Created once
/// via [`Gfx::init`]; each frame [`Gfx::update`] writes the uniforms from a
/// [`RenderState`] and draws the scene plus the egui overlay in a single pass.
///
/// The window is borrowed only to build the surface (and for the per-frame
/// present-notify hint); window visibility and redraw scheduling are the
/// caller's job, driven by the [`FrameOutcome`] this returns.
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
    /// paint backend. The window stays hidden during this (the caller reveals
    /// it after the first presented frame).
    pub fn init(window: Arc<Window>, display: OwnedDisplayHandle) -> Self {
        // Pass the platform display handle so the GLES/EGL backend can open
        // its display connection. Without it, GL adapter enumeration fails on
        // Wayland (winit's default on Linux, including WSL where Vulkan may
        // be absent or broken). The display handle is required by the wgpu
        // docs for GLES when presenting to a surface.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(display),
        ));
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");

        // Shared adapter+device creation (no optional features, default
        // limits). The surface is passed so the chosen adapter is guaranteed
        // able to present to it; the offscreen renderer passes `None` instead.
        let (adapter, device, queue) = request_adapter_device(&instance, Some(&surface));

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

        // Shader-module compilation, texture decode+upload, and pipeline
        // creation happen here, before the first frame; the atmosphere LUT bake
        // already ran at build time (build.rs), but the Terra/star textures are
        // decoded from their embedded JPEG/TIFF now. This work is parallelized
        // internally with rayon (see SceneRenderer::new).
        let scene = SceneRenderer::new(&device, &queue, config.format);

        // The egui overlay shares the frame pass, which now has a depth
        // attachment, so its pipeline must declare the matching depth format
        // (egui builds it depth-test-off / no-write, so the overlay still draws
        // on top regardless of depth).
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

    /// The surface size in pixels (width, height). The caller uses it to build
    /// the camera's projection aspect and to scale input. f64 to match the
    /// camera/renderer math (a pixel count is exact in either float width).
    pub fn viewport(&self) -> (f64, f64) {
        (f64::from(self.config.width), f64::from(self.config.height))
    }

    /// Renders one frame: applies egui's texture-set deltas, writes the
    /// uniforms from `render`, then draws the scene (stars -> planet impostors
    /// -> terra surface -> luna -> atmosphere -> markers, depth-buffered) and
    /// the egui overlay in a single pass, and presents. `window` is borrowed
    /// only for the pre-present hint. Returns a [`FrameOutcome`] so the
    /// caller can reveal the window / reschedule a redraw.
    ///
    /// The egui texture-set deltas are applied **first, before the surface
    /// acquire**, so they survive a frame that never presents - see the comment
    /// at the top of the body for why that ordering is load-bearing.
    pub fn update(&mut self, window: &Window, render: &RenderState, ui: UiFrame) -> FrameOutcome {
        // Apply egui's texture-set deltas (font-atlas allocation + per-glyph
        // updates) BEFORE acquiring the surface, so they are never skipped on a
        // frame that does not present. This ordering is load-bearing: egui's
        // `Context` emits each texture delta exactly once and then forgets it
        // (it assumes the renderer applied it), so a dropped delta desyncs
        // `egui_renderer` permanently with no resend. If these ran after the
        // surface acquire below and that acquire took an early-return arm
        // (Occluded / Lost / Outdated / Timeout), the first frame's full
        // font-atlas *allocation* could be lost; a later *partial* atlas update
        // (a newly rasterized glyph, `ImageDelta.pos = Some`) would then panic
        // inside egui-wgpu with "Tried to update a texture that has not been
        // allocated yet." This shows up on macOS in particular, where the
        // hidden-until-ready startup makes the very first acquire come back
        // Occluded (see `ApplicationState::redraw`). `update_texture` needs only
        // the device/queue, not the swapchain frame, so it is safe to hoist
        // here. (The matching `free` deltas stay *after* present - those
        // textures may still be referenced by this frame's draw; a `free`
        // dropped on an early-return frame only delays cleanup and is overwritten
        // cleanly if egui later reuses the id, so it is benign, unlike a dropped
        // allocation.)
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
