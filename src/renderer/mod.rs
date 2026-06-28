mod headless;
mod mesh;

use std::sync::Arc;

use rayon::prelude::*;
use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

use crate::earth;
use crate::planet;
use crate::simulation::RenderState;
use mesh::Vertex;

pub use headless::HeadlessRenderer;

const STACKS: u32 = 64;
const SLICES: u32 = 128;

/// Radius of the on-screen station marker, in pixels.
const MARKER_RADIUS_PX: f32 = 6.0;

/// Depth buffer format. A 32-bit float depth, paired with the camera's
/// reversed-Z projection (near -> 1, far -> 0), cleared to 0.0 and tested
/// `Greater`. This is what lets the Earth correctly occlude the much more
/// distant Moon across the scene's enormous near/far span (see
/// `Camera::view_proj`).
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// The Sun's angular radius seen from Earth/Moon (~0.266 deg), in radians. Sets
/// the soft penumbra width of the analytic eclipse shadows in the shader.
const SUN_ANGULAR_RADIUS_RAD: f32 = 0.004652;

/// Creates a depth texture view sized to the render target. Recreated whenever
/// the target is resized (the depth attachment must match the color size).
fn create_depth_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth texture"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// The reversed-Z depth attachment, cleared to 0.0 (the far plane). Shared by
/// the windowed and headless passes so they depth-test identically.
fn depth_attachment(view: &wgpu::TextureView) -> wgpu::RenderPassDepthStencilAttachment<'_> {
    wgpu::RenderPassDepthStencilAttachment {
        view,
        depth_ops: Some(wgpu::Operations {
            load: wgpu::LoadOp::Clear(0.0),
            store: wgpu::StoreOp::Store,
        }),
        stencil_ops: None,
    }
}

/// Maximum width or height (pixels) for a single-frame [`HeadlessRenderer`]
/// target. Matches wgpu's default 2D texture dimension limit
/// (`wgpu::Limits::default().max_texture_dimension_2d`, which the globe device
/// requests); the offscreen color texture cannot exceed it. `HeadlessRenderer`
/// `debug_assert`s this against the real device limit so the two cannot drift.
pub const MAX_FRAME_DIMENSION: u32 = 8192;

/// The renderer: owns the GPU surface/device/queue, the globe scene resources
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
    globe: GlobeRenderer,
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

/// Render-ready egui paint output, produced by the caller (which owns the
/// `egui::Context` + `egui_winit::State`) and consumed by [`Gfx::update`].
pub struct UiFrame {
    pub primitives: Vec<egui::ClippedPrimitive>,
    pub textures_delta: egui::TexturesDelta,
    pub pixels_per_point: f32,
}

impl Gfx {
    /// Builds the GPU surface/device, the globe scene resources, and the egui
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
        // able to present to it; the headless renderer passes `None` instead.
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
        // already ran at build time (build.rs), but the Earth/star textures are
        // decoded from their embedded JPEG/TIFF now. This work is parallelized
        // internally with rayon (see GlobeRenderer::new).
        let globe = GlobeRenderer::new(&device, &queue, config.format);

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
            globe,
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
    /// the camera's projection aspect and to scale input.
    pub fn viewport(&self) -> (f32, f32) {
        (self.config.width as f32, self.config.height as f32)
    }

    /// Renders one frame: applies egui's texture-set deltas, writes the
    /// uniforms from `render`, then draws the scene (stars -> surface -> moon
    /// -> atmosphere -> marker, depth-buffered) and the egui overlay in a
    /// single pass, and presents. `window` is borrowed only for the
    /// pre-present hint. Returns a [`FrameOutcome`] so the caller can
    /// reveal the window / reschedule a redraw.
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
        self.globe
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

