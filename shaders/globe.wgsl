struct Uniforms {
    view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    sun_dir: vec3<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var earth_texture: texture_2d<f32>;
@group(0) @binding(2) var earth_sampler: sampler;

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

const NIGHT_AMBIENT: f32 = 0.04;
const ATMOSPHERE_COLOR: vec3<f32> = vec3<f32>(0.3, 0.55, 1.0);

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let base = textureSample(earth_texture, earth_sampler, in.uv).rgb;

    let n = normalize(in.normal);
    let sun = normalize(uniforms.sun_dir);
    // The mesh is a unit sphere, so the normal is also the world position.
    let view_dir = normalize(uniforms.camera_pos - in.normal);

    // Sun lighting, with the terminator softened a touch so the day/night
    // edge doesn't alias.
    let cos_sun = dot(n, sun);
    let daylight = smoothstep(-0.08, 0.2, cos_sun) * max(cos_sun, 0.0)
        + smoothstep(-0.15, 0.05, cos_sun) * 0.08;
    let surface = base * (NIGHT_AMBIENT + (1.0 - NIGHT_AMBIENT) * daylight);

    // Atmosphere: a fresnel rim that fades toward the night side.
    let fresnel = pow(1.0 - max(dot(n, view_dir), 0.0), 3.0);
    let glow = fresnel
        * ATMOSPHERE_COLOR
        * (0.1 + 0.9 * smoothstep(-0.2, 0.3, cos_sun));

    return vec4<f32>(surface + glow, 1.0);
}
