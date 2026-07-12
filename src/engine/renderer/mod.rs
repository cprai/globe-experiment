use rayon::prelude::*;
use wgpu::util::DeviceExt;

use glam::{DMat4, DVec3, DVec4};

use crate::engine::planet;
use crate::engine::scene::celestial_sphere::CelestialSphere;
use crate::engine::scene::satellite;
use crate::engine::scene::{CameraTarget, CelestialBody, RenderState};

const MARKER_RADIUS_PX: f32 = 6.0;

/// Segments per predicted orbit path (one period). 1.4 deg per segment; the
/// chord sagitta at LEO radius (~0.5 km) is sub-pixel at whole-Terra zoom.
const PATH_SEGMENTS: usize = 256;

/// Two satellites' worth: no first-frame realloc for the shipping scenes.
const INITIAL_PATH_CAPACITY: u32 = 2 * PATH_SEGMENTS as u32;

/// Fraction of the period at which the path alpha starts its smoothstep fade
/// (zero at one full period, so the line ends sharply).
const PATH_FADE_START: f32 = 0.85;

/// Paired with the reversed-Z projection (cleared 0.0, tested `Greater`) so
/// Terra occludes the far-off Luna across the huge near/far span.
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// CPU twin of `ATMOSPHERE_TOP_KM` in scene.wgsl and `build.rs mod
/// atmosphere` - all three must stay in sync.
const ATMOSPHERE_TOP_KM: f64 = 6460.0;

/// Sol's physical radius (km); per-body Sol angular radius = asin(R/dist).
const SOL_RADIUS_KM: f64 = 695_700.0;

/// Eclipse-occluder slots per impostor uniform (radius 0 = unused). Must
/// match `MAX_OCCLUDERS` in scene.wgsl.
const MAX_OCCLUDERS: usize = 4;

/// GPU-slot order of the impostor bodies: `planet::ALL`, then Luna, then
/// Terra. Built from `planet::ALL` so the two never drift.
const IMPOSTOR_BODIES: [CelestialBody; 9] = {
    let mut bodies = [CelestialBody::TERRA; 9];
    let mut i = 0;
    while i < planet::ALL.len() {
        bodies[i] = planet::ALL[i];
        i += 1;
    }
    bodies[7] = CelestialBody::LUNA;
    bodies
};

/// Apparent angular-DIAMETER cutoff (arcsec) between the impostor's two
/// trace modes: below it the ORTHOGRAPHIC (parallel-ray) trace, exact and
/// f32-safe at any distance; at/above it the PERSPECTIVE (eye-ray) trace,
/// which matches a near body's foreshortened silhouette but is only f32-safe
/// while distance/radius is modest. ~0.5 deg: comfortably above any planet
/// seen from Terra (Venus peaks ~66"), comfortably below any orbited body.
const PLANET_PERSPECTIVE_MIN_ARCSEC: f64 = 1800.0;

/// The impostor quad spans the silhouette's angular radius times this margin
/// so the full disc lands inside; fragments that miss the ellipse discard.
const PLANET_QUAD_MARGIN: f64 = 1.3;

/// Depth floor for a beyond-far-plane body (billions of km out projects to
/// reversed-Z depth <= 0 and would be z-clipped). Must be > 0 to pass the
/// `Greater`-than-clear test; such a body is sub-pixel in practice.
const PLANET_MIN_DEPTH: f64 = 1e-6;

/// Vertical field of view, degrees. The camera's pan math reads it too.
pub const FOV_Y_DEG: f64 = 45.0;

/// Near clip plane as a fraction of the orbit target's mean radius.
pub const NEAR_PLANE_RADII: f64 = 0.01;

/// Far clip plane FLOOR, km: encloses Luna at apogee + camera distance (and,
/// orbiting Luna, Terra). `prepare` grows the actual far plane per frame.
pub const FAR_PLANE_KM: f64 = 500_000.0;

/// Reversed-Z view-projection (near -> 1, far -> 0), paired with the depth
/// buffer cleared to 0 and tested `Greater`. Takes the look-at POINT (a
/// re-normalized forward would drift off the view the camera implied). All
/// f64; callers cast to f32 only at uniform upload.
pub fn view_proj_reversed_z(
    eye: DVec3,
    look_at: DVec3,
    up: DVec3,
    aspect: f64,
    near: f64,
    far: f64,
) -> DMat4 {
    let view = DMat4::look_at_rh(eye, look_at, up);
    let proj = DMat4::perspective_rh(FOV_Y_DEG.to_radians(), aspect.max(0.01), near, far);

    // `z_clip' = w_clip - z_clip`: negate the proj Z row and add the W row.
    let reverse_z = DMat4::from_cols(
        DVec4::new(1.0, 0.0, 0.0, 0.0),
        DVec4::new(0.0, 1.0, 0.0, 0.0),
        DVec4::new(0.0, 0.0, -1.0, 0.0),
        DVec4::new(0.0, 0.0, 1.0, 1.0),
    );

    reverse_z * proj * view
}

