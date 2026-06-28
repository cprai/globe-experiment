struct Uniforms {
    view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    sun_dir: vec3<f32>,
    // Inverse of the star map's orientation: rotates a camera-relative world
    // direction into the star texture's frame for the equirectangular lookup.
    // Ephemeris-driven (sidereal-rate) and includes the static
    // galactic->equatorial offset, since the texture is drawn in galactic
    // coordinates.
    star_rot_inv: mat3x3<f32>,
    // Marker params shared by every satellite marker:
    // x,y = viewport size in pixels; z = marker radius in pixels; w = unused.
    // Per-marker world position + visibility are per-instance (see vs_marker).
    marker: vec4<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var day_texture: texture_2d<f32>;
@group(0) @binding(2) var earth_sampler: sampler;
@group(0) @binding(3) var night_texture: texture_2d<f32>;
@group(0) @binding(4) var normal_texture: texture_2d<f32>;
@group(0) @binding(5) var specular_texture: texture_2d<f32>;
@group(0) @binding(6) var transmittance_lut: texture_2d<f32>;
@group(0) @binding(7) var lut_sampler: sampler;
@group(0) @binding(8) var inscatter_rayleigh_lut: texture_2d<f32>;
@group(0) @binding(9) var inscatter_mie_lut: texture_2d<f32>;
@group(0) @binding(10) var stars_texture: texture_2d<f32>;

// World space is kilometers, planet center at the origin. `position` is the
// WGS84 ellipsoid surface point; `normal` is the outward geodetic unit
// normal (which the atmosphere/star passes also reuse as a sphere direction).
struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) normal: vec3<f32>,
    // World-space surface position (km), for the view vector.
    @location(2) world_pos: vec3<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.position = uniforms.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    // The ellipsoid normal is supplied per vertex (it is no longer just the
    // normalized position).
    out.normal = in.normal;
    out.world_pos = in.position;
    return out;
}

const DAY_AMBIENT: f32 = 0.04;
// How strongly the normal map perturbs the geometric normal.
// 1.0 is the map's face value; higher exaggerates the terrain relief
// (deliberately past photorealism).
const NORMAL_STRENGTH: f32 = 4.5;
// Roughness for rough land and smooth ocean; the specular map blends
// between them. The ocean value sets how wide the GGX sun glint spreads:
// 0.25 reads glassy-sharp, 0.45 approximates a wave-roughened sea.
const LAND_ROUGHNESS: f32 = 0.9;
const OCEAN_ROUGHNESS: f32 = 0.45;
// Dielectric reflectance at normal incidence.
const LAND_F0: f32 = 0.015;
const OCEAN_F0: f32 = 0.15;

const PI: f32 = 3.14159265;

// Wave texture on the ocean specular: scale is in noise cells across the
// equirectangular map, strength is the +/- fraction of the glint
// modulated. Keep the strength low - it should read as surface texture,
// not sparkle.
const WAVE_SCALE: f32 = 2200.0;
const WAVE_STRENGTH: f32 = 0.04;

// --- Emissive city lights (procedural, from the night map's brightness) ---
// A pixel is a "city" when its night-map luminance clears the threshold.
const EMISSIVE_THRESHOLD: f32 = 0.05;
const EMISSIVE_SOFTNESS: f32 = 0.1;
// Bright yellow glow; STRENGTH > 1 drives the core toward clip (LDR).
const EMISSIVE_COLOR: vec3<f32> = vec3<f32>(1.0, 0.85, 0.3);
const EMISSIVE_STRENGTH: f32 = 1.5;
// Dither-dissolve: begins at this sun cosine (deeper night = more
// negative) and completes at EMISSIVE_FADE_END.
const EMISSIVE_FADE_START: f32 = -0.15;
// Sun cosine at which the dissolve completes. Positive values let the
// lights bleed past the terminator (cos_sun = 0) onto the daylit side, so
// some daylit areas stay lit; 0 fully extinguishes them at the terminator.
const EMISSIVE_FADE_END: f32 = 0.15;
// Noise grain (cells across the unit normal sphere). Fixed - no terminator
// ramp, for a temporally coherent dissolve under sun motion.
const DITHER_SCALE: f32 = 400.0;
// Day-map multiplier for the unlit hemisphere: < 1 darkens (0 = black
// night), > 1 brightens. Intentionally 1.2 - the night side reads a
// touch brighter than full daylight.
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

