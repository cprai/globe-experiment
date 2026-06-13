# PHASE1 — Project state and technical context

Snapshot at the end of phase 1 (2026-06-11). This documents everything an
agent or developer needs to make changes without re-deriving the design.
Companion docs: `claude/phase1/PLAN.md` (original milestones, all of v1 complete
except tile streaming stretch goals) and `claude/phase1/OPTIMIZE.md` (startup
performance ideas, partially explored).

## What the project is

An interactive Google-Earth-style 3D globe viewer. Rust, edition 2024.
GUI and windowing via **iced 0.14**; all 3D rendering via **wgpu** embedded
in iced's widget tree through the `iced::widget::shader` widget. Features:
day/night Earth with city lights, normal-mapped terrain, GGX ocean sun
glint, Hillaire-2020-based atmospheric scattering (precomputed LUTs),
star/sun backdrop, orbital camera with pan/tilt/zoom + flick inertia, and
two UI sliders controlling the sun position.

## Dependencies (Cargo.toml)

Runtime:
- `iced = "0.14.0"` — GUI. **Critical: wgpu is used via the `iced::wgpu`
  re-export (wgpu 27.0). Never add wgpu to Cargo.toml directly** — version
  drift produces cross-version type errors.
- `glam = "0.33.1"` — camera/sun math (Mat3/Mat4/Quat/Vec3).
- `bytemuck = { version = "1.25", features = ["derive"] }` — Pod casts for
  vertex/uniform/LUT data.
- `half = { version = "2.7", features = ["bytemuck"] }` — f16 LUT texels.
- `image = { version = "0.25", default-features = false, features =
  ["jpeg", "png", "tiff"] }` — runtime texture decoding.

Build-dependencies:
- `ureq = "3.3"` — asset download in build.rs.

Profile: `[profile.dev.package.X] opt-level = 3` for `image`, `zune-jpeg`,
`zune-core`, `tiff`, `miniz_oxide`, `weezl` — the five textures are 33 MP
each and unoptimized decode is multi-second. These overrides also apply to
build-dependencies.

## File map

```
build.rs                 downloads the 5 textures into assets/ (gitignored)
src/main.rs              iced app: App state, Message, update, view
src/globe/mod.rs         shader::Program impl, input handling, inertia
src/globe/camera.rs      orbital camera
src/globe/sun.rs         sun position + star map orientation
src/globe/mesh.rs        UV-sphere generator
src/globe/pipeline.rs    shader::Primitive + Pipeline (all wgpu objects)
src/globe/atmosphere.rs  CPU bake of the atmosphere LUTs
shaders/globe.wgsl       ALL shader code (3 passes in one module)
assets/                  gitignored; populated by build.rs on first build
claude/phase1/           planning/context docs (this file, PLAN, OPTIMIZE)
```

## iced integration (the part that's easy to get wrong)

- App entry: `iced::application(App::default, update, view).title(...).run()`
  — iced 0.14's function-based API (no Sandbox/Application traits; 0.13
  examples online don't apply).
- The globe is `iced::widget::shader(Globe::new(camera, sun))`, mapped into
  the app message type via `Element::from(...).map(Message::Globe)`.
- `Globe` implements `shader::Program<Interaction>`:
  - `update()` receives `iced::Event` + bounds + cursor, returns
    `Option<Action<Interaction>>`. `Action::publish(msg)` sends a message
    to the app **and implies a redraw**; `.and_capture()` stops event
    bubbling; `Action::request_redraw()` requests a frame without a
    message.
  - `draw()` returns the `Primitive` (a cheap value carrying `Camera` +
    `Sun` copies).
- `pipeline::Primitive` implements `shader::Primitive` (trait from
  `iced_wgpu`): `prepare(pipeline, device, queue, bounds, viewport)` writes
  uniforms each frame; `draw(pipeline, render_pass) -> bool` records into
  **iced's shared render pass** (returning `true` = handled; we never use
  the separate `render()` path).
- `pipeline::Pipeline` implements `shader::Pipeline`: `new(device, queue,
  format)` is called **once, lazily, on the first frame** — all texture
  decode/upload, LUT baking, and pipeline creation happens there
  (this is the startup blank-window cost; see OPTIMIZE.md).
- **Loading is intentionally sequential.** A parallel `thread::scope`
  version was implemented and then deliberately reverted by the project
  owner. Do not reintroduce it without asking.
- **iced creates the wgpu device with `Features::empty()`** (verified in
  iced_wgpu 0.14 compositor source) and provides no way to request
  features. Consequences: no BC/compressed texture formats, no
  feature-gated anything. GPU-compressed textures would require vendoring
  and patching iced_wgpu.
- iced redraws only on events. Camera-changing interactions publish
  messages (which redraw); the inertia loop sustains itself by publishing
  from `RedrawRequested` events until velocity decays. There is **no
  continuous render loop** — idle = zero GPU work. (A time-animated water
  effect was added then made static again partly for this reason.)