            self.globe.render(&mut render_pass);
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

/// Requests a high-performance adapter and a device with **no** optional
/// features and default limits. Shared by the windowed [`Gfx`] and the headless
/// [`HeadlessRenderer`] so both create the device identically. Pass
/// `Some(&surface)` for the windowed path (the adapter must be able to present
/// to that surface) or `None` for offscreen rendering. `instance` is borrowed
/// because each caller owns it (the windowed path needs it to build the surface
/// first).
///
/// No GPU texture-compression feature is requested: the Earth/star textures are
/// uploaded uncompressed (`Rgba8Unorm`/`Rgba8UnormSrgb`, decoded at runtime by
/// `upload_image`), so the renderer runs on every backend and GPU - including
/// those without BC/ASTC (Apple Silicon, ARM SoCs) - with no per-platform
/// format selection.
fn request_adapter_device(
    instance: &wgpu::Instance,
    compatible_surface: Option<&wgpu::Surface<'_>>,
) -> (wgpu::Adapter, wgpu::Device, wgpu::Queue) {
    // Prefer a high-performance adapter (Vulkan/Metal/DX12). On WSL+X11 or
    // other environments where Vulkan is absent, fall back to any adapter
    // across all backends (GL, software rasterizer) that can present to the
    // surface. Set WGPU_BACKEND=gl to force the OpenGL backend explicitly.
    // wgpu 29: request_adapter returns Result<Adapter, RequestAdapterError>.
    // Convert to Option so or_else can provide the enumerate_adapters fallback
    // without needing to match on the error type.
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface,
        force_fallback_adapter: false,
    }))
    .ok()
    .or_else(|| {
        pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
            .into_iter()
            .find(|a| compatible_surface.is_none_or(|s| a.is_surface_supported(s)))
    })
    .expect("no GPU adapter found; set WGPU_BACKEND=gl to force the OpenGL backend");

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("globe device"),
        // No optional features: textures are uploaded uncompressed, so no
        // BC/ASTC support is needed (maximum platform compatibility).
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
    }))
    .expect("request device");

    (adapter, device, queue)
}

/// Per-frame shader uniforms. Layout must match `Uniforms` in globe.wgsl:
/// vec3 fields are padded to 16-byte alignment.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [f32; 16],
    /// Camera eye in the render frame (km) = relative to the camera target.
    camera_pos: [f32; 3],
    _pad0: f32,
    /// Inverse star map rotation (world -> galactic texture frame);
    /// mat3x3 columns padded to vec4 stride.
    star_rot_inv: [[f32; 4]; 3],
    /// Marker params shared by every marker: x,y = viewport size px,
    /// z = radius px, w = unused. (Per-marker position/visibility is
    /// per-instance, in the marker instance buffer, not here.)
    marker: [f32; 4],
    /// Moon body-fixed -> world rotation; mat3x3 columns padded to vec4 stride.
    moon_rot: [[f32; 4]; 3],
    /// Moon center in the render frame (km) = relative to the camera target.
    moon_pos: [f32; 3],
    _pad2: f32,
    /// Eclipse params: x = Moon mean radius km, y = Earth mean radius km,
    /// z = Sun angular radius rad, w = unused.
    moon_params: [f32; 4],
    /// Sun position in the render frame (km) = relative to the camera target.
    /// Every lit pass derives its Sun direction from this; there is no
    /// Earth-fixed `sun_dir`.
    sun_pos: [f32; 3],
    _pad4: f32,
}

/// Per-planet model uniform (group 1). Layout must match `PlanetUniform` in
/// globe.wgsl: the mat3x3 columns are padded to vec4 stride and the vec3 center
/// is followed by the radius (filling its pad slot).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PlanetUniform {
    /// Body-fixed -> world rotation; mat3x3 columns padded to vec4 stride.
    rot: [[f32; 4]; 3],
    /// Planet center in the render frame (km) = relative to the camera target.
    /// For the ORBITED planet this is exactly zero (its center IS the render
    /// origin), so its mesh is drawn in pure local coordinates - the key to
    /// killing the far-planet f32 jitter.
    pos: [f32; 3],
    /// Planet mean radius (km).
    radius_km: f32,
}

/// Long-lived GPU resources for one planet: its mesh, its per-frame model
/// uniform, and the group-1 bind group (uniform + texture + sampler) bound when
/// it is drawn. One per planet, in `planet::ALL` order.
struct PlanetGpu {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    /// Per-planet `PlanetUniform`, rewritten each frame in `prepare`.
    uniform: wgpu::Buffer,
    /// group-1 bind group: this planet's uniform + texture + the shared
    /// sampler.
    bind_group: wgpu::BindGroup,
}