// Integer-lattice bit-mixing hash. Precision-safe at large coordinates,
// unlike fract(sin(...)) - important here because n_geo * DITHER_SCALE
// pushes the lattice indices into the hundreds, where f32 sin() loses
// precision and the noise develops visible banding. p arrives as an
// integer-valued vec3 (the floored cell corner).
fn hash3(p: vec3<f32>) -> f32 {
    var n = (u32(i32(p.x)) * 1597334677u)
        ^ (u32(i32(p.y)) * 3812015801u)
        ^ (u32(i32(p.z)) * 2369874511u);
    n = (n ^ (n >> 15u)) * 2246822519u;
    n = (n ^ (n >> 13u)) * 3266489917u;
    n = n ^ (n >> 16u);
    return f32(n) / 4294967295.0;
}

// Trilinearly-interpolated 3D value noise. Sampled at the unit geodetic
// normal, so it has no seam and no pole pinch.
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

// --- Atmosphere, after Hillaire 2020. ---
// The medium definition and all scattering integrals live in the
// `mod atmosphere` in build.rs, which bakes them into LUTs at build
// time. The geometric constants here must stay in sync with the Rust
// twins.
// Lengths are kilometers.
const PLANET_RADIUS_KM: f32 = 6360.0;
const ATMOSPHERE_TOP_KM: f32 = 6460.0;
const MIE_G: f32 = 0.8;

const SUN_INTENSITY: f32 = 12.0;

// Transmittance from a point at radius `r` km toward the sun at zenith
// cosine `mu`, via the precomputed LUT (Bruneton parameterization).
fn sun_transmittance(r: f32, mu: f32) -> vec3<f32> {
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

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let albedo = textureSample(day_texture, earth_sampler, in.uv).rgb;
    let night = textureSample(night_texture, earth_sampler, in.uv).rgb;
    let normal_sample = textureSample(normal_texture, earth_sampler, in.uv).xyz * 2.0 - 1.0;
    let specular_mask = textureSample(specular_texture, earth_sampler, in.uv).r;

    // Geometric (geodetic) surface normal. Unit length and with the same
    // lat/lon direction a sphere would have, so the analytic tangent frame
    // and the surface-anchored city-light noise below are unaffected by the
    // ellipsoid shape.
    let n_geo = normalize(in.normal);

    // Analytic tangent frame of the equirectangular mapping: east along
    // increasing u, north along the image's up direction.
    let east = normalize(vec3<f32>(n_geo.z, 0.0, -n_geo.x));
    let north = cross(n_geo, east);
    let n = normalize(
        east * normal_sample.x * NORMAL_STRENGTH
            + north * normal_sample.y * NORMAL_STRENGTH
            + n_geo * normal_sample.z,
    );

    let sun = normalize(uniforms.sun_dir);
    let v = normalize(uniforms.camera_pos - in.world_pos);
    let h = normalize(v + sun);

    let n_dot_l = max(dot(n, sun), 0.0);
    let n_dot_v = max(dot(n, v), 1e-4);
    let n_dot_h = max(dot(n, h), 0.0);
    let v_dot_h = max(dot(v, h), 0.0);

    // Cook-Torrance GGX specular. The specular map marks water: smooth and
    // more reflective, versus rough land.
    let roughness = mix(LAND_ROUGHNESS, OCEAN_ROUGHNESS, specular_mask);
    let f0 = mix(LAND_F0, OCEAN_F0, specular_mask);

    let a = roughness * roughness;
    let a2 = a * a;
    let d_denom = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    let d = a2 / (PI * d_denom * d_denom);

    let k = a / 2.0;
    let g = (n_dot_v / (n_dot_v * (1.0 - k) + k))
        * (n_dot_l / (n_dot_l * (1.0 - k) + k));

    let f = f0 + (1.0 - f0) * pow(1.0 - v_dot_h, 5.0);

    var specular = d * g * f / max(4.0 * n_dot_v * n_dot_l, 1e-4) * n_dot_l;

    // Wave texture, water only: modulate the glint around its mean so
    // the average brightness stays put.
    let wave = wave_noise(in.uv);
    let shimmer = 1.0 + WAVE_STRENGTH * (2.0 * wave - 1.0);
    specular *= mix(1.0, shimmer, specular_mask);

    // Sunlight reaching the surface is filtered by the atmosphere: near
    // the terminator the blue is scattered away on the long grazing path
    // and the remaining light turns orange.
    let cos_sun = dot(n_geo, sun);
    let sun_light = sun_transmittance(PLANET_RADIUS_KM + 0.1, cos_sun);

    let day_lit = albedo
        * (vec3<f32>(DAY_AMBIENT)
            + (1.0 - DAY_AMBIENT) * n_dot_l * sun_light)
        + specular * sun_light;

    // Night side: the day map darkened by sun geometry - no night
    // texture as color. The geometric normal feeds the terminator so
    // bump detail doesn't speckle the day/night edge.
    let daylight = smoothstep(-0.12, 0.18, cos_sun);
    let night_factor = mix(NIGHT_DARKNESS, 1.0, daylight);
    var surface = day_lit * night_factor;

    // City mask: a single uniform luminance threshold on the night map.
    let night_brightness = dot(night, vec3<f32>(0.2126, 0.7152, 0.0722));
    let lit = smoothstep(
        EMISSIVE_THRESHOLD,
        EMISSIVE_THRESHOLD + EMISSIVE_SOFTNESS,
        night_brightness,
    );

    // Dither-dissolve across the terminator. fade goes 0 deep on the
    // night side (EMISSIVE_FADE_START) to 1 at EMISSIVE_FADE_END; when
    // that end is positive the dissolve finishes on the daylit side, so
    // lights linger a little past the terminator before fully clearing.
    let fade = smoothstep(EMISSIVE_FADE_START, EMISSIVE_FADE_END, cos_sun);

    // Fixed-grain noise anchored to the 3D surface position: no crawl on
    // zoom/rotate, and a stable per-pixel dissolve order under sun
    // motion. step() is a hard per-pixel dither - each pixel switches off
    // when fade crosses its own noise value, so cities erode as a
    // coherent wipe and survivors stay at full (uniform) brightness.
    let dither = value_noise_3d(n_geo * DITHER_SCALE);
    let keep = step(fade, dither);

    surface += lit * keep * EMISSIVE_COLOR * EMISSIVE_STRENGTH;

    return vec4<f32>(surface, 1.0);
}

