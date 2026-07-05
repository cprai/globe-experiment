mod mesh;

use rayon::prelude::*;
use wgpu::util::DeviceExt;

use glam::{Mat4, Vec3, Vec4};

use crate::engine::luna;
use crate::engine::planet;
use crate::engine::simulation::celestial_sphere::CelestialSphere;
use crate::engine::simulation::satellite;
use crate::engine::simulation::{CelestialBody, RenderState, TerraSystemEntity};
use crate::engine::terra;
use mesh::Vertex;

const STACKS: u32 = 64;
const SLICES: u32 = 128;

/// Radius of the on-screen station marker, in pixels.
const MARKER_RADIUS_PX: f32 = 6.0;

/// Segments per predicted orbit path (one full period ahead per satellite).
/// 1.4 deg of orbit per segment; the chord sagitta at ISS orbital radius
/// (~6800 km) is ~0.5 km - sub-pixel at any whole-Terra zoom, so the polyline
/// reads as a smooth curve.
const PATH_SEGMENTS: usize = 256;

/// Initial path-instance buffer capacity (segments): two satellites' worth,
/// covering both shipping satellite scenarios with no first-frame realloc.
const INITIAL_PATH_CAPACITY: u32 = 2 * PATH_SEGMENTS as u32;

/// Fraction of the orbital period at which the path alpha starts its
/// smoothstep fade: full opacity before it, zero at 1.0 (one full period), so
/// the line vanishes sharply just before closing on the satellite.
const PATH_FADE_START: f32 = 0.85;

/// Depth buffer format. A 32-bit float depth, paired with the reversed-Z
/// projection (near -> 1, far -> 0), cleared to 0.0 and tested
/// `Greater`. This is what lets Terra correctly occlude the much more
/// distant Luna across the scene's enormous near/far span (see
/// [`view_proj_reversed_z`]).
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Sol's angular radius seen from Terra/Luna (~0.266 deg), in radians. Sets
/// the soft penumbra width of the solar-eclipse shadow on Terra (`fs_main`,
/// group 0). The impostor bodies carry a per-body value computed from
/// [`SOL_RADIUS_KM`] instead, so an outer system's sharper penumbra is
/// automatic.
const SOL_ANGULAR_RADIUS_RAD: f32 = 0.004652;

/// Sol's physical radius (km): the per-body Sol angular radius uploaded with
/// each impostor is `asin(SOL_RADIUS_KM / distance-to-Sol)`.
const SOL_RADIUS_KM: f32 = 695_700.0;

/// Eclipse-occluder slots in each impostor's uniform (unused slots have
/// radius 0). One suffices for the Terra system (Terra shadowing Luna); sized
/// for a future moon system's worth of same-system casters. Must match
/// `MAX_OCCLUDERS` in scene.wgsl.
const MAX_OCCLUDERS: usize = 4;

/// Every body drawn as a shader impostor, in GPU-slot order (the renderer's
/// per-body uniform/bind-group arrays): the seven planets in `planet::ALL`
/// order, then Luna. Built from `planet::ALL` so the two never drift; Luna
/// sits last, so the planet slots keep their `planet::ALL` indices.
const IMPOSTOR_BODIES: [CelestialBody; 8] = {
    let mut bodies = [CelestialBody::LUNA; 8];
    let mut i = 0;
    while i < planet::ALL.len() {
        bodies[i] = planet::ALL[i];
        i += 1;
    }
    bodies
};

/// Apparent-size cutoff (angular DIAMETER, arcsec) below which the planet
/// impostor uses the ORTHOGRAPHIC (parallel-ray) trace and above which it uses
/// the PERSPECTIVE (eye-ray) trace. The orthographic branch is exact when
/// distance >> radius and is f32-safe at any distance; the perspective branch
/// matches the close/orbited planet's foreshortened silhouette but is only
/// numerically safe when distance/radius is modest (the ray math scales into
/// unit-sphere space, so its terms stay O(distance/radius)^2). The cutoff sits
/// comfortably above every planet's apparent size from Terra (Venus peaks ~66",
/// Jupiter ~50"), so the Terra view traces all seven orthographically; and
/// comfortably below the orbited body's size (even at max zoom-out a planet
/// subtends tens of thousands of arcsec), so whichever planet is orbited always
/// gets the perspective trace. ~0.5 deg.
const PLANET_PERSPECTIVE_MIN_ARCSEC: f32 = 1800.0;

/// The impostor quad spans the silhouette's angular radius times this margin,
/// so the full disc (corners included) lands inside the quad; edge fragments
/// that miss the ellipse discard. Mirrors the old billboard's margin.
const PLANET_QUAD_MARGIN: f32 = 1.3;

/// Smallest reversed-Z depth (just in front of the far plane) a planet impostor
/// is placed at. A non-orbited planet is often billions of km out - far beyond
/// the far plane - so its projected depth would be <= 0 and its quad would be
/// z-clipped away. Clamping the baseline depth to this tiny positive value
/// keeps it drawn (behind everything else, which has larger depth) so it shows
/// if it is ever large enough on screen. Must be > 0 so it passes the
/// `Greater`-than-0.0-clear depth test. (In practice a non-orbited planet at
/// true solar-system distance subtends well under a pixel - like the old
/// billboards - so this is mostly a correctness safety net, not a visible
/// speck.)
const PLANET_MIN_DEPTH: f32 = 1e-6;

