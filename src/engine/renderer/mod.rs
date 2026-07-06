use rayon::prelude::*;
use wgpu::util::DeviceExt;

use glam::{DMat4, DVec3, DVec4};

use crate::engine::planet;
use crate::engine::simulation::celestial_sphere::CelestialSphere;
use crate::engine::simulation::satellite;
use crate::engine::simulation::{CameraTarget, CelestialBody, RenderState};

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

/// Top-of-atmosphere shell radius (km), the CPU twin of `ATMOSPHERE_TOP_KM`
/// in scene.wgsl (and `build.rs mod atmosphere`) - all three must stay in
/// sync. `prepare` sizes the atmosphere quad's silhouette from it.
const ATMOSPHERE_TOP_KM: f64 = 6460.0;

/// Sol's physical radius (km): the per-body Sol angular radius uploaded with
/// each impostor is `asin(SOL_RADIUS_KM / distance-to-Sol)`.
const SOL_RADIUS_KM: f64 = 695_700.0;

/// Eclipse-occluder slots in each impostor's uniform (unused slots have
/// radius 0). One suffices for the Terra system (Terra shadowing Luna); sized
/// for a future moon system's worth of same-system casters. Must match
/// `MAX_OCCLUDERS` in scene.wgsl.
const MAX_OCCLUDERS: usize = 4;

/// Every body drawn as a shader impostor - ALL of them, Terra included - in
/// GPU-slot order (the renderer's per-body uniform/bind-group arrays): the
/// seven planets in `planet::ALL` order, then Luna, then Terra. Built from
/// `planet::ALL` so the two never drift; Luna and Terra sit last (in that
/// order), so every pre-existing slot keeps its index.
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
const PLANET_PERSPECTIVE_MIN_ARCSEC: f64 = 1800.0;

/// The impostor quad spans the silhouette's angular radius times this margin,
/// so the full disc (corners included) lands inside the quad; edge fragments
/// that miss the ellipse discard. Mirrors the old billboard's margin.
const PLANET_QUAD_MARGIN: f64 = 1.3;

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
const PLANET_MIN_DEPTH: f64 = 1e-6;

/// Vertical field of view, degrees. The projection authority for the scene; the
/// camera's pan math scales by the same value (`renderer::FOV_Y_DEG`).
pub const FOV_Y_DEG: f64 = 45.0;

/// Near clip plane as a fraction of the orbit target's mean radius. The
/// renderer scales it by `RenderState::camera_target.mean_radius_km()` when it
/// rebuilds the projection, so the near plane tracks whichever body is orbited.
pub const NEAR_PLANE_RADII: f64 = 0.01;

/// Far clip plane, km - a fixed value, NOT a radius multiple. It must enclose
/// Luna at apogee (~406,700 km) plus the camera distance, and - orbiting Luna -
/// Terra ~384,400 km away; the star shell (222,985 km) sits well inside it.
pub const FAR_PLANE_KM: f64 = 500_000.0;

/// Builds the reversed-Z view-projection matrix for an eye at `eye` looking at
/// `look_at` with up `up` (all in the floating-origin render frame). Reversed-Z
/// (near -> 1, far -> 0) pairs with the `Depth32Float` buffer cleared to 0 and
/// a `Greater` test, spreading depth precision across the scene's enormous
/// near/far span so Terra still occludes the far-off Luna. Lives here, not on
/// the camera, because the renderer now rebuilds the projection from
/// `RenderState`'s camera rig (the camera only emits eye / look-at / up).
/// Taking the look-at point directly (not a forward vector) keeps the view
/// exactly the one the camera implied (a re-normalized forward would drift).
/// Built entirely in f64; callers cast to f32 only at uniform upload.
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
    /// Luna as an occluder for the atmosphere pass, the one pass that must
    /// know about Luna without drawing it (Luna itself draws through the
    /// shared body impostor, group 1): xyz = Luna center in the render frame
    /// (km), w = Luna mean radius (km).
    luna_occluder: [f32; 4],
    /// Sol position in the render frame (km) = relative to the camera target.
    /// Every lit pass derives its Sol direction from this; there is no
    /// Earth-fixed `sol_dir`.
    sol_pos: [f32; 3],
    _pad4: f32,
    /// The atmosphere quad's screen placement, computed like an impostor's:
    /// xy = NDC center, zw = NDC half-extent ((0,0,1,1) = full-screen, the
    /// usual case at current camera limits).
    atmosphere_quad: [f32; 4],
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
    /// Shading-feature bits (the `BODY_FLAG_*` values, must match scene.wgsl):
    /// which optional maps/features this body's fragment shading applies.
    /// Packed from the body's `planet::Maps` + `has_atmosphere` - purely
    /// data-driven, so any body gaining a map lights up the feature.
    flags: u32,
    _pad: [u32; 2],
}