/// Depth view sized to the render target; recreate on resize.
pub(crate) fn create_depth_view(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> wgpu::TextureView {
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

/// The reversed-Z depth attachment, cleared to 0.0 (the far plane).
pub(crate) fn depth_attachment(
    view: &wgpu::TextureView,
) -> wgpu::RenderPassDepthStencilAttachment<'_> {
    wgpu::RenderPassDepthStencilAttachment {
        view,
        depth_ops: Some(wgpu::Operations {
            load: wgpu::LoadOp::Clear(0.0),
            store: wgpu::StoreOp::Store,
        }),
        stencil_ops: None,
    }
}

/// Render-ready egui paint output, produced by whichever side owns the egui
/// context and consumed by the presenting renderer.
pub struct UiFrame {
    pub primitives: Vec<egui::ClippedPrimitive>,
    pub textures_delta: egui::TexturesDelta,
    pub pixels_per_point: f32,
}

/// Requests a high-performance adapter and a device with **no** optional
/// features and default limits (textures upload uncompressed, so no BC/ASTC
/// feature is needed - the key to running on every backend/GPU).
pub(crate) fn request_adapter_device(
    instance: &wgpu::Instance,
    compatible_surface: Option<&wgpu::Surface<'_>>,
) -> (wgpu::Adapter, wgpu::Device, wgpu::Queue) {
    // Where Vulkan is absent (WSL+X11, CI) fall back to any adapter across
    // all backends (GL, software) that can present. WGPU_BACKEND=gl forces GL.
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
        label: Some("scene device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
    }))
    .expect("request device");

    (adapter, device, queue)
}

/// Per-frame shader uniforms. Layout must match `Uniforms` in scene.wgsl
/// (vec3s padded to 16 bytes; mat3x3 columns padded to vec4 stride). All
/// positions are render-frame km (relative to the camera target).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [f32; 16],
    /// Per-fragment eye-ray reconstruction (perspective trace, atmosphere,
    /// stars).
    inv_view_proj: [f32; 16],
    camera_pos: [f32; 3],
    _pad0: f32,
    /// World -> galactic texture frame (star map lookup).
    star_rot_inv: [[f32; 4]; 3],
    /// x,y = viewport px, z = marker radius px, w unused. Per-marker data is
    /// in the instance buffer.
    marker: [f32; 4],
    /// xyz = Luna center, w = radius km - for the atmosphere's occlusion
    /// check, the one pass that must know about Luna without drawing it.
    luna_occluder: [f32; 4],
    /// Every lit pass derives its Sol direction from this; there is no
    /// Earth-fixed `sol_dir`.
    sol_pos: [f32; 3],
    _pad4: f32,
    /// xy = NDC center, zw = NDC half-extent ((0,0,1,1) = full-screen, the
    /// usual case at current camera limits).
    atmosphere_quad: [f32; 4],
}

/// Per-body impostor uniform (group 1). Layout must match `PlanetUniform` in
/// scene.wgsl. Positions are render-frame km.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PlanetUniform {
    /// Body-fixed -> world rotation.
    rot: [[f32; 4]; 3],
    /// Exactly zero for the ORBITED body (its center IS the render origin) -
    /// the key to keeping the perspective trace f32-precise.
    pos: [f32; 3],
    /// Sol's angular radius seen from this body (rad): its eclipse penumbra
    /// softness.
    sol_angular_radius: f32,
    ndc_center: [f32; 2],
    ndc_half_extent: [f32; 2],
    /// Triaxial semi-axes km (rx = rz for a planet, all distinct for Luna).
    radii: [f32; 3],
    /// Baseline reversed-Z depth for the orthographic trace (the perspective
    /// trace overrides it per fragment).
    depth: f32,
    /// Same-system eclipse casters: xyz = center, w = radius km (0 = unused).
    occluders: [[f32; 4]; MAX_OCCLUDERS],
    /// 1.0 = perspective (eye-ray) trace, 0.0 = orthographic. See
    /// `PLANET_PERSPECTIVE_MIN_ARCSEC`.
    perspective: f32,
    /// `BODY_FLAG_*` shading-feature bits.
    flags: u32,
    _pad: [u32; 2],
}

