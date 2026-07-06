---
paths:
  - "shaders/scene.wgsl"
---

# Shader rules & invariants

## Surface format (load-bearing)

- **Surface format: non-sRGB.** `Gfx::init` picks `find(|f| !f.is_srgb())`.
  `get_default_config` defaults to sRGB format — this override is deliberate.
  Every look-tuning constant in `scene.wgsl` is calibrated to the non-sRGB
  surface. Do not switch to sRGB.
- **Headless render target must also be non-sRGB** (`Rgba8Unorm`). Do not
  "fix" `OffscreenRenderer` (src/offscreen.rs) to `Rgba8UnormSrgb`. The stored 8-bit bytes
  already equal the sRGB-encoded on-screen pixels and are written verbatim
  to PNG (no gamma transform on readback).
- **No HDR, no bloom.** LDR only. A real bloom pass is explicitly declined.
- **Present mode: `AutoVsync`.** `get_default_config` picks Mailbox on DX12,
  which causes judder. `AutoVsync` is deliberate.

## Rendering invariants

- **Reversed-Z depth buffer (`Depth32Float`).** Cleared to `0.0` (the far
  plane), `depth_compare: Greater` (nearer = larger depth). It exists so the
  Terra occludes the much more distant Luna (incl. a partial limb). Per-pass
  policy (in `SceneRenderer::new`): the **body impostors** (via
  `@builtin(frag_depth)` — the only depth-writing pass)
  write depth + test `Greater`; the **backdrop, atmosphere, markers** neither
  write nor test (`Always`, no write) so they keep their exact draw-order
  layering. Because the atmosphere does not depth-test, `fs_atmosphere` does an
  explicit ray-sphere **Luna occlusion** check (drops the fragment where the ray
  hits Luna in front of the atmosphere) so the additive glow does not bleed
  over a nearer Luna from a Luna-orbit view. The projection is reversed-Z
  (`renderer::view_proj_reversed_z` post-
  multiplies a Z-flip onto `perspective_rh`); the clear value, compare op, and
  projection must all agree or geometry vanishes. egui's overlay pipeline is
  built with `depth_stencil_format: Some(DEPTH_FORMAT)` (depth-off, draws on
  top) so it is compatible with the depth attachment.
- **Draw order: stars -> body impostors (ALL nine: Terra + planets + Luna) ->
  atmosphere -> orbit paths -> markers**, one render pass. EVERY
  body is a **single shader
  impostor** (`vs_planet`/`fs_planet`): a camera-facing quad (no vertex buffer)
  placed in screen space by the CPU (in `prepare`, projecting the body center
  to NDC) that ray-traces the triaxial ellipsoid (Luna genuinely triaxial;
  Terra the WGS84 spheroid; each planet oblate with rx = rz), shaded per the
  body's `BODY_FLAG_*` feature bits (bare Lambert up to the full Terra look:
  normal map, GGX ocean, transmittance-tinted sunlight, city lights), and
  **writes per-fragment depth** (reversed-Z; the only depth-writing pass) - so
  the depth buffer resolves body-vs-body occlusion and draw
  order does not matter (impostors draw right after the backdrop). The trace is
  **distance-adaptive**, chosen per body on the CPU by apparent angular size
  (`2*asin(rmax/distance)` vs `PLANET_PERSPECTIVE_MIN_ARCSEC`, `rmax` = the
  largest semi-axis): a near/orbited
  body uses the **perspective** eye-ray trace (reconstructed from the
  fragment's NDC via `inv_view_proj`, f32-safe because distance/radius is small
  there) and writes the true hit-point depth; a distant body uses the
  **orthographic** parallel-ray trace (built from the quad-corner offset,
  f32-safe at any distance) and the center's baseline depth. **Quad placement
  also depends on the mode**: a distant body's quad is anchored at its
  projected center sized to the angular radius (tight + cheap), but a
  perspective body's quad covers the **whole screen** (`[-1,1]^2`) - its
  projected center can fall far off-screen at high tilt (the center is far off
  the view axis while the near surface still fills the frame), so a
  center-anchored quad would follow the center off-screen and the body would
  vanish; a full-screen quad + per-pixel ray-trace (misses discard) always
  covers the visible surface. At most the orbited body, Terra from a Luna
  orbit (~1.9 deg), and (near its perigee, from a Terra orbit - Luna's
  apparent diameter straddles the cutoff over the month) Luna are perspective,
  so this is at most a few full-screen passes. The
  group-1 layout is shared (per-body bind groups; dummies fill the optional
  map slots); only the solar-system scenario shows the planets prominently
  (the Terra/Luna views carry them too, but far off-screen).