/// Vertical field of view, degrees. The projection authority for the scene; the
/// camera's pan math scales by the same value (`renderer::FOV_Y_DEG`).
pub const FOV_Y_DEG: f32 = 45.0;

/// Near clip plane as a fraction of the orbit target's mean radius. The
/// renderer scales it by `RenderState::camera_target.mean_radius_km()` when it
/// rebuilds the projection, so the near plane tracks whichever body is orbited.
pub const NEAR_PLANE_RADII: f32 = 0.01;

/// Far clip plane, km - a fixed value, NOT a radius multiple. It must enclose
/// Luna at apogee (~406,700 km) plus the camera distance, and - orbiting Luna -
/// Terra ~384,400 km away; the star shell (222,985 km) sits well inside it.
pub const FAR_PLANE_KM: f32 = 500_000.0;

/// Builds the reversed-Z view-projection matrix for an eye at `eye` looking at
/// `look_at` with up `up` (all in the floating-origin render frame). Reversed-Z
/// (near -> 1, far -> 0) pairs with the `Depth32Float` buffer cleared to 0 and
/// a `Greater` test, spreading depth precision across the scene's enormous
/// near/far span so Terra still occludes the far-off Luna. Lives here, not on
/// the camera, because the renderer now rebuilds the projection from
/// `RenderState`'s camera rig (the camera only emits eye / look-at / up).
/// Taking the look-at point directly (not a forward vector) keeps this
/// bit-identical to the camera's previous `view_proj`.
pub fn view_proj_reversed_z(
    eye: Vec3,
    look_at: Vec3,
    up: Vec3,
    aspect: f32,
    near: f32,
    far: f32,
) -> Mat4 {
    let view = Mat4::look_at_rh(eye, look_at, up);
    let proj = Mat4::perspective_rh(FOV_Y_DEG.to_radians(), aspect.max(0.01), near, far);

    // `z_clip' = w_clip - z_clip`: negate the proj Z row and add the W row.
    let reverse_z = Mat4::from_cols(
        Vec4::new(1.0, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 1.0, 0.0, 0.0),
        Vec4::new(0.0, 0.0, -1.0, 0.0),
        Vec4::new(0.0, 0.0, 1.0, 1.0),
    );

    reverse_z * proj * view
}

/// Creates a depth texture view sized to the render target. Recreated whenever
/// the target is resized (the depth attachment must match the color size).
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

/// The reversed-Z depth attachment, cleared to 0.0 (the far plane). Shared by
/// the windowed and offscreen passes so they depth-test identically.
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

/// Render-ready egui paint output, produced by the caller (the windowed
/// application, which owns the `egui::Context` + `egui_winit::State`, or the
/// headless binary's mock-UI pass) and consumed by the presenting renderer
/// (`Gfx::update` / `OffscreenRenderer::render`).
pub struct UiFrame {
    pub primitives: Vec<egui::ClippedPrimitive>,
    pub textures_delta: egui::TexturesDelta,
    pub pixels_per_point: f32,
}

/// Requests a high-performance adapter and a device with **no** optional
/// features and default limits. Shared by the windowed `Gfx` (main binary) and
/// the offscreen `OffscreenRenderer` (headless binary) so both create the
/// device identically. Pass `Some(&surface)` for the windowed path (the adapter
/// must be able to present to that surface) or `None` for offscreen rendering.
/// `instance` is borrowed because each caller owns it (the windowed path needs
/// it to build the surface first).
///
/// No GPU texture-compression feature is requested: the Terra/star textures are
/// uploaded uncompressed (`Rgba8Unorm`/`Rgba8UnormSrgb`, decoded at runtime by
/// `upload_image`), so the renderer runs on every backend and GPU - including
/// those without BC/ASTC (Apple Silicon, ARM SoCs) - with no per-platform
/// format selection.
pub(crate) fn request_adapter_device(
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
        label: Some("scene device"),
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

/// Per-frame shader uniforms. Layout must match `Uniforms` in scene.wgsl:
/// vec3 fields are padded to 16-byte alignment.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [f32; 16],
    /// Inverse of `view_proj`. The planet impostor reconstructs a per-fragment
    /// eye ray from the fragment's NDC through this (the perspective trace);
    /// kept f32-safe by only tracing perspective for near planets.
    inv_view_proj: [f32; 16],
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
    /// Luna center in the render frame (km) = relative to the camera target.
    /// Luna itself draws through the shared body impostor (group 1); this
    /// group-0 copy feeds the two passes that must know about Luna without
    /// drawing it: the solar-eclipse shadow on Terra (`fs_main`) and the
    /// atmosphere's Luna occlusion check (`fs_atmosphere`).
    luna_pos: [f32; 3],
    _pad2: f32,
    /// Eclipse params: x = Luna mean radius km, y = Terra mean radius km,
    /// z = Sol angular radius rad, w = unused.
    luna_params: [f32; 4],
    /// Sol position in the render frame (km) = relative to the camera target.
    /// Every lit pass derives its Sol direction from this; there is no
    /// Earth-fixed `sol_dir`.
    sol_pos: [f32; 3],
    _pad4: f32,
}

