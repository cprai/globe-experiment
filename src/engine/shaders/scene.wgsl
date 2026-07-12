// All positions are in the RENDER FRAME: relative to the camera target's
// center (floating origin), with `view_proj` built in the same frame, so the
// GPU only sees small target-local coordinates - the orbited body sits at the
// numerical origin and far planets keep f32 precision. No Earth-fixed origin
// and no `sol_dir`: lit passes derive Sol directions from `sol_pos`.
struct Uniforms {
    view_proj: mat4x4<f32>,
    // Per-fragment eye-ray reconstruction (perspective trace; f32-safe
    // because that trace is only used for near planets).
    inv_view_proj: mat4x4<f32>,
    // Camera eye in the render frame (km).
    camera_pos: vec3<f32>,
    // Rotates a camera-relative world direction into the star texture's frame
    // for the equirectangular lookup. Ephemeris-driven (sidereal-rate);
    // includes the static galactic->equatorial offset (the texture is drawn
    // in galactic coordinates).
    star_rot_inv: mat3x3<f32>,
    // x,y = viewport size in pixels; z = marker radius in pixels; w = unused.
    marker: vec4<f32>,
    // Luna occluder for the atmosphere pass (the one pass that must know Luna
    // without drawing it): xyz = center (render frame, km), w = mean radius
    // (km). See fs_atmosphere.
    luna_occluder: vec4<f32>,
    // Sol position in the render frame (km): lights every body and aims the
    // backdrop Sol disc.
    sol_pos: vec3<f32>,
    // Atmosphere quad placement, CPU-computed: xy = NDC center, zw = NDC
    // half-extent (0,0,1,1 = full-screen, the usual case).
    atmosphere_quad: vec4<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var map_sampler: sampler;
@group(0) @binding(2) var transmittance_lut: texture_2d<f32>;
@group(0) @binding(3) var lut_sampler: sampler;
@group(0) @binding(4) var inscatter_rayleigh_lut: texture_2d<f32>;
@group(0) @binding(5) var inscatter_mie_lut: texture_2d<f32>;
@group(0) @binding(6) var stars_texture: texture_2d<f32>;

// Two triangles covering [-1, 1]^2.
fn quad_corner(vertex_index: u32) -> vec2<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    return corners[vertex_index];
}

// View ray through an NDC point: unproject the reversed-Z near (z=1) and far
// (z=0) planes and take the chord direction - exactly the eye ray the
// projection rasterizes through that pixel.
fn view_ray_dir(ndc: vec2<f32>) -> vec3<f32> {
    let near_h = uniforms.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    let far_h = uniforms.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0);
    return normalize(far_h.xyz / far_h.w - near_h.xyz / near_h.w);
}

const DAY_AMBIENT: f32 = 0.04;
// Normal-map perturbation strength; deliberately past photorealism.
const NORMAL_STRENGTH: f32 = 4.5;
// Rough land / smooth ocean; the specular map blends between them. The ocean
// value sets the GGX glint width (0.45 ~ wave-roughened sea).
const LAND_ROUGHNESS: f32 = 0.9;
const OCEAN_ROUGHNESS: f32 = 0.45;
// Dielectric reflectance at normal incidence.
const LAND_F0: f32 = 0.015;
const OCEAN_F0: f32 = 0.15;

const PI: f32 = 3.14159265;

// Ocean-glint wave texture: scale in noise cells across the equirect map,
// strength = +/- fraction of glint modulated (keep low - texture, not
// sparkle).
const WAVE_SCALE: f32 = 2200.0;
const WAVE_STRENGTH: f32 = 0.04;

// --- Emissive city lights (from the night map's brightness) ---
const EMISSIVE_THRESHOLD: f32 = 0.05;
const EMISSIVE_SOFTNESS: f32 = 0.1;
// STRENGTH > 1 drives the core toward clip (LDR).
const EMISSIVE_COLOR: vec3<f32> = vec3<f32>(1.0, 0.85, 0.3);
const EMISSIVE_STRENGTH: f32 = 1.5;
// Dither-dissolve over Sol cosine: starts at FADE_START, completes at
// FADE_END. Positive END deliberately lets lights bleed past the terminator
// (cos_sol = 0) onto the daylit side.
const EMISSIVE_FADE_START: f32 = -0.15;
const EMISSIVE_FADE_END: f32 = 0.15;
// Noise grain (cells across the unit normal sphere). Fixed - no terminator
// ramp - for a temporally coherent dissolve under Sol motion.
const DITHER_SCALE: f32 = 400.0;
// Unlit-hemisphere day-map multiplier. Intentionally > 1: the night side
// reads a touch brighter than full daylight.
const NIGHT_DARKNESS: f32 = 1.2;

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);

    let a = hash2(i);
    let b = hash2(i + vec2<f32>(1.0, 0.0));
    let c = hash2(i + vec2<f32>(0.0, 1.0));
    let d = hash2(i + vec2<f32>(1.0, 1.0));

    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Two octaves of value noise for a natural, non-repeating wave texture.
fn wave_noise(uv: vec2<f32>) -> f32 {
    let n1 = value_noise(uv * WAVE_SCALE);
    let n2 = value_noise(uv * WAVE_SCALE * 2.3);
    return n1 * 0.65 + n2 * 0.35;
}

// Integer-lattice bit-mixing hash: precision-safe at large coordinates,
// unlike fract(sin(...)) - n_geo * DITHER_SCALE pushes lattice indices into
// the hundreds, where f32 sin() bands visibly. p is integer-valued (the
// floored cell corner).
fn hash3(p: vec3<f32>) -> f32 {
    var n = (u32(i32(p.x)) * 1597334677u)
        ^ (u32(i32(p.y)) * 3812015801u)
        ^ (u32(i32(p.z)) * 2369874511u);
    n = (n ^ (n >> 15u)) * 2246822519u;
    n = (n ^ (n >> 13u)) * 3266489917u;
    n = n ^ (n >> 16u);
    return f32(n) / 4294967295.0;
}

