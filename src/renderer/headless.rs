//! Headless single-frame renderer: draws the globe scene to an offscreen
//! texture and reads it back to CPU pixels, with no window, surface, egui, or
//! present. Used by the `render` CLI mode (see `crate::snapshot`).
//!
//! It shares the scene core ([`GlobeRenderer`]) and the device-creation path
//! ([`request_adapter_device`]) with the windowed [`Gfx`](super::Gfx); the only
//! differences are the presentation target (an owned color texture + a readback
//! buffer instead of a swapchain surface) and the absence of any UI. The draw
//! sequence is identical: clear to black, then stars -> surface -> atmosphere
//! (markers are skipped because render mode tracks none).

use super::{GlobeRenderer, MAX_FRAME_DIMENSION, request_adapter_device};
use crate::simulation::RenderState;

/// Offscreen color format. **Non-sRGB on purpose.** Every look-tuning constant
/// in `globe.wgsl` is calibrated to the windowed surface, which is also
/// non-sRGB (`Gfx::init` picks `!is_srgb()`). On a non-sRGB target the shader's
/// 8-bit output is stored raw, and those bytes already equal the sRGB-encoded
/// pixels a display shows - so writing them verbatim into a PNG (which viewers
/// read as sRGB) reproduces the on-screen look. An sRGB target here would
/// hardware-encode the output and render visibly brighter than the window.
/// `Rgba8Unorm` (rather than the surface's usual `Bgra8Unorm`) also means the
/// read-back bytes are already in RGBA order, with no channel swap.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Renders a single globe frame offscreen and returns it as CPU pixels. Built
/// once per `render` invocation, used for one frame, then dropped.
pub struct HeadlessRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    globe: GlobeRenderer,
    /// The offscreen render target (RENDER_ATTACHMENT | COPY_SRC).
    color: wgpu::Texture,
    /// CPU-mappable buffer the color texture is copied into (rows padded).
    readback: wgpu::Buffer,
    width: u32,
    height: u32,
    /// Bytes per readback row, padded up to `COPY_BYTES_PER_ROW_ALIGNMENT`.
    padded_bytes_per_row: u32,
}

impl HeadlessRenderer {
    /// Builds a headless renderer targeting a `width` x `height` image. Creates
    /// its own surfaceless device (BC7 feature, same as the windowed path),
    /// the globe scene resources, the offscreen color texture, and the readback
    /// buffer. Panics if the dimensions are outside `1..=MAX_FRAME_DIMENSION`
    /// (the caller validates first and reports a clean CLI error).
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

        // Guard against MAX_FRAME_DIMENSION drifting from the real device limit
        // (both come from the default wgpu limits today).
        debug_assert!(device.limits().max_texture_dimension_2d >= MAX_FRAME_DIMENSION);

        let globe = GlobeRenderer::new(&device, &queue, FORMAT);

        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("headless color target"),
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
            label: Some("headless readback"),
            size: u64::from(padded_bytes_per_row) * u64::from(height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            device,
            queue,
            globe,
            color,
            readback,
            width,
            height,
            padded_bytes_per_row,
        }
    }

    /// Renders one frame from `render` and returns it as an RGBA8 image. Writes
    /// the uniforms, draws the scene into the offscreen target in a single pass
    /// (no depth, draw-order occlusion - same invariant as the windowed path),
    /// copies the result into the readback buffer, blocks until it is mapped,
    /// then un-pads the rows into a tight RGBA8 buffer.
    pub fn render(&mut self, render: &RenderState) -> image::RgbaImage {
        let viewport = (self.width as f32, self.height as f32);
        self.globe
            .prepare(&self.device, &self.queue, render, viewport);

        let view = self
            .color
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("headless frame encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("headless frame pass"),
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
            self.globe.render(&mut pass);
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
        self.queue.submit([encoder.finish()]);

        // Map the readback buffer and block until the GPU work + mapping finish.
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
