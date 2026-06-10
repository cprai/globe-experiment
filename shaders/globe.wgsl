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
@group(0) @binding(6) var transmittance_lut: texture_2d<f32>;
@group(0) @binding(7) var lut_sampler: sampler;

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
// between them.
const LAND_ROUGHNESS: f32 = 0.9;
const OCEAN_ROUGHNESS: f32 = 0.25;
// Dielectric reflectance at normal incidence.
const LAND_F0: f32 = 0.015;
const OCEAN_F0: f32 = 0.05;

const PI: f32 = 3.14159265;

// --- Atmosphere medium, after Hillaire 2020. ---
// Earth parameters; lengths in km, coefficients per km. The Rust twins
// of these constants live in src/globe/atmosphere.rs and bake the
// transmittance LUT — keep them in sync.
const PLANET_RADIUS_KM: f32 = 6360.0;
const ATMOSPHERE_TOP_KM: f32 = 6460.0;
const RAYLEIGH_SCATTERING: vec3<f32> =
    vec3<f32>(0.005802, 0.013558, 0.0331);
const RAYLEIGH_SCALE_HEIGHT: f32 = 8.0;
const MIE_SCATTERING: f32 = 0.003996;
const MIE_EXTINCTION: f32 = 0.00440;
const MIE_SCALE_HEIGHT: f32 = 1.2;
const MIE_G: f32 = 0.8;
const OZONE_ABSORPTION: vec3<f32> =
    vec3<f32>(0.000650, 0.001881, 0.000085);

const SUN_INTENSITY: f32 = 12.0;
const RAYMARCH_SAMPLES: i32 = 24;

// World space is in planet radii; the atmosphere shell sits at the
// top-of-atmosphere radius.
const ATMOSPHERE_SHELL: f32 = ATMOSPHERE_TOP_KM / PLANET_RADIUS_KM;

struct Medium {
    scatter_r: vec3<f32>,
    scatter_m: f32,
    extinction: vec3<f32>,
};

fn sample_medium(h: f32) -> Medium {
    let density_r = exp(-h / RAYLEIGH_SCALE_HEIGHT);
    let density_m = exp(-h / MIE_SCALE_HEIGHT);
    // Ozone concentration is a tent function peaking at 25 km.
    let density_o = max(0.0, 1.0 - abs(h - 25.0) / 15.0);

    var medium: Medium;
    medium.scatter_r = RAYLEIGH_SCATTERING * density_r;
    medium.scatter_m = MIE_SCATTERING * density_m;
    medium.extinction = medium.scatter_r
        + vec3<f32>(MIE_EXTINCTION * density_m)
        + OZONE_ABSORPTION * density_o;
    return medium;
}

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

    let specular =
        d * g * f / max(4.0 * n_dot_v * n_dot_l, 1e-4) * n_dot_l;

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
// with additive blending after the globe. Each fragment raymarches
// single scattering along its view ray, using the transmittance LUT for
// sunlight attenuation — long grazing paths near the terminator lose
// their blue to scattering and the inscattered light turns orange.

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

    // March from where the ray enters the atmosphere to where it exits
    // or hits the ground.
    let shell = ray_sphere(origin, dir, ATMOSPHERE_TOP_KM);
    if shell.y <= 0.0 {
        return vec4<f32>(0.0);
    }
    let t_start = max(shell.x, 0.0);
    var t_end = shell.y;

    let ground = ray_sphere(origin, dir, PLANET_RADIUS_KM);
    if ground.x > 0.0 {
        t_end = min(t_end, ground.x);
    }

    let mu = dot(dir, sun);
    let phase_r = 3.0 / (16.0 * PI) * (1.0 + mu * mu);
    // Cornette-Shanks phase for Mie.
    let g2 = MIE_G * MIE_G;
    let phase_m = 3.0 / (8.0 * PI) * ((1.0 - g2) * (1.0 + mu * mu))
        / ((2.0 + g2) * pow(1.0 + g2 - 2.0 * MIE_G * mu, 1.5));

    let dt = (t_end - t_start) / f32(RAYMARCH_SAMPLES);
    var transmittance = vec3<f32>(1.0);
    var luminance = vec3<f32>(0.0);

    for (var s = 0; s < RAYMARCH_SAMPLES; s++) {
        let t = t_start + (f32(s) + 0.5) * dt;
        let p = origin + dir * t;
        let r = length(p);
        let h = max(r - PLANET_RADIUS_KM, 0.0);

        let medium = sample_medium(h);
        let mu_sun = dot(p / r, sun);
        let t_sun = sun_transmittance(r, mu_sun);

        let inscatter = (medium.scatter_r * phase_r
            + vec3<f32>(medium.scatter_m * phase_m))
            * t_sun;

        // Analytic integration of the inscatter across the step
        // (Hillaire 2020, eq. 11): exact for constant medium per step.
        let step_trans = exp(-medium.extinction * dt);
        luminance += transmittance
            * (inscatter - inscatter * step_trans)
            / max(medium.extinction, vec3<f32>(1e-6));
        transmittance *= step_trans;
    }

    // Soft exposure roll-off keeps the bright limb from clipping.
    let color = 1.0 - exp(-luminance * SUN_INTENSITY);
    return vec4<f32>(color, 1.0);
}