/// One on-screen satellite marker, as instance data for the marker pipeline.
/// Layout must match the marker instance attributes in `vs_marker`
/// (globe.wgsl). One instance is drawn per tracked satellite.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MarkerInstance {
    /// World-frame position (km).
    position: [f32; 3],
    /// Visible flag: 1.0 = drawn, 0.0 = hidden (occluded by the globe; the
    /// vertex shader pushes it off-screen).
    visible: f32,
}

/// Initial marker-instance buffer capacity (number of satellites). The buffer
/// grows on demand if more are tracked; this just avoids a first-frame
/// reallocation for the common small counts.
const INITIAL_MARKER_CAPACITY: u32 = 8;

/// Allocates a marker-instance vertex buffer sized for `capacity` markers.
fn make_marker_buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("marker instances"),
        size: u64::from(capacity) * std::mem::size_of::<MarkerInstance>() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Owns every long-lived wgpu object for the globe: textures, LUTs,
/// mesh buffers, and the three render pipelines. A private scene helper owned
/// by [`Gfx`].
struct GlobeRenderer {
    render_pipeline: wgpu::RenderPipeline,
    atmosphere_pipeline: wgpu::RenderPipeline,
    stars_pipeline: wgpu::RenderPipeline,
    moon_pipeline: wgpu::RenderPipeline,
    marker_pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    /// Lunar mesh (triaxial ellipsoid, body-fixed frame). Separate buffers from
    /// the globe because the Moon has its own geometry and is drawn with its
    /// own model transform (`moon_rot` + `moon_pos_world`).
    moon_vertices: wgpu::Buffer,
    moon_indices: wgpu::Buffer,
    moon_index_count: u32,
    /// Planet pipeline (`vs_planet`/`fs_planet`), shared by all seven planets;
    /// each draw swaps its group-1 bind group + mesh.
    planet_pipeline: wgpu::RenderPipeline,
    /// Per-planet GPU resources, in `planet::ALL` order. Always built (textures
    /// upload at init), but only drawn when `RenderState::planets` is
    /// non-empty.
    planets: Vec<PlanetGpu>,
    /// Number of planets to draw this frame (0 except the solar-system
    /// scenario).
    planet_count: u32,
    /// Whether to draw the Earth system this frame: the Earth surface, the
    /// atmosphere, the Moon, and any satellite markers. True only when the
    /// camera target is Earth or the Moon (render origin at the Earth, so their
    /// absolute meshes are also the local meshes). When orbiting a planet these
    /// are skipped - the Earth's atmosphere physics is Earth-centered and
    /// meaningless billions of km away, and drawing it there would otherwise
    /// need the absolute (imprecise) world.
    draw_earth_system: bool,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Per-satellite marker instance data (position + visibility), drawn
    /// instanced. Grows on demand in `prepare`; not in the bind group, so
    /// growing it never touches `bind_group`.
    markers: wgpu::Buffer,
    /// Number of markers `markers` can hold without reallocation.
    marker_capacity: u32,
    /// Number of markers written for the current frame (instances to draw).
    marker_count: u32,
}