/// Must match the `BODY_FLAG_*` consts in scene.wgsl. A bit per optional
/// feature because they degrade independently (a dummy specular mask would
/// still leave a land-level GGX sheen, so it must be off entirely).
const BODY_FLAG_NIGHT: u32 = 1;
const BODY_FLAG_NORMAL_MAP: u32 = 2;
const BODY_FLAG_SPECULAR: u32 = 4;
const BODY_FLAG_ATMO_LIT: u32 = 8;

fn body_shading_flags(body: CelestialBody) -> u32 {
    let maps = body.maps();
    let mut flags = 0;
    if maps.night.is_some() {
        flags |= BODY_FLAG_NIGHT;
    }
    if maps.normal.is_some() {
        flags |= BODY_FLAG_NORMAL_MAP;
    }
    if maps.specular.is_some() {
        flags |= BODY_FLAG_SPECULAR;
    }
    if body.has_atmosphere() {
        flags |= BODY_FLAG_ATMO_LIT;
    }
    flags
}

/// One impostor body's long-lived GPU resources, in `IMPOSTOR_BODIES` order.
struct PlanetGpu {
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// Marker instance data; layout must match `vs_marker` in scene.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MarkerInstance {
    position: [f32; 3],
    /// 1.0 = drawn, 0.0 = hidden (the vertex shader pushes it off-screen).
    visible: f32,
}

const INITIAL_MARKER_CAPACITY: u32 = 8;

fn make_marker_buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("marker instances"),
        size: u64::from(capacity) * std::mem::size_of::<MarkerInstance>() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// One orbit-path segment; layout must match `PathInstance` in scene.wgsl.
/// Each segment carries the neighboring sample on each side so the vertex
/// shader can miter the joints (both quads at a joint offset the shared
/// endpoint identically - watertight, no overlap). Neighbors equal the
/// endpoint at the path ends, degenerating those joints to butt ends.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PathInstance {
    prev: [f32; 3],
    _pad0: f32,
    p0: [f32; 3],
    /// Fade alpha (1 at the satellite, 0 one period ahead).
    alpha0: f32,
    p1: [f32; 3],
    alpha1: f32,
    next: [f32; 3],
    _pad1: f32,
}

fn make_path_buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("path instances"),
        size: u64::from(capacity) * std::mem::size_of::<PathInstance>() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Path alpha at `t` = fraction of the period: 1 until [`PATH_FADE_START`],
/// then one smoothstep to 0 at a full period.
fn path_fade(t: f32) -> f32 {
    let s = ((t - PATH_FADE_START) / (1.0 - PATH_FADE_START)).clamp(0.0, 1.0);
    1.0 - s * s * (3.0 - 2.0 * s)
}

/// Every long-lived wgpu object for the scene; the shared render core owned
/// by each binary's presenter (windowed `Gfx` or `OffscreenRenderer`).
pub(crate) struct SceneRenderer {
    atmosphere_pipeline: wgpu::RenderPipeline,
    stars_pipeline: wgpu::RenderPipeline,
    marker_pipeline: wgpu::RenderPipeline,
    /// Depth-TESTED without write (`Greater`) so solids occlude the path's
    /// far side while the translucent line occludes nothing.
    path_pipeline: wgpu::RenderPipeline,
    /// One impostor pipeline shared by ALL nine bodies (group 1 swaps per
    /// draw). Writes per-fragment depth - the scene's only depth-writing
    /// pass.
    planet_pipeline: wgpu::RenderPipeline,
    /// In `IMPOSTOR_BODIES` order; always built, drawn per
    /// `planet_draw_indices`.
    planets: Vec<PlanetGpu>,
    /// Bodies whose center projects in front of the camera this frame. Order
    /// irrelevant (depth-tested).
    planet_draw_indices: Vec<usize>,
    /// True when a `has_atmosphere` body sits exactly at the render origin -
    /// the pass's LUT math assumes the body at the origin, and from a planet
    /// orbit the Terra atmosphere is sub-pixel anyway.
    draw_atmosphere: bool,
    /// True when the render origin is Terra: satellite positions are
    /// Terra-frame world coordinates, meaningless from a planet orbit.
    draw_satellite_overlays: bool,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Marker instances; grows on demand (not in the bind group, so growth
    /// never touches `bind_group`).
    markers: wgpu::Buffer,
    marker_capacity: u32,
    marker_count: u32,
    /// Orbit-path segment instances; same grow-on-demand pattern.
    paths: wgpu::Buffer,
    path_capacity: u32,
    path_count: u32,
}

impl SceneRenderer {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let markers = make_marker_buffer(device, INITIAL_MARKER_CAPACITY);
        let paths = make_path_buffer(device, INITIAL_PATH_CAPACITY);

