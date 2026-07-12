//! `OffscreenRenderer`: surfaceless single-frame render + CPU readback
//! around the shared `SceneRenderer`, with an optional egui overlay (the
//! `--scene` mock UI). The `headless` binary's presenter.

use crate::engine::renderer::{
    DEPTH_FORMAT, SceneRenderer, UiFrame, create_depth_view, depth_attachment,
    request_adapter_device,
};
use crate::engine::scene::RenderState;

/// Maximum width or height (pixels) for the offscreen target. Matches wgpu's
/// default `max_texture_dimension_2d`; a `debug_assert` checks it against
/// the real device limit so the two cannot drift.
pub const MAX_FRAME_DIMENSION: u32 = 8192;

/// Offscreen color format. **Non-sRGB on purpose**: the windowed surface is
/// also non-sRGB, so the shader's 8-bit output stored raw already equals the
/// sRGB-encoded pixels a display shows - written verbatim to PNG it
/// reproduces the on-screen look (an sRGB target would render brighter).
/// `Rgba8Unorm` also means read-back bytes are already in RGBA order.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Renders a single scene frame offscreen and returns it as CPU pixels.
pub struct OffscreenRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    scene: SceneRenderer,
    /// egui paint backend, used only when a [`UiFrame`] is supplied. Created
    /// unconditionally - it allocates nothing until primitives render.
    egui_renderer: egui_wgpu::Renderer,
    color: wgpu::Texture,
    /// Reversed-Z depth buffer matching the color target.
    depth_view: wgpu::TextureView,
    /// CPU-mappable buffer the color texture is copied into (rows padded).
    readback: wgpu::Buffer,
    width: u32,
    height: u32,
    /// Bytes per readback row, padded up to `COPY_BYTES_PER_ROW_ALIGNMENT`.
    padded_bytes_per_row: u32,
}

impl OffscreenRenderer {
    /// Builds an offscreen renderer targeting a `width` x `height` image.
    /// Panics if the dimensions are outside `1..=MAX_FRAME_DIMENSION` (the
    /// caller validates first and reports a clean CLI error).
    pub fn new(width: u32, height: u32) -> Self {
        assert!(
            width > 0
                && height > 0
                && width <= MAX_FRAME_DIMENSION
                && height <= MAX_FRAME_DIMENSION,
            "render dimensions must be 1..={MAX_FRAME_DIMENSION}, got {width}x{height}"
        );

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        // No surface: offscreen rendering only.
        let (_adapter, device, queue) = request_adapter_device(&instance, None);

        // Guard against MAX_FRAME_DIMENSION drifting from the real device
        // limit (both come from the default wgpu limits today).
        debug_assert!(device.limits().max_texture_dimension_2d >= MAX_FRAME_DIMENSION);

        let scene = SceneRenderer::new(&device, &queue, FORMAT);

        let egui_renderer = egui_wgpu::Renderer::new(
            &device,
            FORMAT,
            egui_wgpu::RendererOptions {
                depth_stencil_format: Some(DEPTH_FORMAT),
                ..Default::default()
            },
        );

        let depth_view = create_depth_view(&device, width, height);

        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen color target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        // copy_texture_to_buffer requires each row aligned to 256 bytes.
        let padded_bytes_per_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("offscreen readback"),
            size: u64::from(padded_bytes_per_row) * u64::from(height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            device,
            queue,
            scene,
            egui_renderer,
            color,
            depth_view,
            readback,
            width,
            height,
            padded_bytes_per_row,
        }
    }

    /// Renders one frame from `render`, optionally composites an egui overlay
    /// from `ui`, and returns the frame as a tight RGBA8 image (blocking on
    /// the readback).
    pub fn render(&mut self, render: &RenderState, ui: Option<UiFrame>) -> image::RgbaImage {
        let viewport = (f64::from(self.width), f64::from(self.height));
        self.scene
            .prepare(&self.device, &self.queue, render, viewport);

        // This path always renders to completion (no early-return acquire),
        // so the windowed set-before-acquire ordering rule is trivially met;
        // set-before / free-after is kept anyway.
        let screen = ui.as_ref().map(|ui| {
            for (id, delta) in &ui.textures_delta.set {
                self.egui_renderer
                    .update_texture(&self.device, &self.queue, *id, delta);
            }
            egui_wgpu::ScreenDescriptor {
                size_in_pixels: [self.width, self.height],
                pixels_per_point: ui.pixels_per_point,
            }
        });

        let view = self
            .color
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("offscreen frame encoder"),
            });

        // egui's per-frame buffers update through the encoder before the
        // pass, yielding prologue command buffers submitted ahead of the
        // main one. Empty when there is no UI to draw.
        let egui_commands = match (ui.as_ref(), screen.as_ref()) {
            (Some(ui), Some(screen)) => self.egui_renderer.update_buffers(
                &self.device,
                &self.queue,
                &mut encoder,
                &ui.primitives,
                screen,
            ),
            _ => Vec::new(),
        };

        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("offscreen frame pass"),
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
            // egui-wgpu needs a `RenderPass<'static>`; the pass still ends at
            // this scope's close, before the encoder is finished.
            let mut pass = pass.forget_lifetime();
            self.scene.render(&mut pass);
            if let (Some(ui), Some(screen)) = (ui.as_ref(), screen.as_ref()) {
                self.egui_renderer.render(&mut pass, &ui.primitives, screen);
            }
        }

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue
            .submit(egui_commands.into_iter().chain([encoder.finish()]));

        // Free egui's released textures after submit - the just-submitted
        // frame may still reference them.
        if let Some(ui) = ui.as_ref() {
            for id in &ui.textures_delta.free {
                self.egui_renderer.free_texture(id);
            }
        }

        // Map the readback buffer and block until the GPU work + mapping
        // finish.
        let slice = self.readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .expect("poll device for readback");
        rx.recv()
            .expect("map_async callback delivered")
            .expect("readback buffer mapped");

        // Strip the per-row padding into a tight width*height*4 RGBA8 buffer.
        let data = slice.get_mapped_range();
        let row_bytes = (self.width * 4) as usize;
        let mut pixels = Vec::with_capacity(row_bytes * self.height as usize);
        for row in 0..self.height as usize {
            let start = row * self.padded_bytes_per_row as usize;
            pixels.extend_from_slice(&data[start..start + row_bytes]);
        }
        drop(data);
        self.readback.unmap();

        image::RgbaImage::from_raw(self.width, self.height, pixels)
            .expect("pixel buffer length equals width * height * 4")
    }
}