## Coordinate and mapping conventions (used consistently everywhere)

- The globe is a **unit sphere at the origin** in world space; distances in
  globe radii. +Y = north pole. Longitude 0°, latitude 0° faces **+Z**;
  position on sphere = `(cos(lat)·sin(lon), sin(lat), cos(lat)·cos(lon))`.
- Equirectangular UVs: u = (lon+180°)/360°, v = 0 at north pole → 1 at
  south. The mesh duplicates the seam column (u=0 and u=1) so the texture
  wraps; sampler repeats on U, clamps on V.
- Tangent frame at a surface point (used for normal mapping and camera):
  `east = normalize(Y × radial)` (in WGSL: `normalize(vec3(n.z, 0, -n.x))`),
  `north = radial × east`.
- Since the mesh is a unit sphere, **vertex position = surface normal =
  world position**; the shaders rely on this identity.
- Mesh: `uv_sphere(64, 128)` (stacks, slices), ~8.4k vertices, u32 indices,
  CCW winding viewed from outside.

## Camera (src/globe/camera.rs)

Orbital model: `longitude`, `latitude` (look-at point on the surface,
degrees), `distance` (eye to look-at point, radii), `tilt` (degrees off
nadir, 0 = straight down). Defaults: 0°N 0°E, distance 2.0, tilt 0.
- `frame()` builds eye/target/up: target on unit sphere; tilt rotates the
  eye offset around the local east axis (tilting reveals the horizon to the
  north). `view_proj(aspect)` = `perspective_rh(45°, aspect, 0.01, 50.0)` ×
  `look_at_rh`. glam's `_rh` variants give wgpu's 0..1 depth (no depth
  buffer is used anyway — see render passes).
- Clamps: latitude ±89°, distance 0.01..10.0 (`MIN/MAX_DISTANCE`), tilt
  0..80° (`MAX_TILT`). Longitude wraps via `rem_euclid`.
- `pan_degrees_per_pixel(viewport_height)` — cursor-stable panning:
  `2·distance·tan(fov/2)/height` world units per pixel, ×degrees-per-radian
  (unit sphere: 1 world unit of ground = 1 radian). Used by both live drags
  and inertia.

## Input (src/globe/mod.rs)

`Interaction` enum: `Pan{dlon,dlat}`, `Zoom{factor}`, `Tilt{degrees}` —
applied to app-owned camera state in `main.rs::update`. Camera state lives
in the app, NOT in the widget; widget state only tracks gestures.
- Left drag pans (globe follows cursor: dlon = −dx·scale, dlat = +dy·scale).
- Right drag: vertical = tilt (−dy·0.25°/px). Horizontal currently unused
  (heading rotation was deliberately left out of v1).
- Wheel zooms: `factor = 0.9^ticks` (Lines delta = ticks; Pixels delta /60).
  Requires cursor over widget.
- Flick inertia: drag velocity tracked as an exponential moving average
  (`alpha = 1−e^(−20·dt)`, frame-rate independent). On left-button release:
  if speed > `FLICK_SPEED` (50 px/s) and last move < `FLICK_TIMEOUT`
  (0.1 s) ago, coast. Coasting integrates in `RedrawRequested` using the
  event's timestamp (dt capped at 0.1 s), decays with `HALF_LIFE` 0.3 s,
  stops below `STOP_SPEED` 15 px/s, publishes `Pan` each frame (which
  sustains the redraw loop). Pressing a button cancels inertia.
- Cursor: `Interaction::Grab`/`Grabbing`.

## Sun model (src/globe/sun.rs)

`Sun { longitude, latitude }` = the **subsolar point** in degrees. Default
(−40°, 15°). `direction()` = unit vector to the sun (same formula as any
lat/lon point). Designed for future animation: time of day sweeps longitude
westward 360°/day (solar noon at UTC hour h ≈ (12−h)·15°); season moves
latitude ±23.44°.

`star_rotation() -> Mat3` = `R_y(lon) · R_x(−lat)`: the star map is
**rigidly attached to the sun** (sun pinned at the map's center). This is
deliberately NOT astronomically correct — the user explicitly rejected the
physical model (sky rotating only about the polar axis with RA derived from
declination) because both sliders then rotated the sky around the same
axis. Latitude tilts the sky off the poles; that's the intended behavior.

## Rendering (src/globe/pipeline.rs + shaders/globe.wgsl)

One WGSL module, one shared bind group (group 0), one shared sphere
vertex/index buffer, three render pipelines drawn in this order into
**iced's render pass** (no depth buffer anywhere; ordering does the
occlusion):

1. **stars** (`vs_stars`/`fs_stars`): sphere ×35 (STARS_RADIUS; must
   enclose camera max distance ~11 but stay inside far plane 50),
   front-face culling (seen from inside), no blending. Per-vertex it
   computes the **camera-relative** view direction (`world − camera_pos`,
   linear ⇒ interpolation exact), once rotated by `star_rot_inv` for the
   equirectangular star lookup (per-fragment atan2/acos — per-vertex UVs
   would smear the seam) and once raw for the **sun disc**: angle between
   view dir and sun dir < SUN_ANGULAR_RADIUS (0.012 rad) draws an AA'd
   disc + cubic glow (SUN_GLOW_RADIUS 0.12, STRENGTH 0.5). Backdrop is a
   function of view *direction* only (true infinity — no parallax, no
   zoom dependence); the globe drawn after it provides occlusion.