// Trilinear 3D value noise. Sampled at the unit geodetic normal: no seam, no
// pole pinch.
fn value_noise_3d(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);

    let c000 = hash3(i + vec3<f32>(0.0, 0.0, 0.0));
    let c100 = hash3(i + vec3<f32>(1.0, 0.0, 0.0));
    let c010 = hash3(i + vec3<f32>(0.0, 1.0, 0.0));
    let c110 = hash3(i + vec3<f32>(1.0, 1.0, 0.0));
    let c001 = hash3(i + vec3<f32>(0.0, 0.0, 1.0));
    let c101 = hash3(i + vec3<f32>(1.0, 0.0, 1.0));
    let c011 = hash3(i + vec3<f32>(0.0, 1.0, 1.0));
    let c111 = hash3(i + vec3<f32>(1.0, 1.0, 1.0));

    let x00 = mix(c000, c100, u.x);
    let x10 = mix(c010, c110, u.x);
    let x01 = mix(c001, c101, u.x);
    let x11 = mix(c011, c111, u.x);
    let y0 = mix(x00, x10, u.y);
    let y1 = mix(x01, x11, u.y);
    return mix(y0, y1, u.z);
}

// --- Atmosphere, after Hillaire 2020. Lengths in km. ---
// The medium definition and scattering integrals are baked into LUTs by
// `mod atmosphere` in build.rs; these geometric constants must match the
// Rust twins there.
const PLANET_RADIUS_KM: f32 = 6360.0;
const ATMOSPHERE_TOP_KM: f32 = 6460.0;
const MIE_G: f32 = 0.8;

const SOL_INTENSITY: f32 = 12.0;

// Transmittance from radius `r` km toward Sol at zenith cosine `mu`, via the
// precomputed LUT (Bruneton parameterization).
fn sol_transmittance(r: f32, mu: f32) -> vec3<f32> {
    // The planet shadows everything below the local horizon.
    let sin_horizon = PLANET_RADIUS_KM / r;
    let cos_horizon = -sqrt(max(1.0 - sin_horizon * sin_horizon, 0.0));
    if mu < cos_horizon {
        return vec3<f32>(0.0);
    }

    let rp = PLANET_RADIUS_KM;
    let ra = ATMOSPHERE_TOP_KM;
    let h_top = sqrt(ra * ra - rp * rp);
    let rho = sqrt(max(r * r - rp * rp, 0.0));

    // Distance to the top of the atmosphere along the ray.
    let d = -r * mu + sqrt(max(r * r * (mu * mu - 1.0) + ra * ra, 0.0));
    let d_min = ra - r;
    let d_max = rho + h_top;

    let x_mu = clamp((d - d_min) / max(d_max - d_min, 1e-4), 0.0, 1.0);
    let x_r = clamp(rho / h_top, 0.0, 1.0);

    return textureSampleLevel(
        transmittance_lut,
        lut_sampler,
        vec2<f32>(x_mu, x_r),
        0.0,
    ).rgb;
}

// Entry/exit distances of a ray against a sphere centered on the origin,
// or (-1, -1) on a miss.
fn ray_sphere(origin: vec3<f32>, dir: vec3<f32>, radius: f32) -> vec2<f32> {
    let b = dot(origin, dir);
    let c = dot(origin, origin) - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return vec2<f32>(-1.0, -1.0);
    }
    let s = sqrt(disc);
    return vec2<f32>(-b - s, -b + s);
}

// Fraction of disk 1 (radius r1) covered by disk 2 (radius r2) with centers
// `sep` apart - the standard two-circle lens-area overlap over disk 1's area.
// Used for the eclipse soft shadow with angular radii (disk 1 = Sol, disk 2
// the occluder); 0 = no overlap, 1 = fully covered.
fn disk_overlap_fraction(sep: f32, r1: f32, r2: f32) -> f32 {
    if sep >= r1 + r2 {
        return 0.0;
    }
    if sep <= abs(r1 - r2) {
        // One disk lies entirely within the other.
        let rmin = min(r1, r2);
        return rmin * rmin / (r1 * r1);
    }
    let r1s = r1 * r1;
    let r2s = r2 * r2;
    let a1 = acos(clamp((sep * sep + r1s - r2s) / (2.0 * sep * r1), -1.0, 1.0));
    let a2 = acos(clamp((sep * sep + r2s - r1s) / (2.0 * sep * r2), -1.0, 1.0));
    let tri = 0.5
        * sqrt(max((-sep + r1 + r2) * (sep + r1 - r2) * (sep - r1 + r2) * (sep + r1 + r2), 0.0));
    let area = r1s * a1 + r2s * a2 - tri;
    return area / (PI * r1s);
}

// Fraction of sunlight at surface point `p` (world km) NOT blocked by a
// spherical occluder of radius `occ_radius` at `occ`, Sol toward unit `sol`.
// 1 = fully lit, 0 = umbra; the penumbra is soft because Sol subtends
// `sol_ang` rad (passed in - it differs per planetary system). Spheres are
// exact enough: the penumbra dwarfs the triaxial detail.
fn sol_visibility(p: vec3<f32>, sol: vec3<f32>, occ: vec3<f32>, occ_radius: f32, sol_ang: f32) -> f32 {
    let oc = occ - p;
    let t = dot(oc, sol);
    // The occluder must lie toward Sol to cast a shadow here.
    if t <= 0.0 {
        return 1.0;
    }
    let perp = length(oc - sol * t);
    let ang_sep = atan(perp / t);
    let ang_occ = atan(occ_radius / t);
    return 1.0 - disk_overlap_fraction(ang_sep, sol_ang, ang_occ);
}