/// Per-body impostor uniform (group 1). Layout must match `PlanetUniform` in
/// scene.wgsl. The CPU projects the body's center to screen space and packs
/// the placement here; the GPU draws a single quad at that NDC and ray-traces
/// the triaxial ellipsoid in its fragment shader.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PlanetUniform {
    /// Body-fixed -> world rotation; mat3x3 columns padded to vec4 stride.
    rot: [[f32; 4]; 3],
    /// Body center in the render frame (km) = relative to the camera target.
    /// For the ORBITED body this is exactly zero (its center IS the render
    /// origin) - the key to keeping the perspective trace f32-precise.
    pos: [f32; 3],
    /// Sol's angular radius seen from this body (rad): the penumbra softness
    /// of its eclipse shadows (`asin(SOL_RADIUS_KM / distance to Sol)`).
    sol_angular_radius: f32,
    /// Projected center of the body in NDC (the impostor quad's center).
    ndc_center: [f32; 2],
    /// Half-extent of the impostor quad in NDC (x, y), bounding the silhouette
    /// with margin.
    ndc_half_extent: [f32; 2],
    /// Triaxial semi-axes (km) in the body frame (+X east, +Y pole, +Z prime
    /// meridian); the impostor ellipsoid. rx = rz for a planet, all three
    /// distinct for Luna.
    radii: [f32; 3],
    /// Reversed-Z NDC depth of the projected center: the baseline fragment
    /// depth for the orthographic (distant) trace (the perspective trace
    /// overrides it per fragment from the hit point).
    depth: f32,
    /// Same-system eclipse occluders: xyz = center (render frame km), w =
    /// caster sphere radius (km; 0 = unused slot). Luna carries Terra here
    /// (the lunar eclipse); planets carry none today.
    occluders: [[f32; 4]; MAX_OCCLUDERS],
    /// 1.0 = perspective (eye-ray) trace for a near/orbited body; 0.0 =
    /// orthographic (parallel-ray) trace for a distant one. See
    /// `PLANET_PERSPECTIVE_MIN_ARCSEC`.
    perspective: f32,
    _pad: [f32; 3],
}

/// Long-lived GPU resources for one impostor body: its per-frame impostor
/// uniform and the group-1 bind group (uniform + texture + sampler) bound when
/// it is drawn. One per body, in `IMPOSTOR_BODIES` order. (No mesh: every
/// body is drawn as a single shader impostor quad.)
struct PlanetGpu {
    /// Per-body `PlanetUniform`, rewritten each frame in `prepare`.
    uniform: wgpu::Buffer,
    /// group-1 bind group: this body's uniform + texture + the shared
    /// sampler.
    bind_group: wgpu::BindGroup,
}

/// Triaxial semi-axes (km) of an impostor body, in the shared body-frame
/// convention (+X 90 deg east, +Y the rotation pole, +Z the prime meridian).
/// Luna's come from its geometry module (genuinely triaxial - the long axis
/// faces Terra), the planets' from theirs (oblate: rx = rz).
fn impostor_radii_km(body: CelestialBody) -> Vec3 {
    match body {
        CelestialBody::TerraSystem(TerraSystemEntity::Luna) => Vec3::new(
            luna::RADIUS_ALONGORBIT_KM as f32,
            luna::RADIUS_POLAR_KM as f32,
            luna::RADIUS_SUBTERRA_KM as f32,
        ),
        _ => body.radii_km(),
    }
}

/// One on-screen satellite marker, as instance data for the marker pipeline.
/// Layout must match the marker instance attributes in `vs_marker`
/// (scene.wgsl). One instance is drawn per tracked satellite.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MarkerInstance {
    /// World-frame position (km).
    position: [f32; 3],
    /// Visible flag: 1.0 = drawn, 0.0 = hidden (occluded by the body; the
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

/// One predicted-orbit-path segment, as instance data for the path pipeline.
/// Layout must match `PathInstance` in `vs_path` (scene.wgsl): four vec4s.
/// Besides its two endpoints, each segment carries the neighboring sample on
/// each side so the vertex shader can miter the joints (both quads at a joint
/// offset the shared endpoint identically - watertight, no overlap; see the
/// shader comment for why overlap is visible).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PathInstance {
    /// The sample before the segment (render frame, km); equals `p0` at the
    /// path start, degenerating that joint to a butt end.
    prev: [f32; 3],
    _pad0: f32,
    /// Segment start (render frame, km).
    p0: [f32; 3],
    /// Fade alpha at the start (1 at the satellite, 0 one period ahead).
    alpha0: f32,
    /// Segment end (render frame, km).
    p1: [f32; 3],
    /// Fade alpha at the end.
    alpha1: f32,
    /// The sample after the segment; equals `p1` at the path end.
    next: [f32; 3],
    _pad1: f32,
}

/// Allocates a path-instance vertex buffer sized for `capacity` segments.
fn make_path_buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("path instances"),
        size: u64::from(capacity) * std::mem::size_of::<PathInstance>() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The predicted path's fade profile at `t` = fraction of the orbital period:
