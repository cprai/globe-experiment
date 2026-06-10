struct Uniforms {
    view_proj: mat4x4<f32>,
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

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let base = textureSample(earth_texture, earth_sampler, in.uv).rgb;

    // Mild directional lighting to keep the curvature readable; proper sun
    // lighting comes with the polish milestone.
    let n = normalize(in.normal);
    let light = normalize(vec3<f32>(0.5, 0.8, 1.0));
    let shade = 0.35 + 0.65 * max(dot(n, light), 0.0);

    return vec4<f32>(base * shade, 1.0);
}