        // Shader-module compilation (naga parse + validation) is independent
        // of the texture loads, so it runs on one rayon task while the
        // group-0 textures decode/upload in parallel; the body maps load with
        // the group-1 batch below.
        let texture_inputs: [(&str, &[u8], TexKind); 4] = [
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
        ];

        let (module, views) = rayon::join(
            || {
                device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("scene shader"),
                    source: wgpu::ShaderSource::Wgsl(
                        include_str!("../../../shaders/scene.wgsl").into(),
                    ),
                })
            },
            || {
                texture_inputs
                    .into_par_iter()
                    .map(|(label, bytes, kind)| match kind {
                        TexKind::ColorSrgb => upload_image(device, queue, label, bytes, true),
                        TexKind::Lut => upload_ktx2(device, queue, label, bytes),
                    })
                    .collect::<Vec<_>>()
            },
        );

        // par_iter preserves input order, so the views line up with the
        // bindings below.
        let [
            transmittance_view,
            inscatter_rayleigh_view,
            inscatter_mie_view,
            stars_view,
        ]: [wgpu::TextureView; 4] = views
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
            label: Some("map sampler"),
            // Repeat across the dateline seam, clamp at the poles.
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene bind group layout"),
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
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene bind group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&transmittance_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&lut_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&inscatter_rayleigh_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&inscatter_mie_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&stars_view),
                },
            ],
        });

        // Impostor bodies (group 1). Per-body bind groups keep the body maps
        // out of group 0, holding the worst fragment stage well under the
        // portable 16-sampled-textures-per-stage limit.
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
                    // Optional feature maps (night / normal / specular);
                    // bodies without one bind a shared 1x1 dummy (the layout
                    // needs every slot filled, the flags stop any sampling).
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
                ],
            });

        // Embedded body maps, IMPOSTOR_BODIES order. The include_bytes!
        // paths must match `CelestialBody::maps()` (the single source of the
        // body<->file mapping).
        struct BodyMapBytes {
            albedo: &'static [u8],
            night: Option<&'static [u8]>,
            normal: Option<&'static [u8]>,
            specular: Option<&'static [u8]>,
        }
        const fn albedo_bytes(albedo: &'static [u8]) -> BodyMapBytes {
            BodyMapBytes {
                albedo,
                night: None,
                normal: None,
                specular: None,
            }
        }
        let planet_map_bytes: [BodyMapBytes; IMPOSTOR_BODIES.len()] = [
            albedo_bytes(include_bytes!(concat!(env!("OUT_DIR"), "/8k_mercury.jpg"))),
            albedo_bytes(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/8k_venus_surface.jpg"
            ))),
            albedo_bytes(include_bytes!(concat!(env!("OUT_DIR"), "/8k_mars.jpg"))),
            albedo_bytes(include_bytes!(concat!(env!("OUT_DIR"), "/8k_jupiter.jpg"))),
            albedo_bytes(include_bytes!(concat!(env!("OUT_DIR"), "/8k_saturn.jpg"))),
            albedo_bytes(include_bytes!(concat!(env!("OUT_DIR"), "/2k_uranus.jpg"))),
            albedo_bytes(include_bytes!(concat!(env!("OUT_DIR"), "/2k_neptune.jpg"))),
            albedo_bytes(include_bytes!(concat!(env!("OUT_DIR"), "/8k_moon.jpg"))),
            BodyMapBytes {
                albedo: include_bytes!(concat!(env!("OUT_DIR"), "/8k_earth_daymap.jpg")),
                night: Some(include_bytes!(concat!(
                    env!("OUT_DIR"),
                    "/8k_earth_nightmap.jpg"
                ))),
                normal: Some(include_bytes!(concat!(
                    env!("OUT_DIR"),
                    "/8k_earth_normal_map.tif"
                ))),
                specular: Some(include_bytes!(concat!(
                    env!("OUT_DIR"),
                    "/8k_earth_specular_map.tif"
                ))),
            },
        ];

        let dummy_night = upload_solid_1x1(device, queue, "dummy night map", [0, 0, 0, 255], true);
        let dummy_normal = upload_solid_1x1(
            device,
            queue,
            "dummy normal map",
            [128, 128, 255, 255],
            false,
        );
        let dummy_specular =
            upload_solid_1x1(device, queue, "dummy specular map", [0, 0, 0, 255], false);

        struct BodyMapViews {
            albedo: wgpu::TextureView,
            night: Option<wgpu::TextureView>,
            normal: Option<wgpu::TextureView>,
            specular: Option<wgpu::TextureView>,
        }
        let planet_views: Vec<BodyMapViews> = IMPOSTOR_BODIES
            .par_iter()
            .zip(planet_map_bytes.par_iter())
            .map(|(body, bytes)| {
                let maps = body.maps();
                let upload_opt = |name: Option<&'static str>, bytes: Option<&[u8]>, srgb| {
                    name.zip(bytes)
                        .map(|(name, bytes)| upload_image(device, queue, name, bytes, srgb))
                };
                BodyMapViews {
                    albedo: upload_image(device, queue, maps.albedo, bytes.albedo, true),
                    night: upload_opt(maps.night, bytes.night, true),
                    normal: upload_opt(maps.normal, bytes.normal, false),
                    specular: upload_opt(maps.specular, bytes.specular, false),
                }
            })
            .collect();

        let planets: Vec<PlanetGpu> = planet_views
            .into_iter()
            .map(|views| {
                let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("planet uniform"),
                    size: std::mem::size_of::<PlanetUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
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
                            resource: wgpu::BindingResource::TextureView(&views.albedo),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(
                                views.night.as_ref().unwrap_or(&dummy_night),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::TextureView(
                                views.normal.as_ref().unwrap_or(&dummy_normal),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: wgpu::BindingResource::TextureView(
                                views.specular.as_ref().unwrap_or(&dummy_specular),
                            ),
                        },
                    ],
                });
                PlanetGpu {
                    uniform,
                    bind_group,
                }
            })
            .collect();

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let planet_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("planet pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout), Some(&planet_bind_group_layout)],
            immediate_size: 0,
        });

        // Per-pass reversed-Z depth policy: the body impostor writes + tests
        // `Greater`; the orbit path tests without writing; backdrop,
        // atmosphere, and markers do neither (pure draw-order layering).
        let depth_state =
            |write_enabled: bool, compare: wgpu::CompareFunction| wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(write_enabled),
                depth_compare: Some(compare),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            };

        // Backend pipeline-state compilation is independent per pipeline, so
        // they build concurrently on rayon tasks.
        let make_atmosphere_pipeline = || {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("atmosphere pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_atmosphere"),
                    compilation_options: Default::default(),
                    // Every pass builds its quad from the vertex index; no
                    // vertex buffers anywhere.
                    buffers: &[],
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
                    cull_mode: None,
                    ..Default::default()
                },
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
                    buffers: &[],
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
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(depth_state(false, wgpu::CompareFunction::Always)),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        // Markers: constant-pixel-size circles, one instanced draw.
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
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_marker"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                // Markers are CPU-occluded, so no depth.
                depth_stencil: Some(depth_state(false, wgpu::CompareFunction::Always)),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        // Orbit paths: one screen-space-expanded quad per segment.
        let make_path_pipeline = || {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("orbit path pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_path"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<PathInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x4,
                            1 => Float32x4,
                            2 => Float32x4,
                            3 => Float32x4,
                        ],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_path"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(depth_state(false, wgpu::CompareFunction::Greater)),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let (atmosphere_pipeline, (stars_pipeline, (marker_pipeline, path_pipeline))) =
            rayon::join(make_atmosphere_pipeline, || {
                rayon::join(make_stars_pipeline, || {
                    rayon::join(make_marker_pipeline, make_path_pipeline)
                })
            });

        // The body impostor pipeline: writes per-fragment depth so bodies
        // occlude one another. Built after the join (borrows `planet_layout`).
        let planet_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("planet impostor pipeline"),
            layout: Some(&planet_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_planet"),
                compilation_options: Default::default(),
                buffers: &[],
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
                ..Default::default()
            },
            depth_stencil: Some(depth_state(true, wgpu::CompareFunction::Greater)),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            atmosphere_pipeline,
            stars_pipeline,
            marker_pipeline,
            path_pipeline,
            planet_pipeline,
            planets,
            planet_draw_indices: Vec::new(),
            draw_atmosphere: true,
            draw_satellite_overlays: true,
            uniforms,
            bind_group,
            markers,
            marker_capacity: INITIAL_MARKER_CAPACITY,
            marker_count: 0,
            paths,
            path_capacity: INITIAL_PATH_CAPACITY,
            path_count: 0,
        }
    }

    /// Writes the frame's uniforms + instance buffers from `RenderState`.
    /// Call before submitting the command buffer (`queue.write_buffer` is
    /// ordered before it).
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render: &RenderState,
        viewport: (f64, f64),
    ) {
        let (width, height) = viewport;
        let aspect = width / height.max(1.0);

        // The same pure `CelestialSphere::at` the scene evaluated, so the two
        // agree exactly - what makes the orbited body's render-frame position
        // a bit-exact zero below.
        let celestial = CelestialSphere::at(&render.time);

        // Everything uploaded is in the RENDER FRAME (camera-target-local);
        // the subtraction happens here, on the CPU, in f64.
        let origin = render.camera_target.render_origin(&celestial);
        // Bridges the Terra-frame (geocentric) satellite overlays into the
        // render frame: a Terra-relative point q sits at terra_center + q -
        // origin. Overlays only draw for a Terra-system target, where this is
        // a bit-exact no-op - kept for the render-frame convention.
        let terra_center = celestial.center_world(CelestialBody::TERRA);

        let radius = render.camera_target.mean_radius_km();
        let near = NEAR_PLANE_RADII * radius;
        // Grow the far plane to enclose the orbited body at any zoom: a gas
        // giant at max zoom-out (Jupiter eye-to-center ~770,000 km) sits past
        // the fixed floor and a fixed far plane would clip its whole disc.
        let far = (render.camera_pos.length() + 2.0 * radius).max(FAR_PLANE_KM);
        let view_proj = view_proj_reversed_z(
            render.camera_pos,
            render.camera_look_at,
            render.camera_up,
            aspect,
            near,
            far,
        );
        let inv_view_proj = view_proj.inverse();

        // star_tex_rot_inv (not the equatorial star_rot_inv) is what the
        // shader receives - the star texture is drawn in galactic coords.
        let star_cols = celestial.star_tex_rot_inv.to_cols_array_2d();

        // For the atmosphere's Luna-occlusion check only; Luna itself draws
        // through the shared body impostor below.
        let luna_pos_world = celestial
            .body(CelestialBody::LUNA)
            .map_or(DVec3::ZERO, |state| state.placement.pos_world);

        // Atmosphere gate + quad: draws when a has_atmosphere body sits
        // bit-exactly at the render origin (the LUT math assumes that). The
        // quad covers the top-of-atmosphere silhouette; full-screen when the
        // shell is near/perspective-sized - at current camera limits, always
        // (the tight quad is a correctness net for future limits).
        let tan_half_fov = (FOV_Y_DEG / 2.0).to_radians().tan();
        self.draw_atmosphere = celestial
            .bodies
            .iter()
            .any(|s| s.body.has_atmosphere() && s.placement.pos_world == origin);
        let mut atmosphere_quad = [0.0f64, 0.0, 1.0, 1.0];
        if self.draw_atmosphere {
            let dist = render.camera_pos.length();
            if dist > ATMOSPHERE_TOP_KM * 1.05 {
                let ang_radius = (ATMOSPHERE_TOP_KM / dist).min(0.999).asin();
                let arcsec = 2.0 * ang_radius.to_degrees() * 3600.0;
                let clip = view_proj * DVec3::ZERO.extend(1.0);
                if arcsec < PLANET_PERSPECTIVE_MIN_ARCSEC && clip.w > 0.0 {
                    let half_y = (ang_radius * PLANET_QUAD_MARGIN).tan() / tan_half_fov;
                    atmosphere_quad = [clip.x / clip.w, clip.y / clip.w, half_y / aspect, half_y];
                }
            }
        }

        // The casts below are the f64 -> f32 GPU boundary.
        let uniforms = Uniforms {
            view_proj: view_proj.as_mat4().to_cols_array(),
            inv_view_proj: inv_view_proj.as_mat4().to_cols_array(),
            camera_pos: render.camera_pos.as_vec3().to_array(),
            _pad0: 0.0,
            star_rot_inv: std::array::from_fn(|c| {
                [
                    star_cols[c][0] as f32,
                    star_cols[c][1] as f32,
                    star_cols[c][2] as f32,
                    0.0,
                ]
            }),
            marker: [width as f32, height as f32, MARKER_RADIUS_PX, 0.0],
            luna_occluder: {
                let p = (luna_pos_world - origin).as_vec3();
                [p.x, p.y, p.z, CelestialBody::LUNA.mean_radius_km() as f32]
            },
            sol_pos: (celestial.sol_pos_world - origin).as_vec3().to_array(),
            _pad4: 0.0,
            atmosphere_quad: atmosphere_quad.map(|v| v as f32),
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));

        // Satellite positions are Terra-frame, so overlays draw only for a
        // Terra-system target. Keyed off the target's identity, not
        // `origin == ZERO` - in the heliocentric frame Terra's center is not
        // zero.
        self.draw_satellite_overlays = matches!(
            render.camera_target,
            CameraTarget::Body(CelestialBody::TerraSystem(_))
        );

        // Project each body's center to screen space and pack its impostor
        // uniform - every body draws from every vantage (non-orbited ones are
        // sub-pixel specks or behind the camera).
        self.planet_draw_indices.clear();
        for state in &celestial.bodies {
            let body = state.body;
            let Some(i) = IMPOSTOR_BODIES
                .iter()
                .position(|candidate| *candidate == body)
            else {
                continue;
            };
            let pos_render = state.placement.pos_world - origin;
            let rel = pos_render - render.camera_pos;
            let dist = rel.length();
            if dist <= f64::EPSILON {
                continue;
            }
            // Largest semi-axis bounds the silhouette from any view direction
            // (also the orthographic offset scale in the shader - must
            // agree).
            let radii = body.radii_km();
            let rmax = radii.max_element();

            // Skip bodies behind the camera.
            let clip = view_proj * pos_render.extend(1.0);
            if clip.w <= 0.0 {
                continue;
            }
            let inv_w = 1.0 / clip.w;
            let proj_center = [clip.x * inv_w, clip.y * inv_w];
            // A beyond-far-plane body projects to reversed-Z depth <= 0 and
            // would be z-clipped; clamp keeps it drawn behind everything.
            let depth = (clip.z * inv_w).clamp(PLANET_MIN_DEPTH, 1.0);

            let sin_r = (rmax / dist).min(0.999);
            let ang_radius = sin_r.asin();

            let arcsec = 2.0 * ang_radius.to_degrees() * 3600.0;
            let perspective = arcsec >= PLANET_PERSPECTIVE_MIN_ARCSEC;

            // A distant body gets a tight center-anchored quad. A perspective
            // body's projected center can fall far off-screen at high tilt
            // while its near surface still fills the frame - a center-anchored
            // quad would carry the body off-screen - so its quad is the whole
            // screen and the per-fragment trace decides coverage.
            let (ndc_center, ndc_half_extent) = if perspective {
                ([0.0, 0.0], [1.0, 1.0])
            } else {
                let ang = ang_radius * PLANET_QUAD_MARGIN;
                let half_y = ang.tan() / tan_half_fov;
                (proj_center, [half_y / aspect, half_y])
            };

            // Same-system bodies shadow each other (Terra<->Luna eclipses);
            // cross-system shadows are astronomically negligible.
            let mut occluders = [[0.0f32; 4]; MAX_OCCLUDERS];
            let mut occluder_count = 0;
            for other in &celestial.bodies {
                if occluder_count == MAX_OCCLUDERS {
                    break;
                }
                if !body.same_system(other.body) {
                    continue;
                }
                let occ_pos = (other.placement.pos_world - origin).as_vec3();
                occluders[occluder_count] = [
                    occ_pos.x,
                    occ_pos.y,
                    occ_pos.z,
                    other.body.mean_radius_km() as f32,
                ];
                occluder_count += 1;
            }

            // Per-body Sol angular radius: an outer system's smaller Sol
            // sharpens its eclipse penumbras automatically.
            let sol_dist = (celestial.sol_pos_world - state.placement.pos_world).length();
            let sol_angular_radius = (SOL_RADIUS_KM / sol_dist.max(SOL_RADIUS_KM)).asin();

            let rot_cols = state.placement.rot.to_cols_array_2d();
            let planet_uniform = PlanetUniform {
                rot: std::array::from_fn(|c| {
                    [
                        rot_cols[c][0] as f32,
                        rot_cols[c][1] as f32,
                        rot_cols[c][2] as f32,
                        0.0,
                    ]
                }),
                pos: pos_render.as_vec3().to_array(),
                sol_angular_radius: sol_angular_radius as f32,
                ndc_center: ndc_center.map(|v| v as f32),
                ndc_half_extent: ndc_half_extent.map(|v| v as f32),
                radii: radii.as_vec3().to_array(),
                depth: depth as f32,
                occluders,
                perspective: if perspective { 1.0 } else { 0.0 },
                flags: body_shading_flags(body),
                _pad: [0; 2],
            };
            queue.write_buffer(
                &self.planets[i].uniform,
                0,
                bytemuck::bytes_of(&planet_uniform),
            );
            self.planet_draw_indices.push(i);
        }

        let instances: Vec<MarkerInstance> = render
            .markers
            .iter()
            .map(|m| MarkerInstance {
                position: (terra_center + m.position_km - origin).as_vec3().to_array(),
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

        // Propagate each marker one period ahead. Recomputed every frame, no
        // caching - cheap enough (~65 us SGP4 / ~0.4 ms numerical).
        self.path_count = 0;
        if self.draw_satellite_overlays && !render.markers.is_empty() {
            // Circumscribe the orbit instead of inscribing it: a chord between
            // samples sags up to r*(1 - cos(pi/N)) (~0.5 km) inside the true
            // arc, and where the path grazes Terra's limb that dip fails the
            // depth test at chord midpoints only - the line renders as dashes.
            // Radially lifting every sample by sec(pi/N) puts the chord
            // MIDPOINTS on the true curve (endpoints half a sagitta out,
            // sub-pixel), so the polyline never falsely dips behind the limb.
            let lift = 1.0 / (std::f64::consts::PI / PATH_SEGMENTS as f64).cos();
            let mut segments = Vec::with_capacity(render.markers.len() * PATH_SEGMENTS);
            for marker in &render.markers {
                let points: Vec<glam::Vec3> = satellite::orbit_path_inertial(
                    &marker.propagation,
                    &render.time,
                    PATH_SEGMENTS,
                )
                .into_iter()
                .map(|p| (p * lift + terra_center - origin).as_vec3())
                .collect();
                // Empty = an escape orbit (e >= 1, no period): no line.
                if points.is_empty() {
                    continue;
                }
                for i in 0..PATH_SEGMENTS {
                    segments.push(PathInstance {
                        prev: points[i.saturating_sub(1)].to_array(),
                        _pad0: 0.0,
                        p0: points[i].to_array(),
                        alpha0: path_fade(i as f32 / PATH_SEGMENTS as f32),
                        p1: points[i + 1].to_array(),
                        alpha1: path_fade((i + 1) as f32 / PATH_SEGMENTS as f32),
                        next: points[(i + 2).min(PATH_SEGMENTS)].to_array(),
                        _pad1: 0.0,
                    });
                }
            }
            self.path_count = segments.len() as u32;
            if self.path_count > self.path_capacity {
                self.path_capacity = self.path_count.next_power_of_two();
                self.paths = make_path_buffer(device, self.path_capacity);
            }
            queue.write_buffer(&self.paths, 0, bytemuck::cast_slice(&segments));
        }
    }

    pub(crate) fn render(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        render_pass.set_bind_group(0, &self.bind_group, &[]);

        render_pass.set_pipeline(&self.stars_pipeline);
        render_pass.draw(0..6, 0..1);

        // Body impostors; group 0 stays bound, group 1 swaps per body.
        if !self.planet_draw_indices.is_empty() {
            render_pass.set_pipeline(&self.planet_pipeline);
            for &i in &self.planet_draw_indices {
                render_pass.set_bind_group(1, &self.planets[i].bind_group, &[]);
                render_pass.draw(0..6, 0..1);
            }
        }

        if self.draw_atmosphere {
            render_pass.set_pipeline(&self.atmosphere_pipeline);
            render_pass.draw(0..6, 0..1);
        }

        if self.draw_satellite_overlays {
            // Paths before markers, so each marker dot sits on its own line.
            if self.path_count > 0 {
                render_pass.set_pipeline(&self.path_pipeline);
                render_pass.set_vertex_buffer(0, self.paths.slice(..));
                render_pass.draw(0..6, 0..self.path_count);
            }

            if self.marker_count > 0 {
                render_pass.set_pipeline(&self.marker_pipeline);
                render_pass.set_vertex_buffer(0, self.markers.slice(..));
                render_pass.draw(0..6, 0..self.marker_count);
            }
        }
    }
}

/// Which upload path a texture input takes.
#[derive(Clone, Copy)]
enum TexKind {
    ColorSrgb,
    /// A build-baked f16 atmosphere LUT (KTX2, `Rgba16Float`).
    Lut,
}

/// Decodes embedded JPEG/TIFF bytes and uploads uncompressed RGBA8. `srgb`
/// selects `Rgba8UnormSrgb` for color maps vs `Rgba8Unorm` for data maps
/// (normal/specular - an sRGB decode would warp the vectors).
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

/// 1x1 solid-color texture - the shared dummy for unused optional-map slots.
fn upload_solid_1x1(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    rgba: [u8; 4],
    srgb: bool,
) -> wgpu::TextureView {
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
                width: 1,
                height: 1,
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
        &rgba,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// Uploads a baked atmosphere LUT from its KTX2 container as-is.
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
