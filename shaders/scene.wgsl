// All positions below are in the RENDER FRAME: relative to the camera target's
// center (the "floating origin"). The renderer subtracts that center on the CPU
// before upload, and `view_proj` is built in the same frame, so the GPU only
// ever handles small, target-local coordinates - the orbited body sits at the
// numerical origin and far planets do not lose f32 precision. There is no
// Earth-fixed origin or `sol_dir`; every lit pass derives its Sol direction
// from `sol_pos` (Sol relative to the camera target).
struct Uniforms {
    view_proj: mat4x4<f32>,
    // Camera eye in the render frame (km).
    camera_pos: vec3<f32>,
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
    // Rotation from Luna's body-fixed (selenographic) frame to world space:
    // the ephemeris + IAU lunar orientation. Applied to the lunar mesh's
    // positions and normals (a pure rotation, so normals need no transpose).
    luna_rot: mat3x3<f32>,
    // Luna center in the render frame (km).
    luna_pos: vec3<f32>,
    // Eclipse-geometry params: x = Luna mean radius (km); y = Terra mean radius
    // (km); z = Sol's angular radius (rad), which sets the penumbra
    // softness; w = unused.
    luna_params: vec4<f32>,
    // Sol position in the render frame (km). Lights every body
    // (`normalize(sol_pos - surface)`) and aims the backdrop Sol disc.
    sol_pos: vec3<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var day_texture: texture_2d<f32>;
@group(0) @binding(2) var terra_sampler: sampler;
@group(0) @binding(3) var night_texture: texture_2d<f32>;
@group(0) @binding(4) var normal_texture: texture_2d<f32>;
@group(0) @binding(5) var specular_texture: texture_2d<f32>;
@group(0) @binding(6) var transmittance_lut: texture_2d<f32>;
@group(0) @binding(7) var lut_sampler: sampler;
@group(0) @binding(8) var inscatter_rayleigh_lut: texture_2d<f32>;
@group(0) @binding(9) var inscatter_mie_lut: texture_2d<f32>;
@group(0) @binding(10) var stars_texture: texture_2d<f32>;
@group(0) @binding(11) var luna_texture: texture_2d<f32>;

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
    // The Terra mesh is built at the world origin, which IS the render origin
    // whenever the Terra surface draws (it only draws when orbiting the
    // Terra/Luna), so its vertices are already in the render frame.
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
// between them. The ocean value sets how wide the GGX Sol glint spreads:
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
// Dither-dissolve: begins at this Sol cosine (deeper night = more
// negative) and completes at EMISSIVE_FADE_END.
const EMISSIVE_FADE_START: f32 = -0.15;
// Sol cosine at which the dissolve completes. Positive values let the
// lights bleed past the terminator (cos_sol = 0) onto the daylit side, so
// some daylit areas stay lit; 0 fully extinguishes them at the terminator.
const EMISSIVE_FADE_END: f32 = 0.15;
// Noise grain (cells across the unit normal sphere). Fixed - no terminator
// ramp, for a temporally coherent dissolve under Sol motion.
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

const SOL_INTENSITY: f32 = 12.0;

// Transmittance from a point at radius `r` km toward Sol at zenith
// cosine `mu`, via the precomputed LUT (Bruneton parameterization).
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

// Fraction of disk 1 (radius r1) covered by disk 2 (radius r2) when their
// centers are `sep` apart - the standard two-circle lens-area overlap, divided
// by disk 1's area. Used for the eclipse soft shadow: disk 1 is Sol, disk 2
// the occluding body, all as angular radii. Returns 0 (no overlap) to 1 (disk 1
// fully covered).
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

// Fraction of sunlight reaching a surface point `p` (world km) that is NOT
// blocked by a spherical occluder of radius `occ_radius` centered at `occ`
// (world km), with Sol toward unit `Sol`. This is the analytic eclipse
// shadow shared by both directions: Luna shadowing Terra (solar
// eclipse) and Terra shadowing Luna (lunar eclipse). 1 = fully lit, 0 =
// total (umbral) shadow; the penumbra is soft because Sol has a finite
// angular radius (`uniforms.luna_params.z`). Both bodies are spheres at this
// scale - exact enough, since the penumbra dwarfs the triaxial/oblate detail.
fn sol_visibility(p: vec3<f32>, sol: vec3<f32>, occ: vec3<f32>, occ_radius: f32) -> f32 {
    let oc = occ - p;
    let t = dot(oc, sol);
    // The occluder must lie toward Sol to cast a shadow here.
    if t <= 0.0 {
        return 1.0;
    }
    let perp = length(oc - sol * t);
    let ang_sep = atan(perp / t);
    let ang_occ = atan(occ_radius / t);
    let sol_ang = uniforms.luna_params.z;
    return 1.0 - disk_overlap_fraction(ang_sep, sol_ang, ang_occ);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let albedo = textureSample(day_texture, terra_sampler, in.uv).rgb;
    let night = textureSample(night_texture, terra_sampler, in.uv).rgb;
    let normal_sample = textureSample(normal_texture, terra_sampler, in.uv).xyz * 2.0 - 1.0;
    let specular_mask = textureSample(specular_texture, terra_sampler, in.uv).r;

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

    // The Sol direction at this surface point (render-frame positions).
    let sol = normalize(uniforms.sol_pos - in.world_pos);
    let v = normalize(uniforms.camera_pos - in.world_pos);
    let h = normalize(v + sol);

    let n_dot_l = max(dot(n, sol), 0.0);
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
    let cos_sol = dot(n_geo, sol);
    // Luna can eclipse Sol for this point (solar eclipse): darken the
    // incoming sunlight by the analytic shadow. Multiplying the transmittance
    // dims both the diffuse and specular Sol terms consistently; the small
    // DAY_AMBIENT term is left untouched, so the umbra is dark but not black
    // (as a real eclipse shadow, lit by scattered skylight, is not).
    let eclipse = sol_visibility(
        in.world_pos,
        sol,
        uniforms.luna_pos,
        uniforms.luna_params.x,
    );
    let sol_light = sol_transmittance(PLANET_RADIUS_KM + 0.1, cos_sol) * eclipse;

    let day_lit = albedo
        * (vec3<f32>(DAY_AMBIENT)
            + (1.0 - DAY_AMBIENT) * n_dot_l * sol_light)
        + specular * sol_light;

    // Night side: the day map darkened by Sol geometry - no night
    // texture as color. The geometric normal feeds the terminator so
    // bump detail doesn't speckle the day/night edge.
    let daylight = smoothstep(-0.12, 0.18, cos_sol);
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
    let fade = smoothstep(EMISSIVE_FADE_START, EMISSIVE_FADE_END, cos_sol);

    // Fixed-grain noise anchored to the 3D surface position: no crawl on
    // zoom/rotate, and a stable per-pixel dissolve order under Sol
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
// with additive blending after the body.
//
// Because the scene is a sphere viewed from outside, the inscatter
// integral along any view ray is precomputed: a ray is identified by its
// impact parameter and the Sol cosine at its reference point (ground hit,
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
    // The atmosphere shell is centered on Terra at the world origin, which
    // IS the render origin whenever it draws (only when orbiting the Terra/Luna).
    let world = in.normal * ATMOSPHERE_TOP_KM;
    out.position = uniforms.view_proj * vec4<f32>(world, 1.0);
    out.world_pos = world;
    return out;
}

@fragment
fn fs_atmosphere(in: AtmosphereOutput) -> @location(0) vec4<f32> {
    // Terra sits at the render origin whenever this draws, so render-frame
    // positions are Earth-centered here. `camera_pos` is the eye in that frame.
    let origin = uniforms.camera_pos;
    let dir = normalize(in.world_pos - origin);
    let sol = normalize(uniforms.sol_pos);

    let shell = ray_sphere(origin, dir, ATMOSPHERE_TOP_KM);
    if shell.y <= 0.0 {
        return vec4<f32>(0.0);
    }

    // Luna (a solid body) occludes Terra's atmosphere when it sits
    // between the camera and Terra. This pass deliberately does not depth-
    // test (it layers its aerial perspective over Terra's own near disc,
    // whose depth is closer than this far-side shell), so without an explicit
    // check the additive glow bleeds over the nearer Luna - visible as a faint
    // spot on the lunar disc from a Luna-orbit view. Drop the fragment where the
    // ray meets Luna in front of where it enters the atmosphere. (Luna is
    // ~384,000 km out, so from a Terra orbit it is always far beyond the
    // atmosphere and this never triggers.)
    let luna = ray_sphere(origin - uniforms.luna_pos, dir, uniforms.luna_params.x);
    if luna.y > 0.0 && luna.x > 0.0 && luna.x < shell.x {
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

// Star backdrop: the same sphere mesh inflated into a shell centered on the
// camera and rendered inside-out, before everything else. It reuses the mesh
// UVs, so the equirectangular star map shares the terra texture's orientation
// (celestial poles over the geographic poles).

// Shell radius, in km. The shell is centered on the camera (see vs_stars), so it
// always encloses the eye whatever the orbit target; this radius only has to sit
// between the near and far planes (the far plane is 500,000 km). ~35 mean Terra
// radii.
const STARS_RADIUS_KM: f32 = 222985.0;
const STARS_BRIGHTNESS: f32 = 0.8;

// The Sol disc, drawn into the backdrop along the Sol direction. The
// real Sol subtends ~0.0046 rad (0.53 deg); this one is drawn a little
// larger because it reads better. The glow is the standard LDR cheat
// for brightness: a clipped-white core inside a wide soft falloff.
const SOL_ANGULAR_RADIUS: f32 = 0.012;
const SOL_GLOW_RADIUS: f32 = 0.12;
const SOL_GLOW_STRENGTH: f32 = 0.5;
const SOL_COLOR: vec3<f32> = vec3<f32>(1.0, 0.96, 0.9);

struct StarsOutput {
    @builtin(position) position: vec4<f32>,
    // Camera-relative view direction, rotated into the star map's base
    // frame. The backdrop is at infinity, so everything on it is a
    // function of view direction from the eye - anchoring it to the celestial
    // sphere's surface instead would parallax against Sol.
    @location(0) dir: vec3<f32>,
    // The same view direction in the world frame, for Sol.
    @location(1) view: vec3<f32>,
};

@vertex
fn vs_stars(in: VertexInput) -> StarsOutput {
    var out: StarsOutput;
    // Inflate the unit normal into a km-scale shell centered on the CAMERA, not
    // the Terra origin, so it encloses the eye at any orbit target - including
    // Luna, ~384,000 km from the origin and far outside an origin-centered
    // shell (which is why half the sky and Sol vanished from a Luna view).
    // Centering on the camera also makes the camera-relative direction exactly
    // the vertex normal direction (no camera_pos term), so the star/Sol lookup
    // is a pure function of view direction - a true backdrop at infinity - and
    // the Terra-orbit framing is unchanged (the eye was always inside the old
    // shell, where the two formulations give the same per-pixel direction).
    let relative = in.normal * STARS_RADIUS_KM;
    // camera_pos is already in the render frame, so the shell is too.
    let world = uniforms.camera_pos + relative;
    out.position = uniforms.view_proj * vec4<f32>(world, 1.0);

    // Linear in the vertex position, so interpolation is exact; both
    // outputs are normalized per fragment.
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

    let stars = textureSampleLevel(stars_texture, terra_sampler, uv, 0.0).rgb;

    // Sol, along the same camera-relative view direction as the
    // stars, so the two stay locked under rotation and zoom. The body
    // draws after the backdrop and occludes it; the atmosphere pass
    // then glows over it near the limb.
    //
    // The direction is the one the ORBITED BODY sees Sol in: `sol_pos` is
    // already Sol relative to the camera target. From a distant planet the
    // Sol is in a wholly different direction than from Terra, and this is what
    // makes the drawn disc agree with that planet's terminator (`fs_planet`
    // lights from `sol_pos - world_pos`, the same direction). For Terra/Luna it
    // is the Terra->Sol direction. It is parallax-free under local orbit/zoom
    // (the render origin, hence `sol_pos`, is constant while orbiting one body).
    let view = normalize(in.view);
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

// Satellite markers: a flat, constant-pixel-size circle drawn at each tracked
// object's projected screen position, after everything else (so they read as
// overlays) with alpha blending. Drawn as one instanced call - the quad is
// generated from the vertex index, and the per-marker world position +
// visibility arrive as instance attributes (one per satellite). Visibility
// (occlusion behind the body) is decided on the CPU; when hidden the quad is
// pushed off-screen so it produces no fragments.

const MARKER_FILL: vec3<f32> = vec3<f32>(1.0, 0.25, 0.2);
const MARKER_RING: vec3<f32> = vec3<f32>(1.0, 1.0, 1.0);

struct MarkerInstance {
    // World-frame marker position (km).
    @location(0) position: vec3<f32>,
    // Visible flag: >= 0.5 = drawn, < 0.5 = hidden (occluded by the body).
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

    // Hidden (occluded by the body): emit an off-screen, clipped vertex.
    // Markers only draw when orbiting the Terra/Luna (render origin at the
    // Terra), so the satellite's Terra-frame position is already render-frame.
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

// Luna: the triaxial lunar mesh, oriented into world space by the
// ephemeris + IAU lunar rotation (`uniforms.luna_rot`) and placed at its true
// world position. Lit by the same Sol as Terra, but with no atmosphere
// (a hard terminator) and its own eclipse shadow: Terra can block Sol
// (lunar eclipse), darkening the lit disk and leaving a faint coppery glow from
// sunlight refracted through Terra's atmosphere. Drawn with the depth buffer so
// Terra correctly occludes it; Luna is always farther than Terra
// from any near-Terra camera, so this is what hides it behind the planet.

// Faint fill on the lunar night side (terrashine + scattered light), so the
// unlit limb is not pure black.
const LUNA_AMBIENT: f32 = 0.02;
// Coppery glow on the eclipsed (umbral) Luna, from sunlight refracted through
// Terra's atmosphere - the "blood-red Luna". Dim and red-biased.
const LUNA_ECLIPSE_GLOW: vec3<f32> = vec3<f32>(0.06, 0.012, 0.004);

struct LunaOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) world_pos: vec3<f32>,
};

@vertex
fn vs_luna(in: VertexInput) -> LunaOutput {
    var out: LunaOutput;
    // Body-fixed mesh -> world: rotate by the lunar orientation, then translate
    // to Luna's world center. The rotation is orthonormal, so it carries
    // the normal too.
    // luna_pos is already in the render frame, so the lunar mesh lands there.
    let world = uniforms.luna_rot * in.position + uniforms.luna_pos;
    out.position = uniforms.view_proj * vec4<f32>(world, 1.0);
    out.uv = in.uv;
    out.normal = uniforms.luna_rot * in.normal;
    out.world_pos = world;
    return out;
}

@fragment
fn fs_luna(in: LunaOutput) -> @location(0) vec4<f32> {
    let albedo = textureSample(luna_texture, terra_sampler, in.uv).rgb;
    let n = normalize(in.normal);
    // The Sol direction at this lunar surface point (render-frame positions).
    let sol = normalize(uniforms.sol_pos - in.world_pos);

    // Hard (atmosphere-free) terminator: plain Lambert, with the slightest
    // softening for edge antialiasing.
    let sunlit = max(dot(n, sol), 0.0);

    // Terra shadow on Luna (lunar eclipse): Terra can block Sol. The
    // Luna only draws when orbiting the Terra/Luna, so Terra is at the render
    // origin (vec3(0)). Soft penumbra from Sol's angular size.
    let eclipse = sol_visibility(in.world_pos, sol, vec3<f32>(0.0), uniforms.luna_params.y);

    var color = albedo * (LUNA_AMBIENT + sunlit * eclipse);
    // Coppery umbral glow, only where Luna would otherwise be sunlit.
    color += LUNA_ECLIPSE_GLOW * (1.0 - eclipse) * sunlit * albedo;

    return vec4<f32>(color, 1.0);
}

// A planet: the oblate planet mesh, oriented into world space by the ephemeris
// + IAU planet rotation and placed at its center in the render frame, lit by
// Sol with a simple Lambert term (albedo x diffuse + small ambient) - no
// atmosphere, no eclipse shadow. Each planet is drawn in its own pass with its
// own group-1 bind group (per-planet uniform + texture), which keeps the seven
// planet textures out of the shared group-0 layout (so its 9 sampled textures
// never grow toward the portable 16-per-stage limit). The shared group-0
// uniforms supply view_proj and the Sol position.

// Faint fill on the planet night side, so the unlit limb is not pure black.
const PLANET_AMBIENT: f32 = 0.02;

// Per-planet model + texture (one bound per planet draw).
struct PlanetUniform {
    // Body-fixed -> world rotation (ephemeris Earth orientation x IAU planet
    // rotation); a pure rotation, so it carries the normal too.
    rot: mat3x3<f32>,
    // Planet center in the RENDER FRAME (km) = relative to the camera target.
    // Exactly zero for the orbited planet (its center IS the render origin), so
    // its mesh is drawn in pure local coordinates - no far-planet f32 jitter.
    pos: vec3<f32>,
    // Equatorial semi-axis (+X/+Z) in km. The mesh path ignores it; the
    // billboard impostor needs the true oblate ellipsoid to trace.
    equatorial_radius_km: f32,
    // Polar semi-axis (+Y, the rotation pole) in km.
    polar_radius_km: f32,
};

@group(1) @binding(0) var<uniform> planet: PlanetUniform;
@group(1) @binding(1) var planet_texture: texture_2d<f32>;
@group(1) @binding(2) var planet_sampler: sampler;

struct PlanetOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) world_pos: vec3<f32>,
};

@vertex
fn vs_planet(in: VertexInput) -> PlanetOutput {
    var out: PlanetOutput;
    // Body-fixed mesh -> render frame: rotate, then translate to the planet's
    // render-frame center. For the orbited planet `planet.pos` is exactly zero,
    // so this is `planet.rot * in.position` - small, fully f32-precise.
    let world = planet.rot * in.position + planet.pos;
    out.position = uniforms.view_proj * vec4<f32>(world, 1.0);
    out.uv = in.uv;
    out.normal = planet.rot * in.normal;
    out.world_pos = world;
    return out;
}

@fragment
fn fs_planet(in: PlanetOutput) -> @location(0) vec4<f32> {
    let albedo = textureSample(planet_texture, planet_sampler, in.uv).rgb;
    let n = normalize(in.normal);
    // The Sol direction at this surface point; both positions are in the render
    // frame, so the difference is the true planet-surface->Sol vector.
    let sol = normalize(uniforms.sol_pos - in.world_pos);
    let sunlit = max(dot(n, sol), 0.0);

    let color = albedo * (PLANET_AMBIENT + sunlit);
    return vec4<f32>(color, 1.0);
}

// A distant planet drawn as a camera-facing billboard impostor instead of a
// full mesh. When a planet's apparent angular size falls below a threshold (the
// renderer classifies this on the CPU and routes it here), a mesh is wasteful -
// the planet is at most a few pixels. The impostor is a single camera-facing
// quad whose fragment shader ray-traces the true oblate ellipsoid, samples the
// same group-1 albedo texture, and Lambert-lights it from Sol, so the
// silhouette, rotation/libration, terminator, and texture all stay faithful.
//
// The rays are treated as PARALLEL (orthographic), which is exact here: the
// billboard is only ever used when distance >> radius (that is the selection
// criterion), and parallel rays avoid the catastrophic f32 cancellation a true
// perspective ray-sphere trace would suffer from a camera millions-to-billions
// of km away (`dot(O,O) - 1 ~ 1e10` swamps the O(1) discriminant).

// Drawn within the far plane, on the same shell radius as the backdrop. Depth
// is irrelevant (the pass neither tests nor writes it - billboards are always
// the far bodies and are painted over by the later opaque Terra/Luna/meshes).
const PLANET_BILLBOARD_SHELL_KM: f32 = STARS_RADIUS_KM;
// The quad spans the equatorial angular radius times this margin, so the full
// ellipse (corners included) lands inside the quad; edge rays miss and discard.
const PLANET_BILLBOARD_MARGIN: f32 = 1.15;

struct PlanetBillboardOutput {
    @builtin(position) position: vec4<f32>,
    // View-plane offset of this fragment from the planet center, in units of
    // the equatorial radius (the orthographic impostor coordinate); spans
    // [-MARGIN, MARGIN]^2 across the quad.
    @location(0) offset: vec2<f32>,
    // The camera->planet direction (render frame) and the quad's screen basis.
    // Identical at every vertex, so interpolation returns the constant.
    @location(1) dir: vec3<f32>,
    @location(2) right: vec3<f32>,
    @location(3) up: vec3<f32>,
};

@vertex
fn vs_planet_billboard(@builtin(vertex_index) vertex_index: u32) -> PlanetBillboardOutput {
    // Two triangles covering [-1, 1]^2 (same quad idiom as vs_marker).
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[vertex_index];

    let to_planet = planet.pos - uniforms.camera_pos;
    let dist = length(to_planet);
    let dir = to_planet / dist;

    // A stable basis perpendicular to the view direction. Use world up unless
    // dir is nearly parallel to it (planets sit near the ecliptic, so this
    // rarely triggers), then fall back to the X axis.
    var up_ref = vec3<f32>(0.0, 1.0, 0.0);
    if abs(dir.y) > 0.999 {
        up_ref = vec3<f32>(1.0, 0.0, 0.0);
    }
    let right = normalize(cross(up_ref, dir));
    let up = cross(dir, right);

    // Quad on the backdrop shell, sized to the equatorial angular radius (with
    // margin). req/dist is tan(angular radius), exact at these small angles.
    let half_size = PLANET_BILLBOARD_SHELL_KM
        * (planet.equatorial_radius_km / dist)
        * PLANET_BILLBOARD_MARGIN;
    let center = uniforms.camera_pos + dir * PLANET_BILLBOARD_SHELL_KM;
    let world = center + right * (corner.x * half_size) + up * (corner.y * half_size);

    var out: PlanetBillboardOutput;
    out.position = uniforms.view_proj * vec4<f32>(world, 1.0);
    out.offset = corner * PLANET_BILLBOARD_MARGIN;
    out.dir = dir;
    out.right = right;
    out.up = up;
    return out;
}

@fragment
fn fs_planet_billboard(in: PlanetBillboardOutput) -> @location(0) vec4<f32> {
    let req = planet.equatorial_radius_km;
    let rpol = planet.polar_radius_km;
    let radii = vec3<f32>(req, rpol, req);
    let rot_t = transpose(planet.rot);

    // Parallel view direction (eye -> planet) and the view-plane basis, all in
    // the body frame.
    let vd = normalize(rot_t * normalize(in.dir));
    let rb = rot_t * normalize(in.right);
    let ub = rot_t * normalize(in.up);

    // Ray origin on the plane through the planet center, offset by this
    // fragment's perpendicular distance (km = offset * req).
    let o_body = (in.offset.x * req) * rb + (in.offset.y * req) * ub;

    // Intersect the oblate ellipsoid by scaling into unit-sphere space; every
    // term stays O(1) (the parallel-ray precision win).
    let o1 = o_body / radii;
    let d1 = vd / radii;
    let a = dot(d1, d1);
    let b = dot(o1, d1);
    let c = dot(o1, o1) - 1.0;
    let disc = b * b - a * c;
    if disc < 0.0 {
        discard;
    }
    // The near (eye-facing) root: vd points into the planet, so the smaller
    // root is the front surface.
    let t = (-b - sqrt(disc)) / a;
    let p1 = o1 + t * d1; // point on the unit sphere

    // Body-frame geodetic normal (ellipsoid gradient) + equirectangular UV, the
    // same conventions as the mesh path.
    let n_body = normalize(p1 / radii);
    let uv = vec2<f32>(
        atan2(p1.x, p1.z) / (2.0 * PI) + 0.5,
        acos(clamp(p1.y, -1.0, 1.0)) / PI,
    );

    let albedo = textureSampleLevel(planet_texture, planet_sampler, uv, 0.0).rgb;
    let n = normalize(planet.rot * n_body);
    let surf = planet.pos + planet.rot * (p1 * radii);
    let sol = normalize(uniforms.sol_pos - surf);
    let sunlit = max(dot(n, sol), 0.0);

    let color = albedo * (PLANET_AMBIENT + sunlit);
    return vec4<f32>(color, 1.0);
}