- **Only the atmosphere and the satellite overlays are gated; every body
  always draws.** The atmosphere (a CPU-sized screen quad) draws when a
  `has_atmosphere` body sits bit-exactly at the render origin
  (`draw_atmosphere`; Terra under a Terra/Luna target today - its LUT math
  assumes the body at the origin, and from a planet orbit it is sub-pixel).
  Orbit paths + satellite markers draw only when `render_origin == 0`
  (`draw_satellite_overlays`; their positions are Terra-frame world
  coordinates).
- **Render frame (floating origin): the shader is fully camera-target-local.**
  Every position uniform (`camera_pos`, `luna_occluder`, `sol_pos`, the per-planet
  `pos`) is **already relative to the camera target** — the renderer subtracts
  the origin (`camera_target.render_origin()`) on the CPU, so there is NO
  `render_origin` uniform and no `world - render_origin` in any vertex shader.
  The orbited body's `pos` is a bit-exact zero, so it is drawn in pure local
  coordinates (the f32-jitter fix). For Terra/Luna (`render_origin` = Terra's
  center) the render frame is the Terra-centered (old geocentric) frame (the
  `CelestialSphere` is heliocentric, but the subtraction cancels Terra's
  center - done in f64, like all of `prepare`'s math, to dodge f32
  cancellation; f32 appears only in the uniform layouts themselves).
  There is
  **no `sol_dir`**: every lit pass
  derives its Sol direction from `sol_pos` (`normalize(sol_pos - world_pos)` for
  surfaces, `normalize(sol_pos)` for the backdrop disc).
- **Markers are instanced screen-space overlays** drawn last. CPU occlusion
  per marker (`marker_occluded` in `src/engine/simulation/mod.rs`). No depth test.
- **Orbit paths are instanced mitered line segments** (`vs_path`/`fs_path`),
  drawn just before the markers: constant pixel width via the marker `clip.w`
  trick, watertight miter joints from per-instance neighbor samples (any
  alpha-blended quad overlap - even AA fringes - beads at every joint), and the
  scene's only depth **test-without-write** pass (`Greater`, no write) so
  solids occlude the path's far side. Vertices keep the centerline endpoint's
  clip z/w, so the fat quad depth-tests as the thin 3D line. Fade alpha is a
  per-endpoint instance value computed on the CPU.
- **Mutual eclipse shadows are analytic** (`sol_visibility` in `scene.wgsl`):
  the soft two-disk overlap of Sol and an occluding sphere, with the Sol
  angular radius passed per call. Every body is shadowed **generically** in
  `fs_planet`: its uniform carries a fixed-size occluder list (`occluders`,
  `MAX_OCCLUDERS` slots; radius 0 = unused) that the CPU fills with the body's
  same-system neighbors (`CelestialBody::same_system` - Terra shadowing
  Luna, the lunar-eclipse coppery glow, AND Luna shadowing Terra, the
  solar-eclipse spot), plus a per-body Sol angular radius
  computed from the true Sol distance. A future moon system self-shadows by
  adding its enum arm to `same_system` - no renderer/shader change. No shadow
  maps.
- **Idle = zero GPU work.** Never add an unconditional vsync loop.
  `ControlFlow::Wait` + targeted `request_redraw`. The clock starts playing,
  so the app is non-idle from launch until paused.
- **Terminator / night-side darkening must use the GEOMETRIC normal**
  (`dot(n_geo, sol)`), never the bump-mapped normal `n`. Bump detail on the
  day/night edge speckles it.