impl GlobeRenderer {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let mesh = mesh::wgs84_ellipsoid(STACKS, SLICES);

        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globe vertices"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globe indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let moon_mesh = mesh::moon_ellipsoid(STACKS, SLICES);
        let moon_vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("moon vertices"),
            contents: bytemuck::cast_slice(&moon_mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let moon_indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("moon indices"),
            contents: bytemuck::cast_slice(&moon_mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globe uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let markers = make_marker_buffer(device, INITIAL_MARKER_CAPACITY);

        // The five Earth/star textures are downloaded verbatim by the build
        // script (original JPEG/TIFF) and embedded; they are decoded with the
        // `image` crate and uploaded as uncompressed RGBA8 here - no GPU
        // compression feature required (see request_adapter_device). The three
        // atmosphere LUTs are still baked into f16 KTX2 by the build script and
        // uploaded as-is. Each entry's `TexKind` tells the parallel loader which
        // path to take: an sRGB color image, a linear data image, or an f16 LUT.
        //
        // The eight loads are mutually independent, and shader-module
        // compilation (naga parse + validation) is independent of all of them,
        // so the module is compiled on one rayon task while the textures decode
        // and upload in parallel across the rest of the pool. Device, Queue, and
        // the produced views/module are all Send + Sync. (Decoding 33 MP images
        // is CPU-heavy, so this parallelism matters more than it did for the old
        // memcpy-only BC7 uploads.)
        let texture_inputs: [(&str, &[u8], TexKind); 9] = [
            (
                "earth day texture",
                include_bytes!(concat!(env!("OUT_DIR"), "/8k_earth_daymap.jpg")),
                TexKind::ColorSrgb,
            ),
            (
                "earth night texture",
                include_bytes!(concat!(env!("OUT_DIR"), "/8k_earth_nightmap.jpg")),
                TexKind::ColorSrgb,
            ),
            (
                "earth normal texture",
                include_bytes!(concat!(env!("OUT_DIR"), "/8k_earth_normal_map.tif")),
                TexKind::DataLinear,
            ),
            (
                "earth specular texture",
                include_bytes!(concat!(env!("OUT_DIR"), "/8k_earth_specular_map.tif")),
                TexKind::DataLinear,
            ),
            // The atmosphere LUTs are baked by the build script (see
            // build.rs::bake_luts) - uploaded as f16 KTX2.
            (
                "transmittance lut",
                include_bytes!(concat!(env!("OUT_DIR"), "/transmittance.ktx2")),
                TexKind::Lut,
            ),
            (
                "inscatter rayleigh lut",
                include_bytes!(concat!(env!("OUT_DIR"), "/inscatter_rayleigh.ktx2")),
                TexKind::Lut,
            ),
            (
                "inscatter mie lut",
                include_bytes!(concat!(env!("OUT_DIR"), "/inscatter_mie.ktx2")),
                TexKind::Lut,
            ),
            (
                "stars texture",
                include_bytes!(concat!(env!("OUT_DIR"), "/8k_stars_milky_way.jpg")),
                TexKind::ColorSrgb,
            ),
            (
                "moon texture",
                include_bytes!(concat!(env!("OUT_DIR"), "/8k_moon.jpg")),
                TexKind::ColorSrgb,
            ),
        ];

        let (module, views) = rayon::join(
            || {
                device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("globe shader"),
                    source: wgpu::ShaderSource::Wgsl(
                        include_str!("../../shaders/globe.wgsl").into(),
                    ),
                })
            },
            || {
                texture_inputs
                    .into_par_iter()
                    .map(|(label, bytes, kind)| match kind {
                        TexKind::ColorSrgb => upload_image(device, queue, label, bytes, true),
                        TexKind::DataLinear => upload_image(device, queue, label, bytes, false),
                        TexKind::Lut => upload_ktx2(device, queue, label, bytes),
                    })
                    .collect::<Vec<_>>()
            },
        );

        // par_iter preserves input order, so the views line up with
        // `texture_inputs` above and the bindings below.
        let [
            day_view,
            night_view,
            normal_view,
            specular_view,
            transmittance_view,
            inscatter_rayleigh_view,
            inscatter_mie_view,
            stars_view,
            moon_view,
        ]: [wgpu::TextureView; 9] = views
            .try_into()
            .expect("upload_ktx2 returns one view per input");