// Atmospheric scattering, after Hillaire 2020: the same sphere mesh,
// inflated to the top-of-atmosphere radius and rendered far-side-only
// with additive blending after the globe.
//
// Because the scene is a sphere viewed from outside, the inscatter
// integral along any view ray is precomputed: a ray is identified by its
// impact parameter and the sun cosine at its reference point (ground hit,
// or closest approach for limb rays). Per fragment this costs two LUT
// samples; the phase functions are constant per ray and applied here.
// Long grazing paths near the terminator lose their blue to scattering
// and the inscattered light turns orange.

struct AtmosphereOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
};

@vertex
fn vs_atmosphere(in: VertexInput) -> AtmosphereOutput {
    var out: AtmosphereOutput;
    // The scattering model is spherical: build the shell from the unit
    // normal (not the ellipsoid position) so it is a true sphere at the
    // top-of-atmosphere radius, in km.
    let world = in.normal * ATMOSPHERE_TOP_KM;
    out.position = uniforms.view_proj * vec4<f32>(world, 1.0);
    out.world_pos = world;
    return out;
}

@fragment
fn fs_atmosphere(in: AtmosphereOutput) -> @location(0) vec4<f32> {
    // World space is already km, planet center at the origin.
    let origin = uniforms.camera_pos;
    let dir = normalize(in.world_pos - origin);
    let sun = normalize(uniforms.sun_dir);

    let shell = ray_sphere(origin, dir, ATMOSPHERE_TOP_KM);
    if shell.y <= 0.0 {
        return vec4<f32>(0.0);
    }

    // Impact parameter: the ray's closest approach to the planet center.
    let b = length(origin - dot(origin, dir) * dir);

    // Reference point and the LUT's split row mapping: lower half is
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

    let mu_ref = dot(normalize(reference), sun);
    let uv = vec2<f32>(mu_ref * 0.5 + 0.5, v);

    let sum_r = textureSampleLevel(inscatter_rayleigh_lut, lut_sampler, uv, 0.0).rgb;
    let sum_m = textureSampleLevel(inscatter_mie_lut, lut_sampler, uv, 0.0).rgb;

    // Phase functions are constant along the ray.
    let mu = dot(dir, sun);
    let phase_r = 3.0 / (16.0 * PI) * (1.0 + mu * mu);
    // Cornette-Shanks phase for Mie.
    let g2 = MIE_G * MIE_G;
    let phase_m = 3.0 / (8.0 * PI) * ((1.0 - g2) * (1.0 + mu * mu))
        / ((2.0 + g2) * pow(1.0 + g2 - 2.0 * MIE_G * mu, 1.5));

    let luminance = sum_r * phase_r + sum_m * phase_m;

    // Soft exposure roll-off keeps the bright limb from clipping.
    let color = 1.0 - exp(-luminance * SUN_INTENSITY);
    return vec4<f32>(color, 1.0);
}

// Star backdrop: the same sphere mesh inflated to enclose the camera at
// any zoom and rendered inside-out, before everything else. It reuses the
// mesh UVs, so the equirectangular star map shares the earth texture's
// orientation (celestial poles over the geographic poles).

