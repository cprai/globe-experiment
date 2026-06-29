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
  "fix" `HeadlessRenderer` to `Rgba8UnormSrgb`. The stored 8-bit bytes
  already equal the sRGB-encoded on-screen pixels and are written verbatim
  to PNG (no gamma transform on readback).
- **No HDR, no bloom.** LDR only. A real bloom pass is explicitly declined.
- **Present mode: `AutoVsync`.** `get_default_config` picks Mailbox on DX12,
  which causes judder. `AutoVsync` is deliberate.

## Rendering invariants

- **Reversed-Z depth buffer (`Depth32Float`).** Cleared to `0.0` (the far
  plane), `depth_compare: Greater` (nearer = larger depth). It exists so the
  Earth occludes the much more distant Moon (incl. a partial limb). Per-pass
  policy (in `SceneRenderer::new`): the **solid bodies** (Earth surface, Moon)
  write depth + test `Greater`; the **backdrop, atmosphere, markers** neither
  write nor test (`Always`, no write) so they keep their exact draw-order
  layering. Because the atmosphere does not depth-test, `fs_atmosphere` does an
  explicit ray-sphere **Moon occlusion** check (drops the fragment where the ray
  hits the Moon in front of the atmosphere) so the additive glow does not bleed
  over a nearer Moon from a Moon-orbit view. The camera projection is reversed-Z
  (`Camera::view_proj` post-
  multiplies a Z-flip onto `perspective_rh`); the clear value, compare op, and
  projection must all agree or geometry vanishes. egui's overlay pipeline is
  built with `depth_stencil_format: Some(DEPTH_FORMAT)` (depth-off, draws on
  top) so it is compatible with the depth attachment.
- **Draw order: stars -> distant-planet billboards -> Earth surface -> Moon ->
  near-planet meshes -> atmosphere -> markers**, one render pass. The Moon +
  planet meshes (solid bodies, depth write + `Greater`) draw between the surface
  and atmosphere so the depth buffer resolves occlusion before the additive
  atmosphere. Each planet is classified per frame by apparent angular size
  (CPU, in `prepare`, `2*atan(req/distance)` vs `PLANET_BILLBOARD_MAX_ARCSEC`):
  large/near ones draw as **meshes** (`vs_planet`/`fs_planet`); far ones below
  the threshold draw as **billboard impostors** (`vs_planet_billboard`/
  `fs_planet_billboard`) - a camera-facing quad (no vertex buffer) that
  ray-traces the oblate ellipsoid (orthographic / parallel-ray, f32-safe at
  distance), still textured + Lambert-lit. Billboards are depth-off and drawn
  right after the backdrop (they are always the far bodies, so the later opaque
  draws paint over them); they are sorted far-to-near in `prepare`. Both share
  the group-1 bind group; only the solar-system scenario fills the planets.
- **Earth system is gated to Earth/Moon targets.** The Earth surface, atmosphere,
  Moon, and satellite markers draw only when `render_origin == 0` (orbiting the
  Earth or the Moon); the renderer sets a `draw_earth_system` flag. Orbiting a
  planet they would be a far speck and the Earth-centered atmosphere physics is
  meaningless, so they are skipped — only the planets + backdrop draw.
- **Render frame (floating origin): the shader is fully camera-target-local.**
  Every position uniform (`camera_pos`, `moon_pos`, `sun_pos`, the per-planet
  `pos`) is **already relative to the camera target** — the renderer subtracts
  `render_origin` on the CPU, so there is NO `render_origin` uniform and no
  `world - render_origin` in any vertex shader. The orbited body's `pos` is a
  bit-exact zero, so its mesh is drawn in pure local coordinates (the f32-jitter
  fix). For Earth/Moon (`render_origin == 0`) the render frame is the absolute
  frame, so geometry is bit-identical. There is **no `sun_dir`**: every lit pass
  derives its Sun direction from `sun_pos` (`normalize(sun_pos - world_pos)` for
  surfaces, `normalize(sun_pos)` for the backdrop disc).
- **Markers are instanced screen-space overlays** drawn last. CPU occlusion
  per marker (`marker_occluded` in `src/simulation/mod.rs`). No depth test.
- **Mutual eclipse shadows are analytic** (`sun_visibility` in `scene.wgsl`):
  the soft two-disk overlap of the Sun and an occluding sphere. The Moon
  shadows the Earth in `fs_main` (solar-eclipse spot); the Earth shadows the
  Moon in `fs_moon` (lunar-eclipse coppery glow). No shadow maps.
- **Idle = zero GPU work.** Never add an unconditional vsync loop.
  `ControlFlow::Wait` + targeted `request_redraw`. The clock starts playing,
  so the app is non-idle from launch until paused.
- **Terminator / night-side darkening must use the GEOMETRIC normal**
  (`dot(n_geo, sun)`), never the bump-mapped normal `n`. Bump detail on the
  day/night edge speckles it.