2. **globe surface** (`vs_main`/`fs_main`): unit sphere, back-face culling,
   no blending. Details below.
3. **atmosphere** (`vs_atmosphere`/`fs_atmosphere`): sphere ×ATMOSPHERE_SHELL
   (= 6460/6360), front-face culling, **additive blending** (One/One).
   Covers the whole planet disc (aerial perspective) and the limb ring.

### Uniforms (must match in Rust `Uniforms` struct and WGSL)

```
view_proj: mat4x4<f32>
camera_pos: vec3<f32> + 1 f32 pad
sun_dir: vec3<f32>    + 1 f32 pad
star_rot_inv: mat3x3<f32>   // Rust side: 3 columns each padded to [f32;4]
```
WGSL mat3x3 columns have vec4 stride — the Rust struct pads each column.
`star_rot_inv` = transpose of `star_rotation()` (orthonormal ⇒ inverse).
Written every frame in `prepare()`; aspect from widget bounds.

### Bind group 0 layout

| binding | resource | format/notes |
|---|---|---|
| 0 | uniforms | visibility VERTEX_FRAGMENT |
| 1 | day texture | Rgba8UnormSrgb, 8192×4096 |
| 2 | `earth_sampler` | repeat U, clamp V, linear |
| 3 | night texture | Rgba8UnormSrgb |
| 4 | normal map | **Rgba8Unorm (linear — data, not color!)** |
| 5 | specular mask | Rgba8Unorm (linear), .r = water |
| 6 | transmittance LUT | Rgba16Float 256×64 |
| 7 | `lut_sampler` | clamp both, linear |
| 8 | inscatter Rayleigh LUT | Rgba16Float 256×128 |
| 9 | inscatter Mie LUT | Rgba16Float 256×128 |
| 10 | stars texture | Rgba8UnormSrgb |

All textures mip_level_count 1 (no mipmaps yet — known shimmer at far zoom,
noted in PLAN.md as acceptable for v1).

### Surface shading (fs_main)

- Normal mapping in the analytic equirectangular tangent frame;
  `NORMAL_STRENGTH` (currently 4.5) scales the tangent components —
  deliberately exaggerated relief per user request. Assumes OpenGL-style
  green-up normal map (looked correct).
- Cook-Torrance GGX: D = GGX, G = Smith (k = a/2), F = Schlick. Roughness
  and F0 blend between land and ocean by the specular mask
  (LAND_ROUGHNESS 0.9 / OCEAN_ROUGHNESS ~0.45, LAND_F0 0.015 /
  OCEAN_F0 ~0.15 — ocean values are user-tuned for a wide bright glint;
  check current file values before assuming).
- Static two-octave value noise (hash-based, WAVE_SCALE/WAVE_STRENGTH)
  modulates the ocean glint around its mean — surface texture, was
  animated once and deliberately made static + subtle.