/// full opacity until [`PATH_FADE_START`], then one smoothstep down to zero at
/// a full period, so the line ends sharply rather than tapering forever.
fn path_fade(t: f32) -> f32 {
    let s = ((t - PATH_FADE_START) / (1.0 - PATH_FADE_START)).clamp(0.0, 1.0);
    1.0 - s * s * (3.0 - 2.0 * s)
}

/// Owns every long-lived wgpu object for the scene: textures, LUTs,
/// mesh buffers, and the six render pipelines (terra surface, atmosphere,
/// stars, markers, orbit paths, body impostor). The shared scene core,
/// owned by each binary's presenter: the windowed `Gfx` (main binary) or the
/// `OffscreenRenderer` (headless binary).
pub(crate) struct SceneRenderer {
    render_pipeline: wgpu::RenderPipeline,
    atmosphere_pipeline: wgpu::RenderPipeline,
    stars_pipeline: wgpu::RenderPipeline,
    marker_pipeline: wgpu::RenderPipeline,
    /// The predicted-orbit-path pipeline (`vs_path`/`fs_path`): thick
    /// screen-space-expanded segments, alpha-blended, depth-TESTED (`Greater`,
    /// no write) so solid bodies occlude the path's far side.
    path_pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    /// The single body impostor pipeline (`vs_planet`/`fs_planet`), shared by
    /// the seven planets AND Luna; each draw swaps its group-1 bind group. No
    /// vertex buffer (the quad is built from the vertex index); writes
    /// per-fragment depth so bodies occlude one another and Terra occludes
    /// them.
    planet_pipeline: wgpu::RenderPipeline,
    /// Per-body GPU resources, in `IMPOSTOR_BODIES` order (planets then
    /// Luna). Always built (textures upload at init), but only drawn for the
    /// bodies visible this frame.
    planets: Vec<PlanetGpu>,
    /// Indices into `planets` of the bodies to draw this frame (those whose
    /// center projects in front of the camera; Luna only within the Terra
    /// system). Rebuilt each `prepare`. Order is irrelevant: the impostor
    /// depth-tests, so occlusion is resolved by the depth buffer, not draw
    /// order.
    planet_draw_indices: Vec<usize>,
    /// Whether to draw the Terra system this frame: the Terra surface, the
    /// atmosphere, the Luna impostor, and any satellite markers. True only
    /// when the camera target is Terra or Luna (render origin at Terra, so
    /// their absolute geometry is also the local geometry). When orbiting a
    /// planet these are skipped - Terra's atmosphere physics is Terra-centered
    /// and meaningless billions of km away, and drawing it there would
    /// otherwise need the absolute (imprecise) world.
    draw_terra_system: bool,
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
    /// Per-segment predicted-orbit-path instance data (one segment per
    /// instance, `PATH_SEGMENTS` per satellite), rebuilt each `prepare` by
    /// propagating every marker's `Propagation` (analytic SGP4 or numerical
    /// orbitprop) one period ahead. Same grow-on-demand, bind-group-free
    /// pattern as `markers`.
    paths: wgpu::Buffer,
    /// Number of segments `paths` can hold without reallocation.
    path_capacity: u32,
    /// Number of path segments written for the current frame.
    path_count: u32,
}

impl SceneRenderer {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        let mesh = mesh::wgs84_ellipsoid(STACKS, SLICES);

        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scene vertices"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scene indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let markers = make_marker_buffer(device, INITIAL_MARKER_CAPACITY);
        let paths = make_path_buffer(device, INITIAL_PATH_CAPACITY);