// Atmosphere: a screen quad CPU-sized to the top-of-atmosphere silhouette
// (full-screen when near), additively blended after the bodies; the fragment
// shader ray-traces the spherical shell analytically (the quad only provides
// coverage + a per-fragment eye ray).
//
// Viewed from outside a sphere, the inscatter integral along any view ray is
// precomputed: a ray is identified by its impact parameter and the Sol
// cosine at its reference point (ground hit, or closest approach for limb
// rays). Two LUT samples per fragment; the phase functions are constant per
// ray and applied here.

struct AtmosphereOutput {
    @builtin(position) position: vec4<f32>,
    // Fragment NDC, for the eye-ray reconstruction.
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_atmosphere(@builtin(vertex_index) vertex_index: u32) -> AtmosphereOutput {
    // Clip z is irrelevant: the pass neither tests nor writes depth.
    let ndc = uniforms.atmosphere_quad.xy + quad_corner(vertex_index) * uniforms.atmosphere_quad.zw;
    var out: AtmosphereOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.ndc = ndc;
    return out;
}

@fragment
fn fs_atmosphere(in: AtmosphereOutput) -> @location(0) vec4<f32> {
    // Terra sits at the render origin whenever this pass draws (the CPU
    // gate), so render-frame positions are Earth-centered here.
    let origin = uniforms.camera_pos;
    let dir = view_ray_dir(in.ndc);
    let sol = normalize(uniforms.sol_pos);

    let shell = ray_sphere(origin, dir, ATMOSPHERE_TOP_KM);
    if shell.y <= 0.0 {
        return vec4<f32>(0.0);
    }

    // This pass does not depth-test, so without an explicit check the
    // additive glow bleeds over a nearer Luna (a faint spot on the lunar
    // disc from a Luna-orbit view). Drop the fragment where the ray meets
    // Luna before entering the shell.
    let luna = ray_sphere(origin - uniforms.luna_occluder.xyz, dir, uniforms.luna_occluder.w);
    if luna.y > 0.0 && luna.x > 0.0 && luna.x < shell.x {
        return vec4<f32>(0.0);
    }

    // Impact parameter: the ray's closest approach to the planet center.
    let b = length(origin - dot(origin, dir) * dir);

    // Reference point + the LUT's split row mapping: lower half is
    // ground-hitting rays, upper half limb rays. Must match the bake in
    // build.rs (mod atmosphere).
    let ground = ray_sphere(origin, dir, PLANET_RADIUS_KM);
    var reference: vec3<f32>;
    var v: f32;
    if ground.x > 0.0 {
        reference = origin + dir * ground.x;
        v = 0.5 * clamp(b / PLANET_RADIUS_KM, 0.0, 1.0);
    } else {
        reference = origin - dot(origin, dir) * dir;
        v = 0.5
            + 0.5
                * clamp(
            (b - PLANET_RADIUS_KM)
                        / (ATMOSPHERE_TOP_KM - PLANET_RADIUS_KM),
            0.0,
            1.0,
        );
    }

    let mu_ref = dot(normalize(reference), sol);
    let uv = vec2<f32>(mu_ref * 0.5 + 0.5, v);

    let sum_r = textureSampleLevel(inscatter_rayleigh_lut, lut_sampler, uv, 0.0).rgb;
    let sum_m = textureSampleLevel(inscatter_mie_lut, lut_sampler, uv, 0.0).rgb;

    // Phase functions are constant along the ray.
    let mu = dot(dir, sol);
    let phase_r = 3.0 / (16.0 * PI) * (1.0 + mu * mu);
    // Cornette-Shanks phase for Mie.
    let g2 = MIE_G * MIE_G;
    let phase_m = 3.0 / (8.0 * PI) * ((1.0 - g2) * (1.0 + mu * mu))
        / ((2.0 + g2) * pow(1.0 + g2 - 2.0 * MIE_G * mu, 1.5));

    let luminance = sum_r * phase_r + sum_m * phase_m;

    // Soft exposure roll-off keeps the bright limb from clipping.
    let color = 1.0 - exp(-luminance * SOL_INTENSITY);
    return vec4<f32>(color, 1.0);
}

// Star backdrop: a full-screen quad at infinity - every fragment is a pure
// function of the camera-relative view direction, which also makes it
// camera-centered (required for non-Terra targets: a shell anchored to the
// origin would exclude a Luna-orbit eye).

const STARS_BRIGHTNESS: f32 = 0.8;

// Sol disc in the backdrop. Real Sol subtends ~0.0046 rad; drawn larger
// because it reads better. The glow is the standard LDR brightness cheat:
// clipped-white core inside a wide soft falloff.
const SOL_ANGULAR_RADIUS: f32 = 0.012;
const SOL_GLOW_RADIUS: f32 = 0.12;
const SOL_GLOW_STRENGTH: f32 = 0.5;
const SOL_COLOR: vec3<f32> = vec3<f32>(1.0, 0.96, 0.9);

struct StarsOutput {
    @builtin(position) position: vec4<f32>,
    // Fragment NDC, for the view-direction reconstruction.
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_stars(@builtin(vertex_index) vertex_index: u32) -> StarsOutput {
    // Full-screen quad; clip z irrelevant (no depth test or write).
    let corner = quad_corner(vertex_index);
    var out: StarsOutput;
    out.position = vec4<f32>(corner, 0.0, 1.0);
    out.ndc = corner;
    return out;
}

@fragment
fn fs_stars(in: StarsOutput) -> @location(0) vec4<f32> {
    // Pure function of the camera-relative view direction - a true backdrop
    // at infinity (anchoring the lookup to a celestial-sphere surface point
    // would parallax against Sol).
    let view = view_ray_dir(in.ndc);

    // Equirectangular lookup, per fragment (not per vertex) so the dateline
    // seam doesn't smear.
    let d = normalize(uniforms.star_rot_inv * view);
    let lon = atan2(d.x, d.z);
    let uv = vec2<f32>(
        lon / (2.0 * PI) + 0.5,
        acos(clamp(d.y, -1.0, 1.0)) / PI,
    );

    let stars = textureSampleLevel(stars_texture, map_sampler, uv, 0.0).rgb;

    // Sol along the same camera-relative direction as the stars, so the two
    // stay locked under rotation and zoom. `sol_pos` is Sol relative to the
    // CAMERA TARGET, so the drawn disc agrees with the orbited body's
    // terminator (fs_planet lights from the same direction), and is
    // parallax-free under local orbit/zoom.
    let sol = normalize(uniforms.sol_pos);
    let angle = acos(clamp(dot(view, sol), -1.0, 1.0));

    // Anti-aliased disc core plus a soft glow falloff.
    let disc = 1.0
        - smoothstep(SOL_ANGULAR_RADIUS * 0.85, SOL_ANGULAR_RADIUS, angle);
    let glow = SOL_GLOW_STRENGTH
        * pow(max(1.0 - angle / SOL_GLOW_RADIUS, 0.0), 3.0);

    let color = stars * STARS_BRIGHTNESS + SOL_COLOR * (disc + glow);
    return vec4<f32>(color, 1.0);
}

// Satellite markers: constant-pixel-size circles at each tracked object's
// projected position, alpha-blended after everything else. One instanced
// draw: the quad comes from the vertex index, per-marker position +
// visibility from instance attributes. Occlusion behind the body is decided
// on the CPU; a hidden marker's quad is pushed off-screen so it emits no
// fragments.

const MARKER_FILL: vec3<f32> = vec3<f32>(1.0, 0.25, 0.2);
const MARKER_RING: vec3<f32> = vec3<f32>(1.0, 1.0, 1.0);

struct MarkerInstance {
    // World-frame marker position (km).
    @location(0) position: vec3<f32>,
    // >= 0.5 = drawn, < 0.5 = hidden (occluded by the body).
    @location(1) visible: f32,
};

struct MarkerOutput {
    @builtin(position) position: vec4<f32>,
    // Unit-square corner in [-1, 1]; its length is the disc radius.
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_marker(@builtin(vertex_index) vertex_index: u32, inst: MarkerInstance) -> MarkerOutput {
    // Two triangles covering [-1, 1]^2.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[vertex_index];

    var out: MarkerOutput;
    out.uv = corner;

    // Hidden: emit an off-screen, clipped vertex. Terra-frame satellite
    // positions are already render-frame (markers only draw with the render
    // origin at Terra).
    let clip = uniforms.view_proj * vec4<f32>(inst.position, 1.0);
    if inst.visible < 0.5 || clip.w <= 0.0 {
        out.position = vec4<f32>(0.0, 0.0, 2.0, 1.0);
        return out;
    }

    // Offset the projected center by a constant pixel radius: one pixel is
    // 2/viewport NDC units, and multiplying by clip.w pre-compensates the
    // perspective divide - the circle stays round and size-stable at any
    // depth.
    let radius_px = uniforms.marker.z;
    let ndc = corner * radius_px * 2.0 / uniforms.marker.xy;
    out.position = vec4<f32>(clip.xy + ndc * clip.w, clip.z, clip.w);
    return out;
}

@fragment
fn fs_marker(in: MarkerOutput) -> @location(0) vec4<f32> {
    let r = length(in.uv);
    // Antialias the outer edge over roughly one pixel of the unit circle.
    let aa = fwidth(r);
    let alpha = 1.0 - smoothstep(1.0 - aa, 1.0, r);
    if alpha <= 0.0 {
        discard;
    }
    // White ring around a colored fill, so the dot reads on any background.
    let ring = smoothstep(0.6 - aa, 0.6 + aa, r);
    let color = mix(MARKER_FILL, MARKER_RING, ring);
    return vec4<f32>(color, alpha);
}

// Predicted orbit path: ~one period of CPU-propagated samples drawn as a
// thick constant-pixel-width antialiased polyline. One instance per segment;
// the vertex shader expands a screen-space quad around the segment (the same
// clip.w pre-divide trick as the markers) while keeping each vertex's clip
// z/w at its CENTERLINE endpoint - the lateral offset touches only clip.xy -
// so interpolation carries exact thin-line depth across the fat quad. The
// pipeline depth-TESTS (Greater, no write): solid bodies occlude the path,
// the translucent line occludes nothing.
//
// Joints are MITERED, not capped: each instance carries its neighbor points,
// and both quads at a joint offset the shared endpoint along the same
// averaged normal - shared edge, zero overlap, zero gap. Any alpha-blended
// overlap (even just the AA fringe) would double-blend into a brighter
// "bead" at every joint, a visible periodic stitch along the line.

const PATH_WIDTH_PX: f32 = 3.0;
// Extra quad slack beyond the half width so the AA fringe is not clipped.
const PATH_AA_PAD_PX: f32 = 1.5;
// Miter length cap (in pad units): the exact miter 1/cos(half turn) diverges
// at sharp screen-space turns. A clamped joint can notch, but only at
// close-flyby foreshortening.
const PATH_MITER_LIMIT: f32 = 4.0;
const PATH_OPACITY: f32 = 0.85;
// Dim cyan-blue, distinct from the red markers riding on the line.
const PATH_COLOR: vec3<f32> = vec3<f32>(0.35, 0.65, 1.0);

struct PathInstance {
    // Neighboring sample BEFORE this segment (xyz, render-frame km); at the
    // path start it duplicates seg0, degenerating the joint to a butt end.
    @location(0) prev: vec4<f32>,
    // Segment endpoints (xyz, render-frame km); w = fade alpha (1 at the
    // satellite, 0 at one full period ahead).
    @location(1) seg0: vec4<f32>,
    @location(2) seg1: vec4<f32>,
    // Neighboring sample AFTER this segment; duplicates seg1 at the end.
    @location(3) next: vec4<f32>,
};

struct PathOutput {
    @builtin(position) position: vec4<f32>,
    // Segment endpoints in framebuffer pixels, for the fragment's distance-
    // to-line. Flat: per-segment values, not interpolants.
    @location(0) @interpolate(flat) p0_px: vec2<f32>,
    @location(1) @interpolate(flat) p1_px: vec2<f32>,
    @location(2) alpha: f32,
};

// `v` normalized, or `fallback` when `v` is too short to have a direction.
fn dir_or(v: vec2<f32>, fallback: vec2<f32>) -> vec2<f32> {
    let len = length(v);
    if len > 1e-4 {
        return v / len;
    }
    return fallback;
}

@vertex
fn vs_path(@builtin(vertex_index) vertex_index: u32, inst: PathInstance) -> PathOutput {
    // Quad corners as (endpoint selector t, lateral side).
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let corner = corners[vertex_index];

    var out: PathOutput;

    // An endpoint behind the camera cannot be screen-space expanded (its
    // projection is meaningless); emit an off-screen clipped vertex and drop
    // the whole segment - at most a 1-segment gap at the near plane.
    let clip0 = uniforms.view_proj * vec4<f32>(inst.seg0.xyz, 1.0);
    let clip1 = uniforms.view_proj * vec4<f32>(inst.seg1.xyz, 1.0);
    if clip0.w <= 0.0 || clip1.w <= 0.0 {
        out.position = vec4<f32>(0.0, 0.0, 2.0, 1.0);
        out.p0_px = vec2<f32>(0.0);
        out.p1_px = vec2<f32>(0.0);
        out.alpha = 0.0;
        return out;
    }

    // Endpoints + neighbors in framebuffer pixels (y down, matching
    // @builtin(position) in the fragment stage).
    let viewport = uniforms.marker.xy;
    let clip_prev = uniforms.view_proj * vec4<f32>(inst.prev.xyz, 1.0);
    let clip_next = uniforms.view_proj * vec4<f32>(inst.next.xyz, 1.0);
    let ndc0 = clip0.xy / clip0.w;
    let ndc1 = clip1.xy / clip1.w;
    let px0 = vec2<f32>((ndc0.x + 1.0) * 0.5 * viewport.x, (1.0 - ndc0.y) * 0.5 * viewport.y);
    let px1 = vec2<f32>((ndc1.x + 1.0) * 0.5 * viewport.x, (1.0 - ndc1.y) * 0.5 * viewport.y);

    // Neighbor directions for the miters; a degenerate (same-pixel) or
    // behind-camera neighbor falls back to the segment's own direction,
    // degenerating that joint to a butt end.
    let dir = dir_or(px1 - px0, vec2<f32>(1.0, 0.0));
    var dir_in = dir;
    if clip_prev.w > 0.0 {
        let ndc = clip_prev.xy / clip_prev.w;
        let px = vec2<f32>((ndc.x + 1.0) * 0.5 * viewport.x, (1.0 - ndc.y) * 0.5 * viewport.y);
        dir_in = dir_or(px0 - px, dir);
    }
    var dir_out = dir;
    if clip_next.w > 0.0 {
        let ndc = clip_next.xy / clip_next.w;
        let px = vec2<f32>((ndc.x + 1.0) * 0.5 * viewport.x, (1.0 - ndc.y) * 0.5 * viewport.y);
        dir_out = dir_or(px - px1, dir);
    }

    // Miter normal per end: the average of the adjoining segments' normals,
    // scaled so its projection onto this segment's normal equals the pad -
    // both quads at a joint then compute the identical offset point, meeting
    // edge-to-edge (watertight). Clamped at sharp turns; at a 180-degree
    // fold the average vanishes and the miter is meaningless - fall back to
    // the plain normal.
    let n = vec2<f32>(-dir.y, dir.x);
    let pad = PATH_WIDTH_PX * 0.5 + PATH_AA_PAD_PX;
    var m0 = dir_or(vec2<f32>(-dir_in.y, dir_in.x) + n, n);
    var m1 = dir_or(n + vec2<f32>(-dir_out.y, dir_out.x), n);
    let len0 = pad / max(dot(m0, n), 1.0 / PATH_MITER_LIMIT);
    let len1 = pad / max(dot(m1, n), 1.0 / PATH_MITER_LIMIT);

    let t = corner.x;
    let vert_px = mix(px0 + m0 * corner.y * len0, px1 + m1 * corner.y * len1, t);

    // Back to NDC, pre-multiplied by the chosen endpoint's clip w (the
    // marker trick: constant pixel width at any depth). z/w stay the
    // centerline endpoint's, so the quad carries exact thin-line depth.
    let zw = mix(vec2<f32>(clip0.z, clip0.w), vec2<f32>(clip1.z, clip1.w), t);
    let vert_ndc = vec2<f32>(
        vert_px.x / viewport.x * 2.0 - 1.0,
        1.0 - vert_px.y / viewport.y * 2.0,
    );
    out.position = vec4<f32>(vert_ndc * zw.y, zw.x, zw.y);
    out.p0_px = px0;
    out.p1_px = px1;
    out.alpha = mix(inst.seg0.w, inst.seg1.w, t);
    return out;
}

@fragment
fn fs_path(in: PathOutput) -> @location(0) vec4<f32> {
    // Pixel distance to the segment's INFINITE line: joints are mitered, so
    // the neighboring quad continues the stroke past the endpoint and a
    // finite-segment distance would fade the shared joint edge from both
    // sides. Continuous across the shared edge (adjacent lines differ only
    // by the tiny per-segment turn).
    let ba = in.p1_px - in.p0_px;
    let pa = in.position.xy - in.p0_px;
    let dist = abs(pa.x * ba.y - pa.y * ba.x) / max(length(ba), 1e-4);

    // Antialias the edge over roughly one pixel.
    let aa = fwidth(dist);
    let half_w = PATH_WIDTH_PX * 0.5;
    let edge = 1.0 - smoothstep(half_w - aa, half_w + aa, dist);
    let alpha = edge * in.alpha * PATH_OPACITY;
    if alpha <= 0.002 {
        discard;
    }
    return vec4<f32>(PATH_COLOR, alpha);
}

// Body impostor, shared by all nine bodies, each drawn with its own group-1
// bind group (per-body uniform + maps - keeps group 0's sampled-texture
// count fixed). A camera-facing quad whose fragment shader ray-traces the
// true triaxial ellipsoid, samples the maps, and lights from Sol, so
// silhouette, rotation/libration, terminator, and texture stay faithful at
// any distance - no mesh. The CPU packs the quad placement (NDC center +
// half-extent + depth), the trace mode, and the shading flags into
// PlanetUniform. Shading is data-driven per body (BODY_FLAG_*): bare
// hard-terminator Lambert up to the full Terra look. Same-system eclipses
// are analytic and generic via the per-body occluder list (sol_visibility).
//
// The trace is DISTANCE-ADAPTIVE:
// - PERSPECTIVE (eye-ray via inv_view_proj) for a near/orbited body, so the
//   foreshortened silhouette matches true perspective. f32-safe only when
//   distance/radius is modest (the intersection scales into unit-sphere
//   space, so its terms stay O(distance/radius)^2) - exactly when this
//   branch is selected.
// - ORTHOGRAPHIC (parallel-ray) for a distant body: exact when distance >>
//   radius, and free of the catastrophic f32 cancellation a perspective
//   trace suffers billions of km out (dot(O,O) ~ 1e10). The ray origin comes
//   straight from the quad-corner offset (km = offset * rmax); the huge
//   eye-relative vector is never formed.
// Either way the impostor writes per-fragment depth, so bodies occlude each
// other.

// Faint night-side fill (for Luna: terrashine + scattered light), so the
// unlit limb is not pure black.
const PLANET_AMBIENT: f32 = 0.02;

// Coppery umbral glow from sunlight refracted through the occluder's
// atmosphere - the "blood-red Luna". Only active where an occluder shadows
// the disc.
const ECLIPSE_GLOW: vec3<f32> = vec3<f32>(0.06, 0.012, 0.004);

// Occluder slots per body (unused slots have radius 0). Must match
// `MAX_OCCLUDERS` in src/engine/renderer/mod.rs.
const MAX_OCCLUDERS: u32 = 4u;

// The quad spans the silhouette angular radius times this margin (the
// orthographic offset runs [-MARGIN, MARGIN]); edge rays miss and discard.
// Must match `PLANET_QUAD_MARGIN` in src/engine/renderer/mod.rs.
const PLANET_QUAD_MARGIN: f32 = 1.3;

// Per-body impostor placement + shading (one bound per body draw).
struct PlanetUniform {
    // Body-fixed -> world rotation; a pure rotation, so it carries normals
    // too.
    rot: mat3x3<f32>,
    // Body center in the RENDER FRAME (km); exactly zero for the orbited
    // body.
    pos: vec3<f32>,
    // Sol's angular radius from this body (rad): eclipse penumbra softness.
    sol_angular_radius: f32,
    // Projected center in NDC (the quad center).
    ndc_center: vec2<f32>,
    // Quad half-extent in NDC, bounding the silhouette with margin.
    ndc_half_extent: vec2<f32>,
    // Triaxial semi-axes (km), body frame (+X east, +Y pole, +Z prime
    // meridian). rx = rz for a planet; Luna differs on all three.
    radii: vec3<f32>,
    // Reversed-Z NDC depth of the center: the orthographic frag-depth
    // baseline (perspective overrides it per fragment).
    depth: f32,
    // Same-system eclipse occluders: xyz = center (render frame km), w =
    // caster sphere radius (km; 0 = unused slot).
    occluders: array<vec4<f32>, MAX_OCCLUDERS>,
    // 1.0 = perspective trace (near/orbited), 0.0 = orthographic (distant).
    perspective: f32,
    // Shading-feature bits (BODY_FLAG_*).
    flags: u32,
};

// Must match the renderer's `BODY_FLAG_*` consts.
const BODY_FLAG_NIGHT: u32 = 1u;
const BODY_FLAG_NORMAL_MAP: u32 = 2u;
const BODY_FLAG_SPECULAR: u32 = 4u;
const BODY_FLAG_ATMO_LIT: u32 = 8u;

@group(1) @binding(0) var<uniform> planet: PlanetUniform;
@group(1) @binding(1) var planet_texture: texture_2d<f32>;
@group(1) @binding(2) var planet_sampler: sampler;
// Optional feature maps (shared 1x1 dummy when absent; the matching flag bit
// gates every sample).
@group(1) @binding(3) var planet_night: texture_2d<f32>;
@group(1) @binding(4) var planet_normal: texture_2d<f32>;
@group(1) @binding(5) var planet_specular: texture_2d<f32>;

struct PlanetOutput {
    @builtin(position) position: vec4<f32>,
    // Fragment NDC, for the perspective eye-ray reconstruction.
    @location(0) ndc: vec2<f32>,
    // View-plane offset from the body center in units of the largest
    // semi-axis (for the orthographic trace); spans [-MARGIN, MARGIN]^2.
    @location(1) offset: vec2<f32>,
};

@vertex
fn vs_planet(@builtin(vertex_index) vertex_index: u32) -> PlanetOutput {
    // Two triangles covering [-1, 1]^2.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[vertex_index];

    // Quad placed directly in NDC at the CPU-projected center + half-extent,
    // at the center's reversed-Z depth (the orthographic baseline).
    let ndc = planet.ndc_center + corner * planet.ndc_half_extent;

    var out: PlanetOutput;
    out.position = vec4<f32>(ndc, planet.depth, 1.0);
    out.ndc = ndc;
    out.offset = corner * PLANET_QUAD_MARGIN;
    return out;
}

struct PlanetFragment {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_planet(in: PlanetOutput) -> PlanetFragment {
    let radii = planet.radii;
    // Largest semi-axis: the silhouette bound the CPU sized the quad with,
    // and the scale of the orthographic quad-corner offset (must agree).
    let rmax = max(radii.x, max(radii.y, radii.z));