- **Star map and Sol are ephemeris-driven.** Sol *position* from JPL DE440
  (uploaded as `sol_pos`, render-frame); `star_rot_inv = P * R_itrf2gcrf * P^T`.
  Do not replace with a Sol-attached rotation. The star matrix additionally folds
  in a static galactic->equatorial offset (`star_tex_rot_inv`), because the star
  texture is drawn in galactic coordinates; see `simulation.md`.
- **Backdrop anchoring**: star lookup is a function of the camera-relative view
  direction, not position on the celestial sphere. Changing the *star* anchoring
  reintroduces parallax between Sol and stars (a fixed bug). The backdrop is a
  **full-screen quad** whose fragment shader reconstructs the per-pixel view
  direction from NDC via `inv_view_proj` - trivially camera-centered, so it
  encloses the eye at any orbit target (an origin-anchored shell excluded a
  Luna-orbit eye; Sol and half the sky vanished). The **Sol disc**
  (`fs_stars`) is drawn in
  the orbited body's direction to Sol (`normalize(sol_pos)`, render-frame) so
  it agrees with each planet's terminator (`fs_planet` lights from the same Sol
  position). See `camera.md`.

## Look-tuning discipline

- **Look-tuning constants drift between sessions.** Always read `scene.wgsl`
  for live values; the snapshot below is dated.
- **Tune and feel-test on a native Windows release build.** The WSLg dev
  environment cannot validate exact colors or interaction feel.

## City light design decisions (intentional, do not revert)

Two live constants depart from the original phase-3 plan:
- **`NIGHT_DARKNESS = 1.2`** (plan was ~0.02): because
  `night_factor = mix(NIGHT_DARKNESS, 1.0, daylight)`, a value > 1 makes the
  unlit hemisphere ~20% **brighter** than full daylight, so Terra reads
  bright all the way around with city glow on top. Owner-confirmed.
- **`EMISSIVE_THRESHOLD = 0.05`** (plan was 0.25): a deliberately permissive
  city mask. Owner-confirmed.

## Marker shader — constant-pixel-size trick

`vs_marker` builds a two-triangle `[-1,1]^2` quad from `@builtin(vertex_index)`.
Per-marker world position + visibility arrive as `MarkerInstance` vertex
attributes. Key non-obvious detail: the quad corner offset is multiplied by
**`clip.w`** before emitting — this pre-compensates the perspective divide
so the circle stays round and size-stable at any depth. Occluded markers
(CPU-decided via `marker_occluded`) and behind-camera markers (`clip.w <= 0`)
emit `(0,0,2,1)` which clips outside NDC, rasterizing nothing.

## Live constant snapshot (2026-07-06 — verify against source)

**`shaders/scene.wgsl`**:
```
DAY_AMBIENT 0.04           NORMAL_STRENGTH 4.5
LAND_ROUGHNESS 0.9         OCEAN_ROUGHNESS 0.45
LAND_F0 0.015              OCEAN_F0 0.15
WAVE_SCALE 2200.0          WAVE_STRENGTH 0.04
EMISSIVE_THRESHOLD 0.05    EMISSIVE_SOFTNESS 0.1
EMISSIVE_COLOR (1.0, 0.85, 0.3)  EMISSIVE_STRENGTH 1.5
EMISSIVE_FADE_START -0.15  EMISSIVE_FADE_END 0.15
DITHER_SCALE 400.0         NIGHT_DARKNESS 1.2
PLANET_RADIUS_KM 6360.0    ATMOSPHERE_TOP_KM 6460.0
MIE_G 0.8                  SOL_INTENSITY 12.0
STARS_BRIGHTNESS 0.8
SOL_ANGULAR_RADIUS 0.012   SOL_GLOW_RADIUS 0.12
SOL_GLOW_STRENGTH 0.5      SOL_COLOR (1.0, 0.96, 0.9)
MARKER_FILL (1.0, 0.25, 0.2)  MARKER_RING (1.0, 1.0, 1.0)
PATH_WIDTH_PX 3.0          PATH_AA_PAD_PX 1.5
PATH_MITER_LIMIT 4.0       PATH_OPACITY 0.85
PATH_COLOR (0.35, 0.65, 1.0)
PLANET_AMBIENT 0.02        ECLIPSE_GLOW (0.06, 0.012, 0.004)
MAX_OCCLUDERS 4 (must match renderer)
terminator: smoothstep(-0.12, 0.18, cos_sol)
```