        // The five Terra/star textures are downloaded verbatim by the build
        // script (original JPEG/TIFF) and embedded; they are decoded with the
        // `image` crate and uploaded as uncompressed RGBA8 here - no GPU
        // compression feature required (see request_adapter_device). The three
        // atmosphere LUTs are still baked into f16 KTX2 by the build script and
        // uploaded as-is. Each entry's `TexKind` tells the parallel loader which
        // path to take: an sRGB color image, a linear data image, or an f16 LUT.
        // (The Luna texture belongs to Luna's impostor bind group, so it loads
        // with the group-1 batch below, not here.)
        //
        // The eight loads are mutually independent, and shader-module
        // compilation (naga parse + validation) is independent of all of them,
        // so the module is compiled on one rayon task while the textures decode
        // and upload in parallel across the rest of the pool. Device, Queue, and
        // the produced views/module are all Send + Sync. (Decoding 33 MP images
        // is CPU-heavy, so this parallelism matters more than it did for the old
        // memcpy-only BC7 uploads.)
        let texture_inputs: [(&str, &[u8], TexKind); 8] = [
            (
                "terra day texture",
                include_bytes!(concat!(env!("OUT_DIR"), "/8k_earth_daymap.jpg")),
                TexKind::ColorSrgb,
            ),
            (
                "terra night texture",
                include_bytes!(concat!(env!("OUT_DIR"), "/8k_earth_nightmap.jpg")),
                TexKind::ColorSrgb,
            ),
            (
                "terra normal texture",
                include_bytes!(concat!(env!("OUT_DIR"), "/8k_earth_normal_map.tif")),
                TexKind::DataLinear,
            ),
            (
                "terra specular texture",
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
        ]: [wgpu::TextureView; 8] = views
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
            label: Some("terra sampler"),
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
            ],
        });

        // --- Impostor bodies (group 1): the seven planets + Luna ---
        // Each body's texture + per-body model uniform live in their own
        // bind group, used only by the impostor pipeline, so the eight body
        // textures never enter the shared group-0 layout (whose 8 sampled
        // textures stay well under the portable 16-per-stage limit, leaving room
        // for Saturn's rings later). The textures decode in parallel like the
        // others. Order matches IMPOSTOR_BODIES (planet::ALL then Luna).
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

        // The embedded body albedo maps, in IMPOSTOR_BODIES order. The
        // literal include_bytes! paths must match `CelestialBody::texture_file()`
        // for the planets (the single source of the planet<->file mapping) plus
        // Luna's map last; build.rs downloads exactly these names into OUT_DIR.
        let planet_texture_bytes: [(&str, &[u8]); 8] = [
            (
                "8k_mercury.jpg",
                include_bytes!(concat!(env!("OUT_DIR"), "/8k_mercury.jpg")),
            ),
            (
                "8k_venus_surface.jpg",
                include_bytes!(concat!(env!("OUT_DIR"), "/8k_venus_surface.jpg")),
            ),
            (
                "8k_mars.jpg",
                include_bytes!(concat!(env!("OUT_DIR"), "/8k_mars.jpg")),
            ),
            (
                "8k_jupiter.jpg",
                include_bytes!(concat!(env!("OUT_DIR"), "/8k_jupiter.jpg")),
            ),
            (
                "8k_saturn.jpg",
                include_bytes!(concat!(env!("OUT_DIR"), "/8k_saturn.jpg")),
            ),
            (
                "2k_uranus.jpg",
                include_bytes!(concat!(env!("OUT_DIR"), "/2k_uranus.jpg")),
            ),
            (
                "2k_neptune.jpg",
                include_bytes!(concat!(env!("OUT_DIR"), "/2k_neptune.jpg")),
            ),
            (
                "8k_moon.jpg",
                include_bytes!(concat!(env!("OUT_DIR"), "/8k_moon.jpg")),
            ),
        ];
        debug_assert_eq!(planet_texture_bytes.len(), IMPOSTOR_BODIES.len());
        // Sanity-tie the planet slots to their texture_file mapping (Luna, the
        // extra last slot, has no planet_data entry to check against).
        debug_assert!(
            planet::ALL
                .iter()
                .zip(&planet_texture_bytes)
                .all(|(body, (file, _))| body.texture_file() == *file)
        );
        let planet_views: Vec<wgpu::TextureView> = planet_texture_bytes
            .par_iter()
            .map(|&(label, bytes)| upload_image(device, queue, label, bytes, true))
            .collect();

        let planets: Vec<PlanetGpu> = planet_views
            .into_iter()
            .map(|view| {
                let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("planet uniform"),
                    size: std::mem::size_of::<PlanetUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                // The body's group-1 bind group reuses the shared `sampler`
                // (repeat U / clamp V), the same wrap Terra uses.
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

        // The planet pipeline binds group 0 (shared frame uniforms) AND group 1
        // (the per-planet uniform + texture), so it needs its own layout.
        let planet_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("planet pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout), Some(&planet_bind_group_layout)],
            immediate_size: 0,
        });

        // Reversed-Z depth state, parameterized per pass. The solid bodies
        // (Terra surface, Luna) write depth and test `Greater` (nearer = larger
        // depth) so Terra occludes the far-off Luna. The backdrop, the
        // additive atmosphere, and the screen-space markers neither write nor
        // test depth (`Always`, no write) - they keep their exact draw-order
        // behavior, layered by the order in `render`. The orbit paths are the
        // in-between: they test `Greater` without writing, so the solids
        // occlude them but the translucent line occludes nothing.
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
                label: Some("terra surface pipeline"),
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
                // The backdrop must never occlude the (more distant) Luna, so
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

        // The predicted orbit paths: one screen-space-expanded quad per orbit
        // segment (corners from the vertex index, endpoints + fade alphas from
        // the instance buffer), alpha-blended. Unlike the markers this pass
        // depth-TESTS (`Greater`, no write): the solid bodies drawn earlier
        // occlude the path's far side, while the translucent line neither
        // occludes anything nor self-occludes.
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
                        // 0 => prev, 1 => seg0 (endpoint + alpha), 2 => seg1,
                        // 3 => next.
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
                        // Standard alpha blend for the antialiased edge + fade.
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
                depth_stencil: Some(depth_state(false, wgpu::CompareFunction::Greater)),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let (
            render_pipeline,
            (atmosphere_pipeline, (stars_pipeline, (marker_pipeline, path_pipeline))),
        ) = rayon::join(make_render_pipeline, || {
            rayon::join(make_atmosphere_pipeline, || {
                rayon::join(make_stars_pipeline, || {
                    rayon::join(make_marker_pipeline, make_path_pipeline)
                })
            })
        });

        // The single body impostor pipeline (the planets + Luna): the
        // two-group layout (so it reuses each body's group-1 bind group), no
        // vertex buffer (the quad is built from the vertex index), and the
        // reversed-Z solid-body depth setup - the impostor writes per-fragment
        // depth, so bodies occlude one another and Terra occludes them, just
        // like a mesh would. Built after the join (it borrows `planet_layout`).
        // No back-face cull: the quad's winding is irrelevant (it is
        // camera-facing).
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
            render_pipeline,
            atmosphere_pipeline,
            stars_pipeline,
            marker_pipeline,
            path_pipeline,
            vertices,
            indices,
            index_count: mesh.indices.len() as u32,
            planet_pipeline,
            planets,
            planet_draw_indices: Vec::new(),
            draw_terra_system: true,
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

    /// Writes the per-frame uniforms, marker instances, and predicted
    /// orbit-path instances from the simulation's `RenderState`. Call before
    /// submitting the frame's command buffer; `queue.write_buffer` is ordered
    /// before it. `viewport` is the surface size in pixels (width, height),
    /// used only for the screen-space markers and path widths. Takes
    /// `&mut self` (and `&Device`) because the marker and path instance
    /// buffers grow on demand when more satellites are tracked than they
    /// currently hold. All camera/astronomical math is done by the simulation
    /// (each scenario's clock + celestial sphere), except the orbit-path
    /// propagation
    /// (`satellite::orbit_path_inertial`), which runs here from each marker's
    /// `Propagation` (analytic SGP4 or numerical orbitprop); otherwise this
    /// just packs finished values into the GPU layout.
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render: &RenderState,
        viewport: (f32, f32),
    ) {
        let (width, height) = viewport;
        let aspect = width / height.max(1.0);

        // Derive the whole celestial scene from the frame's time: Sol, Luna, and
        // the seven planets' positions + orientations, plus the star-texture
        // matrix. The simulation core evaluates the same `CelestialSphere::at`,
        // so the two agree exactly - which is what makes the orbited body's
        // render-frame position a bit-exact zero below.
        let celestial = CelestialSphere::at(&render.time);

        // Everything the GPU sees is in the RENDER FRAME: positions relative to
        // the camera target's center (its render origin). The renderer does the
        // subtraction here on the CPU so the shader only ever handles small,
        // target-local coordinates - the orbited body lands exactly at the
        // origin (its absolute center IS the origin, a bit-exact zero), which is
        // what keeps far planets from jittering.
        let origin = render.camera_target.render_origin(&celestial);

        // Rebuild the view-projection from the camera rig (position + direction).
        // The near plane scales with the orbited body's radius; the eye is
        // already in the render frame, so the matrix is target-local. The
        // inverse lets the planet impostor reconstruct per-fragment eye rays.
        let radius = render.camera_target.mean_radius_km();
        let near = NEAR_PLANE_RADII * radius;
        // The far plane must enclose everything drawn with real depth: the
        // orbited body's far side (eye-to-center + its radius) and, for
        // Terra/Luna, the sibling body 384,000+ km away. `camera_pos` is the eye
        // in the render frame (origin at the orbited body's center for a planet,
        // at Terra for Terra/Luna), so `|camera_pos| + 2*radius` covers the
        // orbited body at any zoom; the `FAR_PLANE_KM` floor covers the
        // Terra-Luna system + the camera-centered star shell. Without this a
        // large planet at max zoom-out (Jupiter's eye-to-center ~770,000 km) sits
        // beyond a fixed 500,000 km plane and its whole disc is clipped away.
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

        // star_tex_rot_inv (world -> galactic texture frame) is uploaded as
        // `star_rot_inv`; its mat3x3 columns are padded to vec4 stride.
        let star_cols = celestial.star_tex_rot_inv.to_cols_array_2d();

        // Luna's placement feeds the group-0 uniforms for the two passes that
        // must know about Luna without drawing it (the solar-eclipse shadow in
        // fs_main, the atmosphere's occlusion check); its shadow-caster radius
        // is implied by the identity (`luna::MEAN_RADIUS_KM`). Luna itself
        // draws through the shared body impostor below.
        let luna_pos_world = celestial
            .body(CelestialBody::TerraSystem(TerraSystemEntity::Luna))
            .map_or(Vec3::ZERO, |state| state.placement.pos_world);

        let uniforms = Uniforms {
            view_proj: view_proj.to_cols_array(),
            inv_view_proj: inv_view_proj.to_cols_array(),
            camera_pos: render.camera_pos.to_array(),
            _pad0: 0.0,
            star_rot_inv: std::array::from_fn(|c| {
                [star_cols[c][0], star_cols[c][1], star_cols[c][2], 0.0]
            }),
            marker: [width, height, MARKER_RADIUS_PX, 0.0],
            luna_pos: (luna_pos_world - origin).to_array(),
            _pad2: 0.0,
            luna_params: [
                luna::MEAN_RADIUS_KM,
                terra::MEAN_RADIUS_KM,
                SOL_ANGULAR_RADIUS_RAD,
                0.0,
            ],
            sol_pos: (celestial.sol_pos_world - origin).to_array(),
            _pad4: 0.0,
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));

        // The Terra system (Terra surface + atmosphere + Luna + markers) renders
        // only when orbiting Terra or Luna, i.e. the render origin is at Terra.
        // Orbiting a planet, the Terra/Luna are a far speck and the atmosphere
        // is Terra-centered physics, so they are skipped.
        self.draw_terra_system = origin == Vec3::ZERO;

        // One impostor uniform per body visible this frame (the planets +
        // Luna). The CPU projects each body's center to screen space (NDC
        // center + quad half-extent + depth) and the GPU draws a single quad
        // there, ray-tracing the triaxial ellipsoid in its fragment shader.
        // `celestial.bodies` is a flat list (Terra, Luna, planets); every
        // entry with an `IMPOSTOR_BODIES` GPU slot drives this loop (Terra is
        // a real mesh and has no slot). Terra/Luna scenarios (origin at Terra)
        // still carry the planets here, but they project far off-screen /
        // behind the camera and are mostly sub-pixel specks.
        self.planet_draw_indices.clear();
        let tan_half_fov = (FOV_Y_DEG / 2.0).to_radians().tan();
        for state in &celestial.bodies {
            let body = state.body;
            let Some(i) = IMPOSTOR_BODIES
                .iter()
                .position(|candidate| *candidate == body)
            else {
                continue;
            };
            // Luna is part of the Terra system: orbiting a planet it is a
            // sub-pixel speck next to the skipped Terra, so it is skipped with
            // the rest of the system (same gate as the Terra surface).
            if body == CelestialBody::LUNA && !self.draw_terra_system {
                continue;
            }
            let pos_render = state.placement.pos_world - origin;
            let rel = pos_render - render.camera_pos;
            let dist = rel.length();
            if dist <= f32::EPSILON {
                continue;
            }
            // Largest semi-axis: bounds the silhouette from any view direction
            // (sizes the quad; also the orthographic offset scale in the
            // shader, which must agree).
            let radii = impostor_radii_km(body);
            let rmax = radii.max_element();

            // Project the center; skip bodies behind the camera (a planet on
            // the far side of the sky from a Terra orbit).
            let clip = view_proj * pos_render.extend(1.0);
            if clip.w <= 0.0 {
                continue;
            }
            let inv_w = 1.0 / clip.w;
            let proj_center = [clip.x * inv_w, clip.y * inv_w];
            // Clamp the baseline (center) depth into (0, 1]: a planet billions of
            // km out projects beyond the far plane (reversed-Z depth <= 0), which
            // would z-clip its quad away. Clamping keeps it drawn as a far speck
            // behind everything (see PLANET_MIN_DEPTH). The orbited body is well
            // within the frustum, so this is a no-op for it (and its perspective
            // fragments write their own true depth regardless).
            let depth = (clip.z * inv_w).clamp(PLANET_MIN_DEPTH, 1.0);

            // Apparent angular radius (tangent lines): asin(rmax/dist).
            let sin_r = (rmax / dist).min(0.999);
            let ang_radius = sin_r.asin();

            // Perspective (eye-ray) trace for a near/orbited body - f32-safe
            // because dist/rmax is small there; orthographic (parallel-ray) for a
            // distant one. The cutoff is on apparent angular DIAMETER.
            let arcsec = 2.0 * ang_radius.to_degrees() * 3600.0;
            let perspective = arcsec >= PLANET_PERSPECTIVE_MIN_ARCSEC;

            // Place the impostor quad. A distant body is a small disc, so a
            // quad at the projected center sized to the angular radius (+margin)
            // is tight and cheap. A near/orbited body is traced perspectively
            // per pixel, and its projected center can fall far off-screen at high
            // tilt (the center is far off the view axis while the near surface
            // still fills the frame) - a center-anchored quad would follow the
            // center off-screen and the body would vanish. So cover the whole
            // screen ([-1,1]^2) and let the fragment ray-trace decide coverage
            // (misses discard). At most the orbited body and (near its perigee,
            // from a Terra orbit) Luna are perspective, so this is at most two
            // full-screen passes.
            let (ndc_center, ndc_half_extent) = if perspective {
                ([0.0, 0.0], [1.0, 1.0])
            } else {
                let ang = ang_radius * PLANET_QUAD_MARGIN;
                let half_y = ang.tan() / tan_half_fov;
                (proj_center, [half_y / aspect, half_y])
            };

            // Same-system eclipse occluders (the generic mutual-shadow rule):
            // every OTHER body of this body's planetary system casts an
            // analytic sphere shadow on it - Terra shadowing Luna is the lunar
            // eclipse. Cross-system shadows are astronomically negligible, so
            // planets get all slots zeroed today.
            let mut occluders = [[0.0f32; 4]; MAX_OCCLUDERS];
            let mut occluder_count = 0;
            for other in &celestial.bodies {
                if occluder_count == MAX_OCCLUDERS {
                    break;
                }
                if !body.same_system(other.body) {
                    continue;
                }
                let occ_pos = other.placement.pos_world - origin;
                occluders[occluder_count] =
                    [occ_pos.x, occ_pos.y, occ_pos.z, other.body.mean_radius_km()];
                occluder_count += 1;
            }

            // Sol's angular radius from this body sets its penumbra softness -
            // per body, so an outer system's smaller Sol sharpens its shadows
            // automatically.
            let sol_dist = (celestial.sol_pos_world - state.placement.pos_world).length();
            let sol_angular_radius = (SOL_RADIUS_KM / sol_dist.max(SOL_RADIUS_KM)).asin();

            let rot_cols = state.placement.rot.to_cols_array_2d();
            let planet_uniform = PlanetUniform {
                rot: std::array::from_fn(|c| [rot_cols[c][0], rot_cols[c][1], rot_cols[c][2], 0.0]),
                pos: pos_render.to_array(),
                sol_angular_radius,
                ndc_center,
                ndc_half_extent,
                radii: radii.to_array(),
                depth,
                occluders,
                perspective: if perspective { 1.0 } else { 0.0 },
                _pad: [0.0; 3],
            };
            queue.write_buffer(
                &self.planets[i].uniform,
                0,
                bytemuck::bytes_of(&planet_uniform),
            );
            self.planet_draw_indices.push(i);
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

        // The predicted orbit path: propagate each marker's `Propagation`
        // (analytic SGP4 or numerical orbitprop, per object) one full period
        // ahead and turn the sample
        // points into per-segment instances with a fading tail. Recomputed
        // every frame - a paused app renders zero frames, so idle stays free.
        // Gated like the markers (Terra-system only); the origin subtraction
        // is a bit-exact no-op there (origin == 0) but keeps the render-frame
        // convention that the GPU never sees absolute positions.
        self.path_count = 0;
        if self.draw_terra_system && !render.markers.is_empty() {
            // Circumscribe the orbit instead of inscribing it: a chord between
            // samples sags up to r*(1 - cos(pi/N)) (~0.5 km) inside the true
            // arc, and where the path grazes Terra's limb that dip fails the
            // depth test at chord midpoints only - the line renders as dashes.
            // Radially lifting every sample by sec(pi/N) puts the chord
            // MIDPOINTS on the true curve (endpoints half a sagitta out,
            // sub-pixel), so the polyline never falsely dips behind the limb.
            let lift = 1.0 / (std::f32::consts::PI / PATH_SEGMENTS as f32).cos();
            let mut segments = Vec::with_capacity(render.markers.len() * PATH_SEGMENTS);
            for marker in &render.markers {
                let points: Vec<Vec3> = satellite::orbit_path_inertial(
                    &marker.propagation,
                    &render.time,
                    PATH_SEGMENTS,
                )
                .into_iter()
                .map(|p| p * lift - origin)
                .collect();
                // A manually-controlled satellite burned to escape (e >= 1)
                // has no period, so its path comes back empty - no line.
                if points.is_empty() {
                    continue;
                }
                for i in 0..PATH_SEGMENTS {
                    segments.push(PathInstance {
                        // Clamped neighbors: the first/last joint duplicates
                        // its endpoint, which the shader treats as a butt end.
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
        render_pass.set_vertex_buffer(0, self.vertices.slice(..));
        render_pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);

        // Backdrop first; it always draws (the stars/Sol frame every body).
        render_pass.set_pipeline(&self.stars_pipeline);
        render_pass.draw_indexed(0..self.index_count, 0, 0..1);

        // Every non-Terra body (the planets + Luna) as a shader impostor: one
        // camera-facing quad each (no vertex buffer - built from the vertex
        // index), placed in screen space by `prepare` and ray-traced in the
        // fragment shader. The impostor writes per-fragment depth (reversed-Z,
        // same as the solid bodies), so the depth buffer resolves
        // body-vs-body and Terra-vs-body occlusion - draw order does not
        // matter. group 0 stays bound; group 1 swaps per body.
        if !self.planet_draw_indices.is_empty() {
            render_pass.set_pipeline(&self.planet_pipeline);
            for &i in &self.planet_draw_indices {
                render_pass.set_bind_group(1, &self.planets[i].bind_group, &[]);
                render_pass.draw(0..6, 0..1);
            }
        }

        // The Terra surface (which writes depth), then the atmosphere over the
        // disc and limb - drawn only when orbiting the Terra/Luna (the render
        // origin is at Terra). Orbiting a planet they would be a far speck
        // (and the Terra-centered atmosphere physics is meaningless), so they
        // are skipped, leaving just the impostors + backdrop. Terra draws
        // after the impostors; the depth buffer keeps Luna / a planet behind
        // Terra hidden, and one in front (only ever Luna, from a Luna orbit)
        // survives.
        if self.draw_terra_system {
            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.draw_indexed(0..self.index_count, 0, 0..1);

            render_pass.set_pipeline(&self.atmosphere_pipeline);
            render_pass.draw_indexed(0..self.index_count, 0, 0..1);

            // The predicted orbit paths, before the markers so each marker dot
            // sits on top of its own line. Depth-tested against the solids
            // drawn above (the far side of an orbit hides behind Terra).
            if self.path_count > 0 {
                render_pass.set_pipeline(&self.path_pipeline);
                render_pass.set_vertex_buffer(0, self.paths.slice(..));
                render_pass.draw(0..6, 0..self.path_count);
            }

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

/// Which upload path a [`SceneRenderer`] texture input takes.
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
/// expected now - the Terra/star textures use [`upload_image`] instead.
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