    // Ray (origin + direction) relative to the body center; two branches
    // keep the math f32-safe at every distance.
    var o_rel: vec3<f32>;
    var d_world: vec3<f32>;
    if planet.perspective > 0.5 {
        // Eye ray through this pixel via inv_view_proj (reversed-Z near/far
        // points), origin expressed relative to the body center. Safe: this
        // branch is only selected for near bodies, where o_rel is small.
        let near_h = uniforms.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
        let far_h = uniforms.inv_view_proj * vec4<f32>(in.ndc, 0.0, 1.0);
        let near_p = near_h.xyz / near_h.w;
        let far_p = far_h.xyz / far_h.w;
        o_rel = near_p - planet.pos;
        d_world = normalize(far_p - near_p);
    } else {
        // Parallel rays along eye->body; the perpendicular origin offset
        // comes straight from the quad corner (km = offset * rmax), so the
        // huge eye-relative vector is never formed - the precision win. The
        // offset basis must map the quad's screen axes to the SAME world
        // directions the real projection does, or the disc renders rotated/
        // mirrored (terminator on the wrong side, snapping at the trace
        // switch): take the camera's world right from view_proj (row 0 = a
        // positive multiple of the view right), re-orthogonalize against
        // this body's ray direction; right x dir gives screen up. Never
        // nearly parallel: a drawn body is within the FOV of the camera
        // forward axis, and right is perpendicular to that axis.
        let dir = normalize(planet.pos - uniforms.camera_pos);
        let cam_right = vec3<f32>(
            uniforms.view_proj[0].x,
            uniforms.view_proj[1].x,
            uniforms.view_proj[2].x,
        );
        let right = normalize(cam_right - dir * dot(cam_right, dir));
        let up = cross(right, dir);
        o_rel = (in.offset.x * rmax) * right + (in.offset.y * rmax) * up;
        d_world = dir;
    }