- (A day-side saturation boost and an `OCEAN_TINT` water-darkening tint
  were implemented and later reverted by the user — as of end of phase 1
  the albedo is used as sampled. Don't re-add without asking.)
- Sunlight color = `sun_transmittance(PLANET_RADIUS+0.1, cos_sun)` from the
  LUT — this is what makes the terminator orange. Applied to diffuse AND
  specular.
- Night side: emissive city-lights texture, blended across the terminator
  with `smoothstep(-0.12, 0.18, cos_sun)` on the **geometric** normal
  (bump detail would speckle the edge).

### Atmosphere (precomputed, after Hillaire 2020)

CPU bake at startup in `atmosphere.rs` (fast, <1 s even in debug):
- Medium: Rayleigh β=(5.802,13.558,33.1)e-3/km, H=8 km; Mie scatter
  3.996e-3, extinction 4.40e-3, H=1.2 km, g=0.8 (Cornette-Shanks); ozone
  absorption (0.650,1.881,0.085)e-3 × tent(25 km ±15 km). Planet radius
  6360 km, atmosphere top 6460 km. **These constants exist in BOTH
  atmosphere.rs and globe.wgsl and must stay in sync** (the WGSL side keeps
  only the geometric ones + MIE_G now; the medium lives in Rust).
- Transmittance LUT: Bruneton (r,μ) parameterization — x = normalized
  distance-to-atmosphere-top along the ray, y = ρ/H_top where
  ρ=√(r²−Rp²). Sampled in WGSL by `sun_transmittance(r, mu)` which also
  applies the below-horizon planet-shadow cutoff.
- Inscatter LUTs (the per-pixel-raymarch replacement; key insight: scene is
  always a sphere seen from outside, so a view ray ≡ (impact parameter b,
  sun cosine μ_ref at a reference point), and phase functions factor out
  because dir·sun is constant along a ray):
  - Row mapping (y): **split** — lower half b∈[0,Rp] = ground-hitting rays
    (reference point = ground hit), upper half b∈[Rp,Ra] = limb rays
    (reference = closest approach). Must match between bake and
    `fs_atmosphere`.
  - Column (x): μ_ref ∈ [−1,1] mapped linearly.
  - Two tables: Σ Rayleigh and Σ Mie (inscatter integrated with view-path
    transmittance and per-step analytic integration, Hillaire eq. 11),
    WITHOUT phase functions — `fs_atmosphere` applies
    `phase_R·Σ_R + phase_M·Σ_M` and the exposure roll-off
    `1 − exp(−L·SUN_INTENSITY)` (SUN_INTENSITY 12.0).
  - Known approximation: the sun's tilt along the ray is baked as
    perpendicular; exact would need Hillaire's per-frame sky-view LUT.
- Terminator orange tuning knobs (documented in conversation): Rayleigh
  blue coefficient ↑, ozone ×2 (most sunset-like), terminator smoothstep
  upper edge ↓, `pow(sun_light, >1)` for artistic saturation.

## App layer (src/main.rs)

`App { camera: Camera, sun: Sun }`. `Message::{Globe(Interaction),
SunLatitude(f32), SunLongitude(f32)}`. View = `stack![shader_globe,
sun_control_panel]` — the panel is a `container(column![text, slider,
text, slider])` overlay (top-left, width 260): latitude slider
−23.44..=23.44 step 0.1 (= season), longitude −180..=180 step 0.5 (= time
of day). The stack pattern is the proven way to overlay iced widgets on
the shader (slider captures its own events; globe gets the rest).

## Build system (build.rs)

- `ASSET_URLS`: five solarsystemscope.com textures
  (8k_earth_daymap.jpg, 8k_earth_nightmap.jpg, 8k_earth_normal_map.tif,
  8k_earth_specular_map.tif, 8k_stars_milky_way.jpg; all 8192×4096).
- `download_if_missing(url)`: derives filename from the URL's last
  segment → `assets/<name>`, emits `cargo::rerun-if-changed=<path>`,
  skips if present, otherwise downloads via ureq with a 100 MB body cap.
  Deleting a file re-triggers just its download.
- `assets/` is in .gitignore. Textures are embedded with `include_bytes!`
  at compile time (~21 MB in the binary), so the build script must run
  before compilation — cargo guarantees this. Fresh clone = first build
  downloads everything (needs network).

## Known issues / context for future work

- Startup: window opens, then blank widget until `Pipeline::new` finishes
  (sequential decode of 5×33 MP + LUT bake + pipeline creation).
  OPTIMIZE.md has the ideas; parallel loading was tried and **reverted by
  the user — ask before re-adding**. User perf-tests native Windows
  release builds (not WSL).
- BC7/compressed textures are blocked by iced's `Features::empty()` device.
- WSLg (the dev environment here) intermittently fails app launch with
  libEGL/MESA errors — transient, retry; not present on native Windows.
- The `cargo add` tool can fail with a bogus "found cargo.toml please
  rename" error on this case-insensitive Windows mount; verify with
  `cargo metadata` before believing it.
- Smoke-test pattern used throughout: `timeout 12 cargo run 2>&1 | head`
  — wgpu validation errors panic in the first frames, so a clean 10–15 s
  run = pipelines/bindings valid. Pipe buffering can swallow output;
  redirect to a file if output looks missing.
- No HDR/bloom: LDR straight to swapchain; sun "brightness" is the
  disc+glow cheat. Real blinding sun would need offscreen HDR + post.
- No heading control, no fly-to animation, no tile streaming (the v2
  leap), no mipmaps, no runtime texture loading.
- Shader look-tuning constants live at the top of `shaders/globe.wgsl`
  and are frequently user-tuned between sessions — read the file for
  current values rather than trusting docs.

---

# Shader algorithms and math (detailed reference)

Everything below is implemented in `shaders/globe.wgsl` (runtime) and
`src/globe/atmosphere.rs` (CPU bake). Vector convention reminder: the
globe is a unit sphere at the origin, so for any surface fragment
`in.normal` = vertex position = geometric normal = world position. All
directions below are unit vectors; `·` is the dot product.

## Texture inventory and exactly how each is used

| Texture | Binding | Format | Sampled by | Channels used | Role |
|---|---|---|---|---|---|
| `day_texture` (8k_earth_daymap.jpg) | 1 | Rgba8UnormSrgb | fs_main | rgb | Surface albedo for the lit side. sRGB format ⇒ hardware linearizes on sample, so all lighting math is in linear space. |
| `night_texture` (8k_earth_nightmap.jpg) | 3 | Rgba8UnormSrgb | fs_main | rgb | **Emissive** city lights. Never multiplied by any light term — cities glow on their own; blended in only by the terminator factor. |
| `normal_texture` (8k_earth_normal_map.tif) | 4 | Rgba8Unorm (**linear**) | fs_main | xyz | Tangent-space normal map, unpacked `*2−1`. x = east offset, y = north offset (OpenGL green-up convention), z = up. Linear format is load-bearing: sRGB decoding would warp the vectors. |
| `specular_texture` (8k_earth_specular_map.tif) | 5 | Rgba8Unorm (linear) | fs_main | r only | Water mask, 1 = ocean. Blends BRDF parameters (roughness, F0) and gates the wave noise. Smooth values along coastlines give soft material transitions. |
| `transmittance_lut` (baked) | 6 | Rgba16Float 256×64 | fs_main + bake | rgb | T(r, μ): fraction of sunlight surviving to the top of atmosphere. See parameterization below. |
| `inscatter_rayleigh_lut` (baked) | 8 | Rgba16Float 256×128 | fs_atmosphere | rgb | Σ_R: view-ray Rayleigh scattering integral, phase factored out. |
| `inscatter_mie_lut` (baked) | 9 | Rgba16Float 256×128 | fs_atmosphere | rgb | Σ_M: same for Mie. |
| `stars_texture` (8k_stars_milky_way.jpg) | 10 | Rgba8UnormSrgb | fs_stars | rgb | Equirectangular sky backdrop, sampled by direction (not mesh UVs). |

Samplers: `earth_sampler` (repeat U — dateline seam, clamp V — poles,
linear) is shared by all image textures including stars; `lut_sampler`
(clamp/clamp, linear) by the three LUTs. LUTs are read with
`textureSampleLevel(..., 0.0)` because `fs_stars`/`fs_atmosphere` also
use it in non-uniform control flow and there are no mips anyway.

## Equirectangular mapping (both directions)

Forward (mesh generation, `mesh.rs`): for stack/slice fractions (u,v),
`lat = 90° − 180°·v`, `lon = 360°·u − 180°`, position =
`(cos lat · sin lon, sin lat, cos lat · cos lon)`.

Inverse (fs_stars, sampling by direction d):
`u = atan2(d.x, d.z) / 2π + 0.5`, `v = acos(d.y) / π`.
The inverse runs **per fragment**; interpolating u across a triangle that
crosses the ±180° seam would smear the whole texture width backwards.

## Analytic tangent frame (fs_main)

For geometric normal n (= position on unit sphere):
`east = normalize((n.z, 0, −n.x))` — this is the exact normalized
∂position/∂longitude; `north = n × east`. No per-vertex tangents needed,
no seam issues, exact everywhere except the poles (where east degenerates;
the textures are near-constant there so it's invisible).

Normal mapping: with sampled offsets (x, y, z),
`n' = normalize(east·x·S + north·y·S + n·z)` where S = NORMAL_STRENGTH.
S scales the tangent-plane components before renormalization, so the
effective slope is multiplied by ≈S (deliberately exaggerated; S=1 is the
map's face value).

## Surface BRDF — Cook-Torrance with GGX (fs_main)

Specular = `D·G·F / (4·(n·v)·(n·l)) · (n·l)`, with α = roughness²:

- **D, GGX/Trowbridge-Reitz**: `D = α² / (π · ((n·h)²(α²−1) + 1)²)`.
  h = normalize(v + l) is the half vector.
- **G, Smith with Schlick-GGX**: `G = G₁(n·v) · G₁(n·l)`,
  `G₁(x) = x / (x(1−k) + k)`, `k = α/2`.
- **F, Schlick**: `F = F0 + (1−F0)·(1 − v·h)⁵`. Scalar F0 (dielectrics).

Material parameters come from the water mask m:
`roughness = mix(LAND_ROUGHNESS, OCEAN_ROUGHNESS, m)`,
`F0 = mix(LAND_F0, OCEAN_F0, m)`. Land is rough/dim (no visible glint);
ocean is smoother/brighter → the sun glint is water-only. Wider glint =
raise OCEAN_ROUGHNESS (lobe width ∝ roughness²); brighter glint = raise
OCEAN_F0 (≈linear, lifts the skirt since the LDR core clips at 1.0).
Numeric guards: n·v clamped ≥ 1e-4 and the 4(n·v)(n·l) denominator
clamped ≥ 1e-4 to avoid grazing-angle division blowups.

Diffuse is plain Lambert with an ambient floor:
`albedo · (DAY_AMBIENT + (1−DAY_AMBIENT) · (n·l) · sun_light)`, and
specular is also multiplied by `sun_light` (the atmospheric transmittance
color) so glints redden near the terminator like everything else.

## Wave noise (fs_main)

`hash2(p) = fract(sin(p·(127.1, 311.7)) · 43758.5453)` — the classic
shader hash (deterministic, no texture). `value_noise` = bilinear
interpolation of the four corner hashes with a Hermite fade
`u = f²(3−2f)`. `wave_noise = 0.65·N(uv·K) + 0.35·N(uv·2.3K)`,
K = WAVE_SCALE cells across the map. Applied mean-preserving:
`specular *= mix(1, 1 + WAVE_STRENGTH·(2·wave−1), m)` — modulates ±strength
around 1 so average glint brightness is unchanged; gated by the water
mask. Static by design (was animated; reverted). Known and accepted:
cells compress toward the poles and there's a pattern seam at the
dateline, both invisible at current strength.

## Day/night terminator composition (fs_main)

`cos_sun = n_geo·sun` (geometric normal, NOT the bumped one — bump detail
would speckle the day/night edge). `daylight = smoothstep(−0.12, 0.18,
cos_sun)`; final surface = `mix(night_emissive, day_lit, daylight)`.
The band edges are look-tuning knobs: widen for a slower dusk, shift the
upper edge down to let surface detail survive deeper into the orange zone.

## Atmosphere — single scattering after Hillaire 2020

### Medium (defined in atmosphere.rs; per-km coefficients)

At altitude h (km):
- Rayleigh: `σs_R(h) = (5.802, 13.558, 33.1)e-3 · exp(−h/8)` (scattering
  = extinction; no absorption). The 1:2.3:5.7 blue bias is what makes the
  sky blue and the transmitted light orange.
- Mie: `σs_M(h) = 3.996e-3 · exp(−h/1.2)`, extinction `4.40e-3 · …` (the
  difference is absorption). Phase asymmetry g = 0.8.
- Ozone: absorption only, `(0.650, 1.881, 0.085)e-3 · max(0, 1 −
  |h−25|/15)` — a tent peaking at 25 km. Eats green/red mid-spectrum on
  grazing paths; the biggest single contributor to sunset purples/reds.
- Total extinction `σt(h)` = sum of the three.

### Transmittance LUT (256×64, baked + sampled by `sun_transmittance`)

Beer–Lambert: `T(p, dir) = exp(−∫ σt(h(t)) dt)` integrated to the top of
the atmosphere (40 midpoint steps in the bake). Parameterized à la
Bruneton so resolution concentrates where T changes fastest (near the
horizon):
- `ρ = √(r² − Rp²)` (distance to the horizon point), `H = √(Ra² − Rp²)`.
- y axis: `x_r = ρ/H`.
- x axis: with `d` = distance along the ray to the atmosphere top
  (`d = −r·μ + √(r²(μ²−1) + Ra²)`), `x_mu = (d − d_min)/(d_max − d_min)`
  where `d_min = Ra − r`, `d_max = ρ + H`.
The WGSL sampler additionally returns 0 when μ is below the geometric
horizon cosine `−√(1 − (Rp/r)²)` — the planet's own shadow. The bake
inverts the same mapping to reconstruct (r, μ) per texel, and the bake's
inscatter step re-samples this table on the CPU with a hand-rolled
bilinear fetch that mirrors GPU sampling.

`fs_main` uses T directly as the **color of sunlight at the ground**:
`sun_light = T(Rp + 0.1 km, cos_sun)` — at high sun nearly white, at the
terminator the long grazing path strips blue → orange→red. This single
lookup is what tints the whole lit-surface shading.

### Inscatter LUTs (2×, 256×128) — why no per-pixel raymarch

Single-scattering luminance along a view ray:
`L = ∫ T_view(t) · σs(h) · Φ(μ_view·sun) · T_sun(p(t)) dt`.
Two exploits make this precomputable for this scene:
1. **Phase functions factor out**: `dir·sun` is constant along a straight
   ray, so `L = Φ_R · Σ_R + Φ_M · Σ_M` with Σ = the integral sans phase.
2. **Spherical symmetry**: viewed from outside a sphere, a ray is fully
   characterized by its impact parameter `b = |origin − (origin·dir)dir|`
   (closest approach to the center) and one sun angle. The chosen angle:
   `μ_ref = r̂_ref · sun` at a **reference point** — the ground hit for
   rays with `b < Rp`, else the closest-approach point.

LUT axes: x = `μ_ref·0.5 + 0.5`; y = **split mapping** — `y = 0.5·(b/Rp)`
for ground-hitting rays (lower half), `y = 0.5 + 0.5·(b−Rp)/(Ra−Rp)` for
limb rays (upper half). The split gives the thin bright limb band half
the table. **The bake (`bake_inscatter`) and `fs_atmosphere` implement
this mapping independently and must match.**

Bake geometry (canonical frame): ray along +x, closest approach (0, b),
entry `t = −√(Ra²−b²)`, exit at the ground `−√(Rp²−b²)` or shell exit
`+√(Ra²−b²)`. The sun is placed so its cosine at the reference point is
μ_ref; elsewhere on the ray it follows `μ(t) = μ_ref · (p̂(t) · r̂_ref)` —
this encodes the **one approximation** of the scheme: the sun's tilt
*along* the ray is treated as perpendicular (exact would need Hillaire's
per-frame sky-view LUT; the error mildly smooths the terminator gradient).

Per step (32 midpoint steps), Hillaire eq. 11 — analytic inscatter
integration assuming constant medium across the step (exact in that
limit, removes step-count sensitivity):
```
S_step = T_view · σs · T_sun · (1 − exp(−σt·dt)) / σt
T_view *= exp(−σt·dt)
```
accumulated separately for Rayleigh and Mie (Mie's scalar σs broadcast
to rgb because T_sun is colored).

### LUT precomputation pipeline (atmosphere.rs, function by function)

> **Note (post-phase-1):** the bake no longer runs at runtime. Phase 2
> moved it to build time (`build.rs`, see PHASE2.md), and on 2026-06-13
> the `src/globe/atmosphere.rs` source was inlined into `build.rs` as an
> in-file `mod atmosphere` and the file deleted. The function-by-function
> description below is still accurate — the code is byte-for-byte the
> same, it just lives in that inline module now and runs at build time
> rather than inside `Pipeline::new`.

Runs once on the calling thread inside `Pipeline::new`, before any
upload. Entry point `bake() -> Luts`:

```
bake_transmittance() ──> Vec<[f32;3]>  (256×64, row-major, kept in f32)
        │
        ├─> to_f16_texels() ─> Luts.transmittance   (RGBA f16)
        │
        └─> bake_inscatter(&transmittance)
                └─> (Luts.inscatter_rayleigh, Luts.inscatter_mie)
```

The transmittance table is deliberately kept in full f32 for the
inscatter bake (it's re-sampled thousands of times there) and only
converted to f16 for the GPU copy.

**`extinction(h)` / `scattering(h)`** — the medium evaluators shared by
both bakes. `extinction` returns σt rgb (Rayleigh + Mie extinction +
ozone); `scattering` returns (σs_Rayleigh rgb, σs_Mie scalar). Note Mie
scatters less than it extinguishes (3.996e-3 vs 4.40e-3 — the
difference is absorption), which is why both functions exist instead of
one.

**`bake_transmittance()`** — for each texel (i, j):
1. Map texel center → parameters by *inverting* the Bruneton mapping:
   `x_r = (j+0.5)/H` → `ρ = x_r·H_top` → `r = √(ρ² + Rp²)`; then
   `x_mu = (i+0.5)/W` → ray length `d = d_min + x_mu·(d_max−d_min)` →
   zenith cosine `μ = (H_top² − ρ² − d²)/(2·r·d)` (clamped to [−1,1];
   d ≤ 0 degenerates to μ = 1). This is the algebraic inverse of the
   forward mapping in WGSL `sun_transmittance` — change one side, change
   both.
2. Midpoint-rule integrate optical depth along the ray, 40 steps:
   at `t = (s+0.5)·dt`, the radius is `r(t) = √(r² + t² + 2·r·μ·t)`
   (law of cosines), altitude `h = r(t) − Rp` (clamped ≥ 0), accumulate
   `σt(h)·dt` per channel.
3. Store `exp(−optical_depth)` — Beer–Lambert.

**`sample_transmittance(table, r, μ)`** — CPU twin of the WGSL sampler,
used only inside the inscatter bake. Applies the same below-horizon
shadow cutoff (`μ < −√(1 − (Rp/r)²)` → zero — this is how the planet's
shadow gets baked *into* the inscatter tables), computes the same
(x_mu, x_r) forward mapping, then does a manual bilinear fetch at texel
centers (`fx = x·W − 0.5`, clamped; lerp 4 texels). Mirroring GPU
filtering here keeps bake-time and draw-time reads consistent.

**`bake_inscatter(&transmittance)`** — for each of the 128 rows:
1. Row → ray class + impact parameter (the split mapping):
   `v = (j+0.5)/128`; if `v < 0.5` → ground-hitting, `b = 2v·Rp`;
   else → limb, `b = Rp + 2(v−0.5)·(Ra−Rp)`. (Exactly mirrored by the
   `v` computation in `fs_atmosphere` — these two must never drift.)
2. Canonical ray in 2D: direction +x, closest approach at (0, b).
   Entry `t_entry = −√(Ra²−b²)`; exit `t_exit = −√(Rp²−b²)` (front face
   of the planet) for ground rays, `+√(Ra²−b²)` (shell exit) for limb
   rays.
3. Reference point: the ground hit `(t_exit, b)` for ground rays, the
   closest approach `(0, b)` for limb rays; normalized to `r̂_ref`
   (with a 1e-3 floor on its length — b→0 head-on rays would otherwise
   normalize a zero vector).
4. For each of the 256 columns: `μ_ref = 2·(i+0.5)/256 − 1`, then a
   32-step midpoint march from entry to exit. At each step:
   - `r(t) = √(t² + b²)`, `h = r − Rp` (clamped ≥ 0 — ground rays can
     dip a hair below Rp at the last step).
   - **Sun cosine transfer**: `μ_sun(t) = μ_ref · (p̂(t) · r̂_ref)`.
     This is the perpendicular-tilt approximation in one line: the sun
     is imagined tilted out of the ray plane such that its cosine at the
     reference point is μ_ref; by spherical symmetry its cosine at any
     other point on the ray scales with the angle between the two zenith
     directions. In 2D: `p̂·r̂_ref = (t·ref_x + b·ref_y)/r`.
   - `t_sun = sample_transmittance(r, μ_sun)` (sunlight reaching the
     sample, including planet shadow).
   - Hillaire eq. 11 per channel: `step_trans = exp(−σt·dt)`,
     `integ = T_view · t_sun · (1 − step_trans)/σt` (σt floored at 1e-6),
     `Σ_R += σs_R · integ`, `Σ_M += σs_M · integ`, then
     `T_view *= step_trans`. The σs factors are sampled at the step
     midpoint — pulled out of the analytic integral, consistent with the
     constant-medium-per-step assumption.
5. Texel layout: rgb = Σ, alpha = 1.0, pushed directly as f16 in
   row-major order. Both LUTs are filled in the same loop from the same
   march (one pass, two accumulators).

Cost: 128×256 texels × 32 steps × a bilinear table fetch ≈ 1M medium
evaluations + 4M table lerps — sub-second even unoptimized; reruns on
every app start (no disk cache; OPTIMIZE.md idea 2 covers moving it to
build time).

Gotchas when modifying:
- Any change to the medium constants, the split mapping, the reference
  point choice, or the Bruneton mapping must be made in **both**
  atmosphere.rs and globe.wgsl (constants + `fs_atmosphere`'s v/μ_ref
  computation + `sun_transmittance`).
- f16 max is 65504 and min normal ~6e-5; current Σ magnitudes are far
  inside that, but a large SUN_INTENSITY-style scale factor must stay in
  the shader, not the bake, or precision suffers.
- The μ_ref axis is sampled at texel centers, so μ_ref = ±1 exactly is
  never baked; the clamp sampler makes the shader read the nearest
  column — fine, but don't add math that depends on exact endpoint
  values.

### Runtime composition (fs_atmosphere)

Shell pass fragments: ray/sphere test against Ra (discard misses), find
b and the reference point via `ray_sphere` (quadratic: `t = −b' ± √(b'²−c)`
with `b' = origin·dir`, `c = |origin|² − R²`), classify ground/limb,
sample both LUTs, then:
- Rayleigh phase: `Φ_R = 3/(16π)(1 + μ²)`.
- Mie, Cornette-Shanks: `Φ_M = 3/(8π) · (1−g²)(1+μ²) / ((2+g²)(1+g²−2gμ)^1.5)`
  — strongly forward-peaked at g=0.8, gives the bright halo when looking
  toward the sun.
- `L = Σ_R·Φ_R + Σ_M·Φ_M`, then tone roll-off `color = 1 − exp(−L·SUN_INTENSITY)`
  (soft-clips the limb instead of hard-saturating; SUN_INTENSITY is the
  overall atmosphere brightness knob).
Drawn additively (One/One) over the globe ⇒ doubles as aerial perspective
across the planet disc, and over the stars/sun beyond the limb ⇒ sunrise
glow over the rising sun for free.

## Star + sun backdrop (vs_stars/fs_stars)

The backdrop is "at infinity," so it must be a function of **view
direction from the eye only**: `rel = world_vertex − camera_pos`
(linear in the vertex ⇒ exact under interpolation; normalized per
fragment). Two outputs:
- `dir = star_rot_inv · rel` → equirectangular star lookup (inverse
  mapping above). `star_rot_inv` = transpose of `R_y(sun_lon)·R_x(−sun_lat)`
  — the sky is rigidly attached to the sun (deliberate, non-physical; see
  Sun model section).
- `view = rel` (world frame) → sun disc: `angle = acos(view·sun)`;
  disc = `1 − smoothstep(0.85·R_sun, R_sun, angle)` (anti-aliased edge),
  glow = `STRENGTH · max(1 − angle/R_glow, 0)³` (cubic falloff). Color =
  `stars·BRIGHTNESS + SUN_COLOR·(disc + glow)`. The glow is the LDR
  brightness cheat — clipped-white core, wide soft halo.

Anchoring **both** lookups to the same camera-relative direction is what
keeps sun and stars locked under orbit/zoom; anchoring either to the
sky-sphere surface instead reintroduces parallax between them (a bug that
was found and fixed — don't regress it). The 35-radius sphere geometry is
purely a screen-coverage device; nothing of it shows in the output.

## Numeric/precision notes

- LUT texels are f16 (`half` crate): plenty for transmittance/inscatter
  magnitudes; keeps the three LUTs at ~96 + 256 + 256 KB.
- All clamps (`max(x, 1e-4)`, `clamp(μ, −1, 1)` before `acos`/`asin`)
  exist because grazing geometry otherwise produces NaNs that propagate
  to black/white pixel speckle.
- The atmosphere pass works in km (positions scaled by Rp) for
  readable constants; the surface/star passes stay in globe radii.