/// `PlanetUniform.flags` bits; must match the `BODY_FLAG_*` consts in
/// scene.wgsl. A bit per optional feature rather than one "rich" bit because
/// the features degrade independently (e.g. a dummy specular mask would still
/// leave a land-level GGX sheen, so it must be off entirely for maskless
/// bodies).
const BODY_FLAG_NIGHT: u32 = 1;
const BODY_FLAG_NORMAL_MAP: u32 = 2;
const BODY_FLAG_SPECULAR: u32 = 4;
const BODY_FLAG_ATMO_LIT: u32 = 8;

/// Packs a body's shading-feature bits from its data-table row: a feature is
/// on exactly when the body has the map (or the atmosphere) driving it.
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
/// mesh buffers, and the five render pipelines (atmosphere, stars, markers,
/// orbit paths, body impostor). The shared scene core, owned by each
/// binary's presenter: the windowed `Gfx` (main binary) or the
/// `OffscreenRenderer` (headless binary).
pub(crate) struct SceneRenderer {
    atmosphere_pipeline: wgpu::RenderPipeline,
    stars_pipeline: wgpu::RenderPipeline,
    marker_pipeline: wgpu::RenderPipeline,
    /// The predicted-orbit-path pipeline (`vs_path`/`fs_path`): thick
    /// screen-space-expanded segments, alpha-blended, depth-TESTED (`Greater`,
    /// no write) so solid bodies occlude the path's far side.
    path_pipeline: wgpu::RenderPipeline,
    /// The single body impostor pipeline (`vs_planet`/`fs_planet`), shared by
    /// ALL nine bodies (Terra included); each draw swaps its group-1 bind
    /// group. No vertex buffer (the quad is built from the vertex index);
    /// writes per-fragment depth - the scene's only depth-writing pass - so
    /// bodies occlude one another.
    planet_pipeline: wgpu::RenderPipeline,
    /// Per-body GPU resources, in `IMPOSTOR_BODIES` order (planets, Luna,
    /// Terra). Always built (textures upload at init), but only drawn for the
    /// bodies visible this frame.
    planets: Vec<PlanetGpu>,
    /// Indices into `planets` of the bodies to draw this frame (those whose
    /// center projects in front of the camera). Rebuilt each `prepare`. Order
    /// is irrelevant: the impostor depth-tests, so occlusion is resolved by
    /// the depth buffer, not draw order.
    planet_draw_indices: Vec<usize>,
    /// Whether to draw the atmosphere this frame: true when a body with an
    /// atmosphere (`planet::Maps`-table `has_atmosphere`; Terra today) sits
    /// exactly at the render origin - i.e. the camera target is in its
    /// system. The pass's LUT math assumes the body at the origin; from a
    /// planet orbit the Terra atmosphere is sub-pixel and its Terra-centered
    /// physics would need the absolute (imprecise) world, so it is skipped.
    draw_atmosphere: bool,
    /// Whether to draw the satellite overlays (orbit paths + markers) this
    /// frame: true when the render origin is at Terra (the camera target is
    /// Terra or Luna). Satellite positions are Terra-frame world coordinates,
    /// so from a planet orbit they are meaningless and skipped.
    draw_satellite_overlays: bool,
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
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let markers = make_marker_buffer(device, INITIAL_MARKER_CAPACITY);
        let paths = make_path_buffer(device, INITIAL_PATH_CAPACITY);