// Must enclose the camera (max ~70000 km from center) but stay inside the
// projection's far plane (~318550 km) from any camera position. ~35 mean
// Earth radii, in km.
const STARS_RADIUS_KM: f32 = 222985.0;
const STARS_BRIGHTNESS: f32 = 0.8;

// The sun disc, drawn into the backdrop along the sun direction. The
// real sun subtends ~0.0046 rad (0.53 deg); this one is drawn a little
// larger because it reads better. The glow is the standard LDR cheat
// for brightness: a clipped-white core inside a wide soft falloff.
const SUN_ANGULAR_RADIUS: f32 = 0.012;
const SUN_GLOW_RADIUS: f32 = 0.12;
const SUN_GLOW_STRENGTH: f32 = 0.5;
const SUN_COLOR: vec3<f32> = vec3<f32>(1.0, 0.96, 0.9);

struct StarsOutput {
    @builtin(position) position: vec4<f32>,
    // Camera-relative view direction, rotated into the star map's base
    // frame. The backdrop is at infinity, so everything on it is a
    // function of view direction from the eye - anchoring it to the celestial
    // sphere's surface instead would parallax against the sun.
    @location(0) dir: vec3<f32>,
    // The same view direction in the world frame, for the sun.
    @location(1) view: vec3<f32>,
};

@vertex
fn vs_stars(in: VertexInput) -> StarsOutput {
    var out: StarsOutput;
    // Inflate the unit normal into a km-scale sphere enclosing the camera.
    let world = in.normal * STARS_RADIUS_KM;
    out.position = uniforms.view_proj * vec4<f32>(world, 1.0);

    // Linear in the vertex position, so interpolation is exact; both
    // outputs are normalized per fragment.
    let relative = world - uniforms.camera_pos;
    out.dir = uniforms.star_rot_inv * relative;
    out.view = relative;
    return out;
}

@fragment
fn fs_stars(in: StarsOutput) -> @location(0) vec4<f32> {
    // Equirectangular lookup from the rotated direction. Computed per
    // fragment (not per vertex) so the dateline seam doesn't smear.
    let d = normalize(in.dir);
    let lon = atan2(d.x, d.z);
    let uv = vec2<f32>(
        lon / (2.0 * PI) + 0.5,
        acos(clamp(d.y, -1.0, 1.0)) / PI,
    );

    let stars = textureSampleLevel(stars_texture, earth_sampler, uv, 0.0).rgb;

    // The sun, along the same camera-relative view direction as the
    // stars, so the two stay locked under rotation and zoom. The globe
    // draws after the backdrop and occludes it; the atmosphere pass
    // then glows over it near the limb.
    let view = normalize(in.view);
    let sun = normalize(uniforms.sun_dir);
    let angle = acos(clamp(dot(view, sun), -1.0, 1.0));

    // Anti-aliased disc core plus a soft glow falloff.
    let disc = 1.0
        - smoothstep(SUN_ANGULAR_RADIUS * 0.85, SUN_ANGULAR_RADIUS, angle);
    let glow = SUN_GLOW_STRENGTH
        * pow(max(1.0 - angle / SUN_GLOW_RADIUS, 0.0), 3.0);

    let color = stars * STARS_BRIGHTNESS + SUN_COLOR * (disc + glow);
    return vec4<f32>(color, 1.0);
}

// Satellite markers: a flat, constant-pixel-size circle drawn at each tracked
// object's projected screen position, after everything else (so they read as
// overlays) with alpha blending. Drawn as one instanced call - the quad is
// generated from the vertex index, and the per-marker world position +
// visibility arrive as instance attributes (one per satellite). Visibility
// (occlusion behind the globe) is decided on the CPU; when hidden the quad is
// pushed off-screen so it produces no fragments.

const MARKER_FILL: vec3<f32> = vec3<f32>(1.0, 0.25, 0.2);
const MARKER_RING: vec3<f32> = vec3<f32>(1.0, 1.0, 1.0);

struct MarkerInstance {
    // World-frame marker position (km).
    @location(0) position: vec3<f32>,
    // Visible flag: >= 0.5 = drawn, < 0.5 = hidden (occluded by the globe).
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

    // Hidden (occluded by the globe): emit an off-screen, clipped vertex.
    let clip = uniforms.view_proj * vec4<f32>(inst.position, 1.0);
    if inst.visible < 0.5 || clip.w <= 0.0 {
        out.position = vec4<f32>(0.0, 0.0, 2.0, 1.0);
        return out;
    }

    // Offset the projected center by a constant pixel radius. One pixel is
    // 2/viewport NDC units; multiplying by clip.w pre-compensates for the
    // perspective divide, keeping the circle round and size-stable at any
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