    // Intersect the ellipsoid by scaling into unit-sphere space; every term
    // stays O(1) (or O(distance/radius)^2 in the perspective branch).
    let rot_t = transpose(planet.rot);
    let o1 = (rot_t * o_rel) / radii;
    let d1 = (rot_t * d_world) / radii;
    let a = dot(d1, d1);
    let b = dot(o1, d1);
    let c = dot(o1, o1) - 1.0;
    let disc = b * b - a * c;
    if disc < 0.0 {
        discard;
    }
    // d points into the body, so the smaller root is the front surface.
    let t = (-b - sqrt(disc)) / a;
    let p1 = o1 + t * d1; // point on the unit sphere

    // Body-frame geodetic normal (ellipsoid gradient) + equirect UV. The UV
    // latitude comes from the NORMAL's y: for a spheroid n_body.y =
    // sin(geodetic lat), which is what the equirect maps (and the CPU-side
    // geodetic surface_position) are addressed by. Longitude stays
    // position-derived - a normal-derived longitude would warp on triaxial
    // Luna (rx != rz).
    let n_body = normalize(p1 / radii);
    let uv = vec2<f32>(
        atan2(p1.x, p1.z) / (2.0 * PI) + 0.5,
        acos(clamp(n_body.y, -1.0, 1.0)) / PI,
    );