        let lut_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("transmittance lut sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("earth sampler"),
            // Repeat across the dateline seam, clamp at the poles.
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globe bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globe bind group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&day_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&night_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&normal_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&specular_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&transmittance_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(&lut_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&inscatter_rayleigh_view),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(&inscatter_mie_view),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(&stars_view),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: wgpu::BindingResource::TextureView(&moon_view),
                },
            ],
        });

        // --- Planets (group 1) ---
        // Each planet's texture + per-planet model uniform live in their own
        // bind group, used only by the planet pipeline, so the seven planet
        // textures never enter the shared group-0 layout (whose 9 sampled
        // textures stay well under the portable 16-per-stage limit, leaving room
        // for Saturn's rings later). The textures decode in parallel like the
        // others. Order matches planet::ALL (and the build.rs download list).
        let planet_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("planet bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        // The embedded planet albedo maps, in planet::ALL order. The literal
        // include_bytes! paths must match `Planet::texture_file()` (the single
        // source of the planet<->file mapping), which is also used as the upload
        // label below; build.rs downloads exactly these names into OUT_DIR.
        let planet_texture_bytes: [&[u8]; 7] = [
            include_bytes!(concat!(env!("OUT_DIR"), "/8k_mercury.jpg")),
            include_bytes!(concat!(env!("OUT_DIR"), "/8k_venus_surface.jpg")),
            include_bytes!(concat!(env!("OUT_DIR"), "/8k_mars.jpg")),
            include_bytes!(concat!(env!("OUT_DIR"), "/8k_jupiter.jpg")),
            include_bytes!(concat!(env!("OUT_DIR"), "/8k_saturn.jpg")),
            include_bytes!(concat!(env!("OUT_DIR"), "/2k_uranus.jpg")),
            include_bytes!(concat!(env!("OUT_DIR"), "/2k_neptune.jpg")),
        ];
        let planet_views: Vec<wgpu::TextureView> = planet::ALL
            .par_iter()
            .zip(planet_texture_bytes.par_iter())
            .map(|(body, &bytes)| upload_image(device, queue, body.texture_file(), bytes, true))
            .collect();

        let planets: Vec<PlanetGpu> = planet::ALL
            .iter()
            .zip(planet_views)
            .map(|(&body, view)| {
                let planet_mesh = mesh::planet_ellipsoid(STACKS, SLICES, body);
                let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("planet vertices"),
                    contents: bytemuck::cast_slice(&planet_mesh.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("planet indices"),
                    contents: bytemuck::cast_slice(&planet_mesh.indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
                let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("planet uniform"),
                    size: std::mem::size_of::<PlanetUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                // The planet's group-1 bind group reuses the shared `sampler`
                // (repeat U / clamp V), the same wrap the globe + moon use.
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("planet bind group"),
                    layout: &planet_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                });
                PlanetGpu {
                    vertices,
                    indices,
                    index_count: planet_mesh.indices.len() as u32,
                    uniform,
                    bind_group,
                }
            })
            .collect();

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("globe pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        // The planet pipeline binds group 0 (shared frame uniforms) AND group 1
        // (the per-planet uniform + texture), so it needs its own layout.
        let planet_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("planet pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout), Some(&planet_bind_group_layout)],
            immediate_size: 0,
        });

        // Reversed-Z depth state, parameterized per pass. The solid bodies
        // (Earth surface, Moon) write depth and test `Greater` (nearer = larger
        // depth) so the Earth occludes the far-off Moon. The backdrop, the
        // additive atmosphere, and the screen-space markers neither write nor
        // test depth (`Always`, no write) - they keep their exact draw-order
        // behavior, layered by the order in `render`.
        let depth_state =
            |write_enabled: bool, compare: wgpu::CompareFunction| wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(write_enabled),
                depth_compare: Some(compare),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            };

        // The three pipelines share the module and layout but each does
        // independent backend pipeline-state compilation, so they build
        // concurrently. (&Device/&ShaderModule/&PipelineLayout are Sync,
        // so the shared borrows below are sound across rayon tasks.)
        let make_render_pipeline = || {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("globe pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x3,
                            1 => Float32x3,
                            2 => Float32x2,
                        ],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(depth_state(true, wgpu::CompareFunction::Greater)),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let make_atmosphere_pipeline = || {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("atmosphere pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_atmosphere"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x3,
                            1 => Float32x3,
                            2 => Float32x2,
                        ],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_atmosphere"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        // Additive: scattering brightens what's behind it.
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    // Render the far side of the shell so it spans the
                    // whole silhouette, beyond the planet's limb.
                    cull_mode: Some(wgpu::Face::Front),
                    ..Default::default()
                },
                // Additive aerial perspective over the disk: keep its exact
                // draw-order look (no depth test/write), drawn after the solids.
                depth_stencil: Some(depth_state(false, wgpu::CompareFunction::Always)),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let make_stars_pipeline = || {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("stars pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_stars"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x3,
                            1 => Float32x3,
                            2 => Float32x2,
                        ],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_stars"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    // The celestial sphere is seen from inside.
                    cull_mode: Some(wgpu::Face::Front),
                    ..Default::default()
                },
                // The backdrop must never occlude the (more distant) Moon, so
                // it neither writes nor tests depth.
                depth_stencil: Some(depth_state(false, wgpu::CompareFunction::Always)),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        // The satellite markers: one constant-pixel-size circle per tracked
        // object, generated from the vertex index, alpha-blended over the
        // finished scene, drawn last as a single instanced draw. The quad
        // corners come from the vertex index; the per-marker world position and
        // visibility come from the instance buffer (one `MarkerInstance` per
        // satellite).
        let make_marker_pipeline = || {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("marker pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_marker"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<MarkerInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        // 0 => position (vec3), 1 => visible (f32).
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_marker"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        // Standard alpha blend for the antialiased edge.
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    // A screen-facing quad; no culling.
                    cull_mode: None,
                    ..Default::default()
                },
                // Screen overlays drawn last; CPU-occluded, so no depth.
                depth_stencil: Some(depth_state(false, wgpu::CompareFunction::Always)),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        // The Moon: same vertex format as the globe, lit by the Sun with its
        // own eclipse shadow. A solid body, so it writes depth and tests
        // `Greater` like the Earth surface (the depth buffer is what makes the
        // Earth occlude it).
        let make_moon_pipeline = || {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("moon pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_moon"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x3,
                            1 => Float32x3,
                            2 => Float32x2,
                        ],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_moon"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(depth_state(true, wgpu::CompareFunction::Greater)),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        // The planet pipeline: same vertex format and reversed-Z solid-body
        // depth setup as the Moon (planets are opaque bodies that the depth
        // buffer resolves against each other), differing only in entry points
        // and the two-group layout. Built after the join (it borrows
        // `planet_layout`); one extra pipeline compile is cheap.
        let make_planet_pipeline = || {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("planet pipeline"),
                layout: Some(&planet_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_planet"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x3,
                            1 => Float32x3,
                            2 => Float32x2,
                        ],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_planet"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(depth_state(true, wgpu::CompareFunction::Greater)),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let (
            render_pipeline,
            (atmosphere_pipeline, (stars_pipeline, (marker_pipeline, moon_pipeline))),
        ) = rayon::join(make_render_pipeline, || {
            rayon::join(make_atmosphere_pipeline, || {
                rayon::join(make_stars_pipeline, || {
                    rayon::join(make_marker_pipeline, make_moon_pipeline)
                })
            })
        });

        let planet_pipeline = make_planet_pipeline();

        Self {
            render_pipeline,
            atmosphere_pipeline,
            stars_pipeline,
            moon_pipeline,
            marker_pipeline,
            vertices,
            indices,
            index_count: mesh.indices.len() as u32,
            moon_vertices,
            moon_indices,
            moon_index_count: moon_mesh.indices.len() as u32,
            planet_pipeline,
            planets,
            planet_count: 0,
            draw_earth_system: true,
            uniforms,
            bind_group,
            markers,
            marker_capacity: INITIAL_MARKER_CAPACITY,
            marker_count: 0,
        }
    }

    /// Writes the per-frame uniforms and marker instances from the simulation's
    /// `RenderState`. Call before submitting the frame's command buffer;
    /// `queue.write_buffer` is ordered before it. `viewport` is the surface
    /// size in pixels (width, height), used only for the screen-space
    /// markers. Takes `&mut self` (and `&Device`) because the marker
    /// instance buffer grows on demand when more satellites are tracked
    /// than it currently holds. All camera/astronomical math is done by the
    /// simulation (see `simulation::SimulationState`); this just packs the
    /// finished values into the GPU layout.
    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render: &RenderState,
        viewport: (f32, f32),
    ) {
        let (width, height) = viewport;

        // Everything the GPU sees is in the RENDER FRAME: positions relative to
        // `render.render_origin` (the camera target's center). The renderer does
        // the subtraction here on the CPU so the shader only ever handles small,
        // target-local coordinates - the orbited body lands exactly at the
        // origin (its absolute center IS render_origin, so the difference is a
        // bit-exact zero), which is what keeps far planets from jittering.
        let origin = render.render_origin;

        // star_rot_inv (world -> celestial) is uploaded as-is; its mat3x3
        // columns are padded to vec4 stride.
        let star_cols = render.star_rot_inv.to_cols_array_2d();
        let moon_cols = render.moon_rot.to_cols_array_2d();

        let uniforms = Uniforms {
            view_proj: render.view_proj.to_cols_array(),
            camera_pos: render.camera_pos.to_array(),
            _pad0: 0.0,
            star_rot_inv: std::array::from_fn(|c| {
                [star_cols[c][0], star_cols[c][1], star_cols[c][2], 0.0]
            }),
            marker: [width, height, MARKER_RADIUS_PX, 0.0],
            moon_rot: std::array::from_fn(|c| {
                [moon_cols[c][0], moon_cols[c][1], moon_cols[c][2], 0.0]
            }),
            moon_pos: (render.moon_pos_world - origin).to_array(),
            _pad2: 0.0,
            moon_params: [
                render.moon_radius_km,
                earth::MEAN_RADIUS_KM,
                SUN_ANGULAR_RADIUS_RAD,
                0.0,
            ],
            sun_pos: (render.sun_pos_world - origin).to_array(),
            _pad4: 0.0,
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));

        // The Earth system (Earth surface + atmosphere + Moon + markers) renders
        // only when orbiting the Earth or the Moon, i.e. the render origin is at
        // the Earth. Orbiting a planet, the Earth/Moon are a far speck and the
        // atmosphere is Earth-centered physics, so they are skipped.
        self.draw_earth_system = origin == glam::Vec3::ZERO;

        // One model uniform per planet to draw this frame, each at its center
        // RELATIVE to the render origin. `render.planets` is in planet::ALL
        // order (empty for non-solar scenarios), the same order as
        // `self.planets`, so they zip 1:1; `planet_count` gates the draw.
        self.planet_count = render.planets.len() as u32;
        for (gpu, state) in self.planets.iter().zip(&render.planets) {
            let rot_cols = state.rot.to_cols_array_2d();
            let planet_uniform = PlanetUniform {
                rot: std::array::from_fn(|c| [rot_cols[c][0], rot_cols[c][1], rot_cols[c][2], 0.0]),
                pos: (state.pos_world - origin).to_array(),
                radius_km: state.planet.mean_radius_km(),
            };
            queue.write_buffer(&gpu.uniform, 0, bytemuck::bytes_of(&planet_uniform));
        }

        // One marker instance per tracked satellite. Grow the buffer first if
        // this frame has more markers than it currently holds (only ever
        // happens once, since the count is fixed at startup).
        let instances: Vec<MarkerInstance> = render
            .markers
            .iter()
            .map(|m| MarkerInstance {
                position: m.position_km.to_array(),
                visible: if m.visible { 1.0 } else { 0.0 },
            })
            .collect();
        self.marker_count = instances.len() as u32;
        if self.marker_count > self.marker_capacity {
            self.marker_capacity = self.marker_count.next_power_of_two();
            self.markers = make_marker_buffer(device, self.marker_capacity);
        }
        if !instances.is_empty() {
            queue.write_buffer(&self.markers, 0, bytemuck::cast_slice(&instances));
        }
    }

    fn render(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertices.slice(..));
        render_pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);

        // Backdrop first; it always draws (the stars/Sun frame every body).
        render_pass.set_pipeline(&self.stars_pipeline);
        render_pass.draw_indexed(0..self.index_count, 0, 0..1);

        // The Earth surface (which writes depth), the Moon, then the atmosphere
        // over the disc and limb - drawn only when orbiting the Earth/Moon (the
        // render origin is at the Earth). Orbiting a planet they would be a far
        // speck (and the Earth-centered atmosphere physics is meaningless), so
        // they are skipped, leaving just the planets + backdrop.
        if self.draw_earth_system {
            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.draw_indexed(0..self.index_count, 0, 0..1);

            render_pass.set_pipeline(&self.moon_pipeline);
            render_pass.set_vertex_buffer(0, self.moon_vertices.slice(..));
            render_pass.set_index_buffer(self.moon_indices.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..self.moon_index_count, 0, 0..1);
        }

        // The planets: each its own mesh + group-1 bind group (model uniform +
        // texture), the same solid-body depth setup as the Moon. The orbited
        // planet is drawn in local coordinates (its center is the render
        // origin); the others sit far off as specks. Only the solar-system
        // scenario fills `planets` (planet_count > 0). group 0 stays bound.
        if self.planet_count > 0 {
            render_pass.set_pipeline(&self.planet_pipeline);
            for gpu in self.planets.iter().take(self.planet_count as usize) {
                render_pass.set_bind_group(1, &gpu.bind_group, &[]);
                render_pass.set_vertex_buffer(0, gpu.vertices.slice(..));
                render_pass.set_index_buffer(gpu.indices.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..gpu.index_count, 0, 0..1);
            }
        }

        if self.draw_earth_system {
            // The atmosphere reuses the globe shell mesh - rebind it.
            render_pass.set_vertex_buffer(0, self.vertices.slice(..));
            render_pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.set_pipeline(&self.atmosphere_pipeline);
            render_pass.draw_indexed(0..self.index_count, 0, 0..1);

            // The satellite markers last, as screen overlays: one instanced
            // draw, one instance per tracked object. Skipped when none tracked.
            if self.marker_count > 0 {
                render_pass.set_pipeline(&self.marker_pipeline);
                render_pass.set_vertex_buffer(0, self.markers.slice(..));
                render_pass.draw(0..6, 0..self.marker_count);
            }
        }
    }
}

