struct Uniforms {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

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
    // Placeholder shading until the Earth texture lands: a 15° lat/lon grid
    // over a blue base, with simple directional lighting.
    let grid = vec2<f32>(fract(in.uv.x * 24.0), fract(in.uv.y * 12.0));

    var base = vec3<f32>(0.18, 0.35, 0.65);
    if grid.x < 0.04 || grid.y < 0.04 {
        base = vec3<f32>(0.9, 0.9, 0.9);
    }

    let n = normalize(in.normal);
    let light = normalize(vec3<f32>(0.5, 0.8, 1.0));
    let shade = 0.25 + 0.75 * max(dot(n, light), 0.0);

    return vec4<f32>(base * shade, 1.0);
}