- **Star map and Sun are ephemeris-driven.** Sun *position* from JPL DE440
  (uploaded as `sun_pos`, render-frame); `star_rot_inv = P * R_itrf2gcrf * P^T`.
  Do not replace with a sun-attached rotation. The star matrix additionally folds
  in a static galactic->equatorial offset (`star_tex_rot_inv`), because the star
  texture is drawn in galactic coordinates; see `simulation.md`.
- **Backdrop anchoring**: star lookup is a function of the camera-relative view
  direction, not position on the celestial sphere. Changing the *star* anchoring
  reintroduces parallax between sun and stars (a fixed bug). The star shell is
  **centered on the camera** (`vs_stars`: `camera_pos + normal *
  STARS_RADIUS_KM`), so it always encloses the eye - required for non-Earth
  targets (orbiting the Moon is ~384,000 km outside an origin-centered shell;
  the Sun and half the sky vanished). The **sun disc** (`fs_stars`) is drawn in
  the orbited body's direction to the Sun (`normalize(sun_pos)`, render-frame) so
  it agrees with each planet's terminator (`fs_planet` lights from the same Sun
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
  unlit hemisphere ~20% **brighter** than full daylight, so Earth reads
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

## Live constant snapshot (2026-06-18 — verify against source)

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
MIE_G 0.8                  SUN_INTENSITY 12.0
STARS_RADIUS_KM 222985.0   STARS_BRIGHTNESS 0.8
SUN_ANGULAR_RADIUS 0.012   SUN_GLOW_RADIUS 0.12
SUN_GLOW_STRENGTH 0.5      SUN_COLOR (1.0, 0.96, 0.9)
MARKER_FILL (1.0, 0.25, 0.2)  MARKER_RING (1.0, 1.0, 1.0)
MOON_AMBIENT 0.02          MOON_ECLIPSE_GLOW (0.06, 0.012, 0.004)
terminator: smoothstep(-0.12, 0.18, cos_sun)
```

**`src/renderer/mod.rs`** (depth/eclipse): `DEPTH_FORMAT Depth32Float`,
`SUN_ANGULAR_RADIUS_RAD 0.004652` (eclipse penumbra; distinct from the star
pass's `SUN_ANGULAR_RADIUS` disc-size cheat).

**`build.rs mod atmosphere`**:
```
RAYLEIGH_SCATTERING [5.802, 13.558, 33.1]e-3   RAYLEIGH_SCALE_HEIGHT 8.0
MIE_SCATTERING 3.996e-3   MIE_EXTINCTION 4.40e-3   MIE_SCALE_HEIGHT 1.2
OZONE_ABSORPTION [0.650, 1.881, 0.085]e-3       (tent peak 25 km, +/-15)
TRANSMITTANCE 256x64 / 40 steps   INSCATTER 256x128 / 32 steps
```

**`src/application/input.rs`**:
```
FLICK_SPEED 50   STOP_SPEED 15   HALF_LIFE 0.3   FLICK_TIMEOUT 0.1
ZOOM_HALF_LIFE_MIN 0.01   ZOOM_HALF_LIFE_MAX 0.1   WHEEL_GAP_CAP 0.25
ZOOM_COAST_HALF_LIFE 0.15   ZOOM_STOP_RATE 0.1
```

**`src/earth.rs`** (WGS84 + dynamics):
```
SEMI_MAJOR_AXIS_KM 6378.137   INVERSE_FLATTENING 298.257223563
SEMI_MINOR_AXIS_KM ~6356.752  ECCENTRICITY_SQ ~0.00669438
MEAN_RADIUS_KM ~6371.0088     GRAVITATIONAL_PARAMETER_KM3_S2 398600.4418
ANGULAR_VELOCITY_RAD_S 7.292115e-5
```

**`src/application/camera.rs`** (limits are radius *ratios* `*_RADII`, scaled by
the orbit target's `mean_radius_km()`; values below are the Earth target):
```
FOV_Y 45 deg   MIN_DISTANCE_RADII 0.01 (~63.7)   MAX_DISTANCE_RADII 10 (~63710)
NEAR_PLANE_RADII 0.01 (~63.7)   FAR_PLANE 500000 (fixed km)   MAX_TILT 80
DEFAULT_DISTANCE_RADII 2 (Earth ~12742)   defaults: lon 0, lat 0, tilt 0   lat clamp +/-89
```

**`src/renderer/mod.rs`**: `STACKS 64`, `SLICES 128`, `MARKER_RADIUS_PX 6`,
`PLANET_BILLBOARD_MAX_ARCSEC 1800` (angular-diameter cutoff: below it a planet
draws as a billboard impostor, above it as a mesh). Billboard shader consts in
`scene.wgsl`: `PLANET_BILLBOARD_SHELL_KM = STARS_RADIUS_KM`,
`PLANET_BILLBOARD_MARGIN 1.15`.