        // The star map is downloaded verbatim by the build script (original
        // JPEG) and embedded; it is decoded with the `image` crate and
        // uploaded as uncompressed RGBA8 here - no GPU compression feature
        // required (see request_adapter_device). The three atmosphere LUTs
        // are baked into f16 KTX2 by the build script and uploaded as-is.
        // Each entry's `TexKind` tells the parallel loader which path to
        // take. (Every body's maps - Terra's four included - belong to the
        // per-body impostor bind groups, so they load with the group-1 batch
        // below, not here.)
        //
        // The loads are mutually independent, and shader-module compilation
        // (naga parse + validation) is independent of all of them, so the
        // module is compiled on one rayon task while the textures decode and
        // upload in parallel across the rest of the pool. Device, Queue, and
        // the produced views/module are all Send + Sync.
        let texture_inputs: [(&str, &[u8], TexKind); 4] = [
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
                        TexKind::Lut => upload_ktx2(device, queue, label, bytes),
                    })
                    .collect::<Vec<_>>()
            },
        );

        // par_iter preserves input order, so the views line up with
        // `texture_inputs` above and the bindings below.
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
                    // The optional feature maps (night / normal / specular).
                    // Bodies without one bind a shared 1x1 dummy - the
                    // uniform's feature flags keep the shader from sampling
                    // it, but the layout needs every slot filled.
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

        // The embedded body maps, in IMPOSTOR_BODIES order. The literal
        // include_bytes! paths must match `CelestialBody::maps()` (the single
        // source of the body<->file mapping), whose names are also the upload
        // labels below; build.rs downloads exactly these names into OUT_DIR.
        // Optional feature maps are None for bodies that have none.
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

        // Shared 1x1 dummies filling the optional-map slots of bodies without
        // a real map (the feature flags keep the shader from ever sampling
        // them; contents are sensible no-op values anyway).
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

        // Decode every body's map set in parallel (the optional maps of a
        // multi-map body decode alongside the albedos).
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
                // The body's group-1 bind group reuses the shared `sampler`
                // (repeat U / clamp V), the same wrap every equirect map uses.
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
        let make_atmosphere_pipeline = || {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("atmosphere pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_atmosphere"),
                    compilation_options: Default::default(),
                    // Screen-space quad from the vertex index; no vertex buffer.
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
                    // A screen-facing quad; no culling. Coverage of the whole
                    // silhouette (incl. the limb ring beyond the disc) comes
                    // from the CPU-sized quad.
                    cull_mode: None,
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
                    // Screen-space quad from the vertex index; no vertex buffer.
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
                    // A full-screen quad; no culling.
                    cull_mode: None,
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

        let (atmosphere_pipeline, (stars_pipeline, (marker_pipeline, path_pipeline))) =
            rayon::join(make_atmosphere_pipeline, || {
                rayon::join(make_stars_pipeline, || {
                    rayon::join(make_marker_pipeline, make_path_pipeline)
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
        viewport: (f64, f64),
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
        // Terra's heliocentric center, used to bridge the Terra-frame
        // (geocentric ECEF) satellite overlays into the render frame: a
        // Terra-relative point `q` is at `terra_center + q` in the heliocentric
        // world frame, so its render-frame coordinate is `terra_center + q -
        // origin`. Satellite overlays only draw for a Terra-system target,
        // where `origin == terra_center`, so this reduces to `q`; today
        // (`terra_center == origin == 0`) it is a bit-exact no-op.
        let terra_center = celestial.center_world(CelestialBody::TERRA);

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

        // Luna's placement feeds the group-0 uniform for the one pass that
        // must know about Luna without drawing it (the atmosphere's occlusion
        // check); its occluder radius comes from the identity
        // (`mean_radius_km`), like any other body. Luna itself draws through
        // the shared body impostor below. (Luna shadowing Terra - the
        // solar-eclipse spot - rides the generic per-body occluder list, not
        // a group-0 uniform.)
        let luna_pos_world = celestial
            .body(CelestialBody::LUNA)
            .map_or(DVec3::ZERO, |state| state.placement.pos_world);

        // The atmosphere gate + quad. The pass draws when a body with an
        // atmosphere sits exactly at the render origin (the camera target is
        // in its system; for Terra this is the Terra/Luna-target case) - the
        // LUT math assumes the body at the origin, and the bit-exact
        // pos == origin equality holds because render_origin reuses the same
        // f32 center. The quad covers the top-of-atmosphere silhouette like
        // an orthographic impostor quad; it goes full-screen when the camera
        // is inside/near the shell or the shell is perspective-sized (which
        // at current camera limits is always - the tight quad is a
        // correctness net for future limits, not an active optimization).
        let tan_half_fov = (FOV_Y_DEG / 2.0).to_radians().tan();
        self.draw_atmosphere = celestial
            .bodies
            .iter()
            .any(|s| s.body.has_atmosphere() && s.placement.pos_world == origin);
        let mut atmosphere_quad = [0.0f64, 0.0, 1.0, 1.0];
        if self.draw_atmosphere {
            // The shell center is the render origin, so the camera's distance
            // to it is just |camera_pos|.
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

        // All the math above ran in f64; the casts here are the GPU boundary
        // (the uniform layout is f32).
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

        // The satellite overlays (orbit paths + markers) render only when
        // orbiting Terra or Luna, whose render origin is Terra's center: their
        // world positions are Terra-frame. Keyed off the camera target's
        // identity rather than `origin == ZERO`, because in the heliocentric
        // frame Terra's center is no longer zero. (The bodies themselves all
        // draw as impostors from every vantage; the atmosphere has its own gate
        // above.)
        self.draw_satellite_overlays = matches!(
            render.camera_target,
            CameraTarget::Body(CelestialBody::TerraSystem(_))
        );

        // One impostor uniform per body visible this frame (the planets +
        // Luna). The CPU projects each body's center to screen space (NDC
        // center + quad half-extent + depth) and the GPU draws a single quad
        // there, ray-tracing the triaxial ellipsoid in its fragment shader.
        // `celestial.bodies` is a flat list (Terra, Luna, planets); every
        // entry has an `IMPOSTOR_BODIES` GPU slot and drives this loop -
        // every body draws from every vantage. Terra/Luna scenarios (origin
        // at Terra) still carry the planets here, but they project far
        // off-screen / behind the camera and are mostly sub-pixel specks; a
        // planet scenario carries Terra/Luna the same way.
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
            // Largest semi-axis: bounds the silhouette from any view direction
            // (sizes the quad; also the orthographic offset scale in the
            // shader, which must agree).
            let radii = body.radii_km();
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
            // planets get all slots zeroed today. Cast to f32 only here, after
            // the f64 render-frame subtraction.
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

            // Sol's angular radius from this body sets its penumbra softness -
            // per body, so an outer system's smaller Sol sharpens its shadows
            // automatically. f64 like every distance here (heliocentric
            // magnitudes); the uniform cast happens at the pack below.
            let sol_dist = (celestial.sol_pos_world - state.placement.pos_world).length();
            let sol_angular_radius = (SOL_RADIUS_KM / sol_dist.max(SOL_RADIUS_KM)).asin();

            // The uniform layout is f32: every cast below is the GPU boundary.
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

        // One marker instance per tracked satellite. Grow the buffer first if
        // this frame has more markers than it currently holds (only ever
        // happens once, since the count is fixed at startup).
        let instances: Vec<MarkerInstance> = render
            .markers
            .iter()
            .map(|m| MarkerInstance {
                // Terra-frame ECEF -> render frame (see `terra_center` above).
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

        // The predicted orbit path: propagate each marker's `Propagation`
        // (analytic SGP4 or numerical orbitprop, per object) one full period
        // ahead and turn the sample
        // points into per-segment instances with a fading tail. Recomputed
        // every frame - a paused app renders zero frames, so idle stays free.
        // Gated like the markers (Terra-system only); the `terra_center -
        // origin` bridge is a bit-exact no-op there (origin == terra_center)
        // but keeps the render-frame convention that the GPU never sees
        // absolute positions.
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
                // Terra-frame ECEF -> render frame (see `terra_center` above);
                // the f32 cast is the GPU boundary (instance-buffer layout).
                .map(|p| (p * lift + terra_center - origin).as_vec3())
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

        // Backdrop first; it always draws (the stars/Sol frame every body).
        // A full-screen quad from the vertex index - no vertex buffer, like
        // every other pass (nothing in the scene is a mesh).
        render_pass.set_pipeline(&self.stars_pipeline);
        render_pass.draw(0..6, 0..1);

        // Every body - Terra included - as a shader impostor: one
        // camera-facing quad each (no vertex buffer - built from the vertex
        // index), placed in screen space by `prepare` and ray-traced in the
        // fragment shader. The impostor writes per-fragment depth (the only
        // depth-writing pass), so the depth buffer resolves body-vs-body
        // occlusion - draw order does not matter. group 0 stays bound;
        // group 1 swaps per body.
        if !self.planet_draw_indices.is_empty() {
            render_pass.set_pipeline(&self.planet_pipeline);
            for &i in &self.planet_draw_indices {
                render_pass.set_bind_group(1, &self.planets[i].bind_group, &[]);
                render_pass.draw(0..6, 0..1);
            }
        }

        // The atmosphere over Terra's disc and limb - drawn when the
        // atmosphere body sits at the render origin (see prepare). A
        // CPU-sized screen quad, additively layered over the Terra impostor
        // drawn above; it does not depth-test (see fs_atmosphere's explicit
        // Luna occlusion check).
        if self.draw_atmosphere {
            render_pass.set_pipeline(&self.atmosphere_pipeline);
            render_pass.draw(0..6, 0..1);
        }

        // The satellite overlays - drawn only when orbiting Terra/Luna (their
        // positions are Terra-frame world coordinates).
        if self.draw_satellite_overlays {
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
    /// A color map decoded from JPEG and uploaded `Rgba8UnormSrgb` (stars),
    /// so sampling linearizes the sRGB bytes on the GPU.
    ColorSrgb,
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

/// Uploads a 1x1 solid-color RGBA8 texture - the shared dummy bound in the
/// optional-map slots of bodies that have no such map (a bind group layout
/// needs every slot filled even though the shader never samples these).
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