/// Which upload path a [`GlobeRenderer`] texture input takes.
#[derive(Clone, Copy)]
enum TexKind {
    /// A color map decoded from JPEG/TIFF and uploaded `Rgba8UnormSrgb`
    /// (day/night/stars), so sampling linearizes the sRGB bytes on the GPU.
    ColorSrgb,
    /// A data map decoded from JPEG/TIFF and uploaded `Rgba8Unorm`, kept linear
    /// (normal/specular).
    DataLinear,
    /// A build-baked f16 atmosphere LUT in a KTX2 container (`Rgba16Float`).
    Lut,
}

/// Decodes an embedded source texture (original JPEG/TIFF bytes, downloaded
/// verbatim by build.rs) with the `image` crate and uploads it as an
/// uncompressed RGBA8 texture. `srgb` selects `Rgba8UnormSrgb` for color maps
/// (day/night/stars) vs `Rgba8Unorm` for data maps (normal/specular). Both
/// formats are universally supported and filterable, so this needs no device
/// feature - the key to running on every backend/GPU.
fn upload_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    bytes: &[u8],
    srgb: bool,
) -> wgpu::TextureView {
    let decoded = image::load_from_memory(bytes)
        .unwrap_or_else(|error| panic!("decode {label}: {error}"))
        .to_rgba8();
    let (width, height) = decoded.dimensions();

    let format = if srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };

    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        decoded.as_raw(),
    );

    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// Uploads a build-script-baked atmosphere LUT from its KTX2 container: the f16
/// texel rows are copied to the GPU as-is. Only the `Rgba16Float` LUT format is
/// expected now - the Earth/star textures use [`upload_image`] instead.
fn upload_ktx2(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    bytes: &[u8],
) -> wgpu::TextureView {
    let reader =
        ktx2::Reader::new(bytes).unwrap_or_else(|error| panic!("parse {label}: {error:?}"));
    let header = reader.header();

    let format = match header.format {
        Some(ktx2::Format::R16G16B16A16_SFLOAT) => wgpu::TextureFormat::Rgba16Float,
        other => panic!("{label}: unexpected ktx2 format {other:?}"),
    };

    let level = reader
        .levels()
        .next()
        .unwrap_or_else(|| panic!("{label}: ktx2 file has no mip levels"));

    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: header.pixel_width,
                height: header.pixel_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        level.data,
    );

    texture.create_view(&wgpu::TextureViewDescriptor::default())
}