    let albedo = textureSampleLevel(planet_texture, planet_sampler, uv, 0.0).rgb;
    // Geometric (geodetic) world normal. Terminator, night-light fade, and
    // dither anchoring MUST use it - never the bump-mapped normal.
    let n_geo = normalize(planet.rot * n_body);
    // Surface point in the render frame, for lighting + perspective depth.
    let surf = planet.pos + planet.rot * (p1 * radii);
    let sol = normalize(uniforms.sol_pos - surf);
    let cos_sol = dot(n_geo, sol);

    // Shading normal: geometric, optionally perturbed by the relief map. The
    // analytic equirect tangent frame (east = increasing u, north = image
    // up) is built in the BODY frame, then rotated out with the normal.
    var n = n_geo;
    if (planet.flags & BODY_FLAG_NORMAL_MAP) != 0u {
        let normal_sample = textureSampleLevel(planet_normal, planet_sampler, uv, 0.0).xyz * 2.0 - 1.0;
        let east = normalize(vec3<f32>(n_body.z, 0.0, -n_body.x));
        let north = cross(n_body, east);
        let n_local = normalize(
            east * normal_sample.x * NORMAL_STRENGTH
                + north * normal_sample.y * NORMAL_STRENGTH
                + n_body * normal_sample.z,
        );
        n = normalize(planet.rot * n_local);
    }
    let n_dot_l = max(dot(n, sol), 0.0);

