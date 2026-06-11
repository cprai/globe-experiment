struct Uniforms {
    view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    sun_dir: vec3<f32>,
    // Inverse of the star map's orientation (sky is rigidly attached to
    // the sun: longitude spins it about the polar axis, latitude tilts
    // it about the horizontal equinox axis).
    star_rot_inv: mat3x3<f32>,
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

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) normal: vec3<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.position = uniforms.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    // The mesh is a unit sphere at the origin, so position doubles as normal.
    out.normal = in.position;
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
const OCEAN_ROUGHNESS: f32 = 0.35;
// Dielectric reflectance at normal incidence.
const LAND_F0: f32 = 0.015;
const OCEAN_F0: f32 = 0.05;

const PI: f32 = 3.14159265;

// Wave texture on the ocean specular: scale is in noise cells across the
// equirectangular map, strength is the +/- fraction of the glint
// modulated. Keep the strength low — it should read as surface texture,
// not sparkle.
const WAVE_SCALE: f32 = 2200.0;
const WAVE_STRENGTH: f32 = 0.04;

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

// --- Atmosphere, after Hillaire 2020. ---
// The medium definition and all scattering integrals live in
// src/globe/atmosphere.rs, which bakes them into LUTs at startup. The
// geometric constants here must stay in sync with the Rust twins.
// Lengths are kilometers.
const PLANET_RADIUS_KM: f32 = 6360.0;
const ATMOSPHERE_TOP_KM: f32 = 6460.0;
const MIE_G: f32 = 0.8;

const SUN_INTENSITY: f32 = 12.0;

// World space is in planet radii; the atmosphere shell sits at the
// top-of-atmosphere radius.
const ATMOSPHERE_SHELL: f32 = ATMOSPHERE_TOP_KM / PLANET_RADIUS_KM;

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
    )
    .rgb;
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
    let normal_sample =
        textureSample(normal_texture, earth_sampler, in.uv).xyz * 2.0 - 1.0;
    let specular_mask = textureSample(specular_texture, earth_sampler, in.uv).r;

    // Geometric normal; the mesh is a unit sphere, so this is also the
    // world position.
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
    let v = normalize(uniforms.camera_pos - in.normal);
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

    var specular =
        d * g * f / max(4.0 * n_dot_v * n_dot_l, 1e-4) * n_dot_l;

    // Wave texture, water only: modulate the glint around its mean so
    // the average brightness stays put.
    let wave = wave_noise(in.uv);
    let shimmer = 1.0 + WAVE_STRENGTH * (2.0 * wave - 1.0);
    specular *= mix(1.0, shimmer, specular_mask);

    // Sunlight reaching the surface is filtered by the atmosphere: near
    // the terminator the blue is scattered away on the long grazing path
    // and the remaining light turns orange.
    let cos_sun = dot(n_geo, sun);
    let sun_light =
        sun_transmittance(PLANET_RADIUS_KM + 0.1, cos_sun);

    let day_lit = albedo
        * (vec3<f32>(DAY_AMBIENT)
            + (1.0 - DAY_AMBIENT) * n_dot_l * sun_light)
        + specular * sun_light;

    // Blend to the emissive night side (city lights) across a softened
    // terminator. Uses the geometric normal so bump detail doesn't
    // speckle the day/night edge.
    let daylight = smoothstep(-0.12, 0.18, cos_sun);
    let surface = mix(night, day_lit, daylight);

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
    let world = in.position * ATMOSPHERE_SHELL;
    out.position = uniforms.view_proj * vec4<f32>(world, 1.0);
    out.world_pos = world;
    return out;
}

@fragment
fn fs_atmosphere(in: AtmosphereOutput) -> @location(0) vec4<f32> {
    // Work in km, planet center at the origin.
    let origin = uniforms.camera_pos * PLANET_RADIUS_KM;
    let dir = normalize(in.world_pos * PLANET_RADIUS_KM - origin);
    let sun = normalize(uniforms.sun_dir);

    let shell = ray_sphere(origin, dir, ATMOSPHERE_TOP_KM);
    if shell.y <= 0.0 {
        return vec4<f32>(0.0);
    }

    // Impact parameter: the ray's closest approach to the planet center.
    let b = length(origin - dot(origin, dir) * dir);

    // Reference point and the LUT's split row mapping: lower half is
    // ground-hitting rays, upper half limb rays. Must match the bake in
    // src/globe/atmosphere.rs.
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

    let sum_r =
        textureSampleLevel(inscatter_rayleigh_lut, lut_sampler, uv, 0.0).rgb;
    let sum_m =
        textureSampleLevel(inscatter_mie_lut, lut_sampler, uv, 0.0).rgb;

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

// Must enclose the camera (max distance ~11 radii) but stay inside the
// projection's far plane (50 radii) from any camera position.
const STARS_RADIUS: f32 = 35.0;
const STARS_BRIGHTNESS: f32 = 0.8;

// The sun disc, drawn into the backdrop along the sun direction. The
// real sun subtends ~0.0046 rad (0.53°); this one is drawn a little
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
    // function of view direction from the eye — anchoring it to the sky
    // sphere's surface instead would parallax against the sun.
    @location(0) dir: vec3<f32>,
    // The same view direction in the world frame, for the sun.
    @location(1) view: vec3<f32>,
};

@vertex
fn vs_stars(in: VertexInput) -> StarsOutput {
    var out: StarsOutput;
    let world = in.position * STARS_RADIUS;
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

    let stars =
        textureSampleLevel(stars_texture, earth_sampler, uv, 0.0).rgb;

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
