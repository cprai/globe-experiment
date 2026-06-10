struct Uniforms {
    view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    sun_dir: vec3<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var day_texture: texture_2d<f32>;
@group(0) @binding(2) var earth_sampler: sampler;
@group(0) @binding(3) var night_texture: texture_2d<f32>;
@group(0) @binding(4) var normal_texture: texture_2d<f32>;
@group(0) @binding(5) var specular_texture: texture_2d<f32>;

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
const ATMOSPHERE_COLOR: vec3<f32> = vec3<f32>(0.3, 0.55, 1.0);
// The haze shell extends this far beyond the planet's unit radius.
const HAZE_RADIUS: f32 = 1.04;
// Peak haze brightness at the limb.
const HAZE_STRENGTH: f32 = 0.45;
// Higher exponents hug the haze tighter against the limb.
const HAZE_FALLOFF: f32 = 2.5;
// How strongly the normal map perturbs the geometric normal.
// 1.0 is the map's face value; higher exaggerates the terrain relief
// (deliberately past photorealism).
const NORMAL_STRENGTH: f32 = 4.5;
// Roughness for rough land and smooth ocean; the specular map blends
// between them.
const LAND_ROUGHNESS: f32 = 0.9;
const OCEAN_ROUGHNESS: f32 = 0.25;
// Dielectric reflectance at normal incidence.
const LAND_F0: f32 = 0.015;
const OCEAN_F0: f32 = 0.05;

const PI: f32 = 3.14159265;

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

    let specular =
        d * g * f / max(4.0 * n_dot_v * n_dot_l, 1e-4) * n_dot_l;

    let day_lit = albedo * (DAY_AMBIENT + (1.0 - DAY_AMBIENT) * n_dot_l)
        + vec3<f32>(specular);

    // Blend to the emissive night side (city lights) across a softened
    // terminator. Uses the geometric normal so bump detail doesn't
    // speckle the day/night edge.
    let cos_sun = dot(n_geo, sun);
    let daylight = smoothstep(-0.12, 0.18, cos_sun);
    let surface = mix(night, day_lit, daylight);

    // Atmosphere: a fresnel rim that fades toward the night side.
    let rim = pow(1.0 - max(dot(n_geo, v), 0.0), 3.0);
    let glow = rim
        * ATMOSPHERE_COLOR
        * (0.1 + 0.9 * smoothstep(-0.2, 0.3, cos_sun));

    return vec4<f32>(surface + glow, 1.0);
}

// Atmospheric haze: the same sphere mesh, inflated to HAZE_RADIUS and
// rendered far-side-only with additive blending, before the globe draws
// over its interior. What survives is a soft ring of scattered light
// around the limb.

struct HazeOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
};

@vertex
fn vs_haze(in: VertexInput) -> HazeOutput {
    var out: HazeOutput;
    let world = in.position * HAZE_RADIUS;
    out.position = uniforms.view_proj * vec4<f32>(world, 1.0);
    out.world_pos = world;
    return out;
}

@fragment
fn fs_haze(in: HazeOutput) -> @location(0) vec4<f32> {
    // Closest approach of the view ray to the globe center: 1.0 when the
    // ray grazes the surface, HAZE_RADIUS at the shell's outer edge.
    let v = normalize(uniforms.camera_pos - in.world_pos);
    let closest = length(in.world_pos - dot(in.world_pos, v) * v);

    let t = clamp(
        (HAZE_RADIUS - closest) / (HAZE_RADIUS - 1.0),
        0.0,
        1.0,
    );
    let falloff = pow(t, HAZE_FALLOFF);

    // Haze is scattered sunlight: fade it out around the night side.
    let day = smoothstep(
        -0.2,
        0.3,
        dot(normalize(in.world_pos), normalize(uniforms.sun_dir)),
    );

    let intensity = HAZE_STRENGTH * falloff * (0.05 + 0.95 * day);
    return vec4<f32>(ATMOSPHERE_COLOR * intensity, intensity);
}