    // Cook-Torrance GGX glint from the water mask, with wave shimmer. Gated:
    // a maskless body must skip this entirely (a zero mask would still leave
    // the land-level sheen).
    var specular = 0.0;
    if (planet.flags & BODY_FLAG_SPECULAR) != 0u {
        let specular_mask = textureSampleLevel(planet_specular, planet_sampler, uv, 0.0).r;
        let v = normalize(uniforms.camera_pos - surf);
        let h = normalize(v + sol);
        let n_dot_v = max(dot(n, v), 1e-4);
        let n_dot_h = max(dot(n, h), 0.0);
        let v_dot_h = max(dot(v, h), 0.0);

        let roughness = mix(LAND_ROUGHNESS, OCEAN_ROUGHNESS, specular_mask);
        let f0 = mix(LAND_F0, OCEAN_F0, specular_mask);

        let ggx_a = roughness * roughness;
        let ggx_a2 = ggx_a * ggx_a;
        let d_denom = n_dot_h * n_dot_h * (ggx_a2 - 1.0) + 1.0;
        let ggx_d = ggx_a2 / (PI * d_denom * d_denom);

        let ggx_k = ggx_a / 2.0;
        let ggx_g = (n_dot_v / (n_dot_v * (1.0 - ggx_k) + ggx_k))
            * (n_dot_l / (n_dot_l * (1.0 - ggx_k) + ggx_k));

        let ggx_f = f0 + (1.0 - f0) * pow(1.0 - v_dot_h, 5.0);

        specular = ggx_d * ggx_g * ggx_f / max(4.0 * n_dot_v * n_dot_l, 1e-4) * n_dot_l;

        // Modulate the glint around its mean (water only) so the average
        // brightness stays put.
        let wave = wave_noise(uv);
        let shimmer = 1.0 + WAVE_STRENGTH * (2.0 * wave - 1.0);
        specular *= mix(1.0, shimmer, specular_mask);
    }