**`src/engine/renderer/mod.rs`** (depth/eclipse): `DEPTH_FORMAT Depth32Float`,
`SOL_RADIUS_KM 695700` (per-impostor-body Sol angular radius =
asin(SOL_RADIUS_KM/dist); every eclipse penumbra, Terra's included - distinct
from the star pass's `SOL_ANGULAR_RADIUS` disc-size cheat), `ATMOSPHERE_TOP_KM
6460` (CPU twin for the atmosphere quad).

**`build.rs mod atmosphere`**:
```
RAYLEIGH_SCATTERING [5.802, 13.558, 33.1]e-3   RAYLEIGH_SCALE_HEIGHT 8.0
MIE_SCATTERING 3.996e-3   MIE_EXTINCTION 4.40e-3   MIE_SCALE_HEIGHT 1.2
OZONE_ABSORPTION [0.650, 1.881, 0.085]e-3       (tent peak 25 km, +/-15)
TRANSMITTANCE 256x64 / 40 steps   INSCATTER 256x128 / 32 steps
```

**`src/engine/application/input.rs`**:
```
FLICK_SPEED 50   STOP_SPEED 15   HALF_LIFE 0.3   FLICK_TIMEOUT 0.1
ZOOM_HALF_LIFE_MIN 0.01   ZOOM_HALF_LIFE_MAX 0.1   WHEEL_GAP_CAP 0.25
ZOOM_COAST_HALF_LIFE 0.15   ZOOM_STOP_RATE 0.1
```

**`src/engine/planet.rs`** (WGS84 + dynamics; Terra is a table row):
```
SEMI_MAJOR_AXIS_KM 6378.137   INVERSE_FLATTENING 298.257223563
SEMI_MINOR_AXIS_KM ~6356.752  TERRA_MEAN_RADIUS_KM ~6371.0088
TERRA_GRAVITATIONAL_PARAMETER_KM3_S2 398600.4418
TERRA_ANGULAR_VELOCITY_RAD_S 7.292115e-5
```

**`src/engine/camera.rs`** (orbit limits are radius *ratios* `*_RADII`,
scaled by the orbit target's `mean_radius_km()`; values below are the Terra
target). FOV/near/far moved to `renderer`:
```
MIN_DISTANCE_RADII 0.01 (~63.7)   MAX_DISTANCE_RADII 10 (~63710)   MAX_TILT 80
DEFAULT_DISTANCE_RADII 2 (Terra ~12742)   defaults: lon 0, lat 0, tilt 0   lat clamp +/-89
```

**`src/engine/renderer/mod.rs`**: `MARKER_RADIUS_PX 6`, projection `FOV_Y_DEG 45`, `NEAR_PLANE_RADII 0.01`
(* target radius), `FAR_PLANE_KM 500000` (far-plane *floor*; the actual far
plane is `max(FAR_PLANE_KM, |camera_pos| + 2*radius)`, so a large orbited body
at max zoom-out is never z-clipped). Body impostor:
`IMPOSTOR_BODIES` (planet::ALL + Luna + Terra, the GPU-slot order),
`PLANET_PERSPECTIVE_MIN_ARCSEC 1800` (angular-diameter cutoff: at/above it the
impostor uses the perspective trace, below it orthographic),
`PLANET_QUAD_MARGIN 1.3` (Rust) / `PLANET_QUAD_MARGIN 1.3` (`scene.wgsl`, must
match), `PLANET_MIN_DEPTH 1e-6` (clamps a beyond-far body so it is not
z-clipped), and `MAX_OCCLUDERS 4` (eclipse-occluder slots per body, must match
`scene.wgsl`).
