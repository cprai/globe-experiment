---
paths:
  - "shaders/globe.wgsl"
---

# Shader rules & invariants

## Surface format (load-bearing)

- **Surface format: non-sRGB.** `Gfx::init` picks `find(|f| !f.is_srgb())`.
  `get_default_config` defaults to sRGB format — this override is deliberate.
  Every look-tuning constant in `globe.wgsl` is calibrated to the non-sRGB
  surface. Do not switch to sRGB.
- **Headless render target must also be non-sRGB** (`Rgba8Unorm`). Do not
  "fix" `HeadlessRenderer` to `Rgba8UnormSrgb`. The stored 8-bit bytes
  already equal the sRGB-encoded on-screen pixels and are written verbatim
  to PNG (no gamma transform on readback).
- **No HDR, no bloom.** LDR only. A real bloom pass is explicitly declined.
- **Present mode: `AutoVsync`.** `get_default_config` picks Mailbox on DX12,
  which causes judder. `AutoVsync` is deliberate.

## Rendering invariants

- **No depth buffer.** Draw order handles all occlusion: stars -> surface ->
  atmosphere -> markers, one render pass. Convex-sphere assumption. Do not
  add geometry that breaks this without also adding a depth attachment.
- **Markers are instanced screen-space overlays** drawn last. CPU occlusion
  per marker (`marker_occluded` in `src/simulation/mod.rs`). No depth.
- **Idle = zero GPU work.** Never add an unconditional vsync loop.
  `ControlFlow::Wait` + targeted `request_redraw`. The clock starts playing,
  so the app is non-idle from launch until paused.
- **Terminator / night-side darkening must use the GEOMETRIC normal**
  (`dot(n_geo, sun)`), never the bump-mapped normal `n`. Bump detail on the
  day/night edge speckles it.
- **Star map and Sun are ephemeris-driven.** `sun_dir` from JPL DE440;
  `star_rot_inv = P * R_itrf2gcrf * P^T`. Do not replace with a sun-attached
  rotation.
- **Backdrop anchoring**: star lookup and sun disc are functions of the
  camera-relative view direction (`world_pos - camera_pos`), not position on
  the celestial sphere. Changing this reintroduces parallax between sun and
  stars (a fixed bug).

## Look-tuning discipline

- **Look-tuning constants drift between sessions.** Always read `globe.wgsl`
  for live values; the snapshot below is dated.
- **Tune and feel-test on a native Windows release build.** The WSLg dev
  environment cannot validate exact colors or interaction feel.

## City light design decisions (intentional, do not revert)

Two live constants depart from the original phase-3 plan:
- **`NIGHT_DARKNESS = 1.2`** (plan was ~0.02): because
  `night_factor = mix(NIGHT_DARKNESS, 1.0, daylight)`, a value > 1 makes the
  unlit hemisphere ~20% **brighter** than full daylight, so the globe reads
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

**`shaders/globe.wgsl`**:
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
terminator: smoothstep(-0.12, 0.18, cos_sun)
```

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

**`src/application/camera.rs`** (km; `<radii> * MEAN_RADIUS_KM`):
```
FOV_Y 45 deg   MIN_DISTANCE ~63.7   MAX_DISTANCE ~63710
NEAR_PLANE ~63.7   FAR_PLANE ~318550   MAX_TILT 80
defaults: lon 0, lat 0, distance ~12742, tilt 0   lat clamp +/-89
```

**`src/renderer/mod.rs`**: `STACKS 64`, `SLICES 128`, `MARKER_RADIUS_PX 6`.