    // Multiply the analytic eclipse visibility over every packed same-system
    // occluder (w = caster radius, 0 = unused); a body with none keeps
    // vis = 1.
    var vis = 1.0;
    for (var i = 0u; i < MAX_OCCLUDERS; i += 1u) {
        let occ = planet.occluders[i];
        if occ.w > 0.0 {
            vis *= sol_visibility(surf, sol, occ.xyz, occ.w, planet.sol_angular_radius);
        }
    }

    var color: vec3<f32>;
    if (planet.flags & BODY_FLAG_ATMO_LIT) != 0u {
        // Atmosphere-lit (Terra): sunlight is filtered by the atmosphere -
        // near the terminator the long grazing path scatters away blue and
        // the light turns orange. Eclipse vis multiplies the transmittance
        // so diffuse and specular dim together; DAY_AMBIENT stays untouched,
        // so the umbra is dark but not black (a real eclipse shadow is
        // skylight-lit). No ECLIPSE_GLOW here: the glow models refraction
        // through the OCCLUDER's atmosphere, and this body's occluders
        // (Luna) have none.
        let sol_light = sol_transmittance(PLANET_RADIUS_KM + 0.1, cos_sol) * vis;
        let day_lit = albedo
            * (vec3<f32>(DAY_AMBIENT)
                + (1.0 - DAY_AMBIENT) * n_dot_l * sol_light)
            + specular * sol_light;

        // Night side: the day map darkened by Sol geometry - no night
        // texture as color. The GEOMETRIC normal feeds the terminator so
        // bump detail doesn't speckle the day/night edge.
        let daylight = smoothstep(-0.12, 0.18, cos_sol);
        let night_factor = mix(NIGHT_DARKNESS, 1.0, daylight);
        color = day_lit * night_factor;
    } else {
        // Plain Lambert (no atmosphere - hard terminator), with the coppery
        // umbral glow only where the disc would otherwise be sunlit.
        color = albedo * (PLANET_AMBIENT + n_dot_l * vis);
        color += ECLIPSE_GLOW * (1.0 - vis) * n_dot_l * albedo;
    }

    // City lights from the night map's luminance, dissolved across the
    // terminator by a hard per-pixel dither: each pixel switches off when
    // fade crosses its own noise value, so cities erode as a coherent wipe
    // and survivors keep full brightness. The dither is anchored to the
    // BODY-frame normal so it stays fixed to the surface under rotation.
    if (planet.flags & BODY_FLAG_NIGHT) != 0u {
        let night = textureSampleLevel(planet_night, planet_sampler, uv, 0.0).rgb;
        let night_brightness = dot(night, vec3<f32>(0.2126, 0.7152, 0.0722));
        let lit = smoothstep(
            EMISSIVE_THRESHOLD,
            EMISSIVE_THRESHOLD + EMISSIVE_SOFTNESS,
            night_brightness,
        );
        let fade = smoothstep(EMISSIVE_FADE_START, EMISSIVE_FADE_END, cos_sol);
        let dither = value_noise_3d(n_body * DITHER_SCALE);
        let keep = step(fade, dither);
        color += lit * keep * EMISSIVE_COLOR * EMISSIVE_STRENGTH;
    }

    var out: PlanetFragment;
    out.color = vec4<f32>(color, 1.0);
    // Perspective: the true hit-point depth (reversed-Z), so a near limb
    // occludes correctly. Orthographic: the center's baseline depth (the
    // distant disc is sub-pixel thin).
    if planet.perspective > 0.5 {
        let clip = uniforms.view_proj * vec4<f32>(surf, 1.0);
        out.depth = clip.z / clip.w;
    } else {
        out.depth = planet.depth;
    }
    return out;
}
