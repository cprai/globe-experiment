---
paths:
  - "src/engine/camera.rs"
---

# Camera rules

## Inertial (star-fixed) frame

**Camera is in the inertial (star-fixed) frame.** The rig is built in the
celestial frame and rotated into the world by:

```
celestial_to_world = star_rot_inv.transpose()
```

`ApplicationState::redraw` reads the frame's sphere from
`Simulation::celestial()`, derives `celestial_to_world =
star_rot_inv.transpose()`, and calls `camera.world_rig(celestial, c2w)` (which
returns the eye, look-at point, and up in the render frame; the renderer
rebuilds the projection from them). The camera takes the sphere so it can
resolve its target's moving center (see "Camera target" below).

Because `star_rot_inv * celestial_to_world = I`, a rig held constant in the
celestial frame yields a constant star lookup direction — **stars are locked
to the camera while the ECEF Terra spins underneath**.

The rig is built from the **equatorial** `star_rot_inv` (the GCRF frame). The
shader samples the star texture with a *different* matrix, `star_tex_rot_inv`
= a static galactic->equatorial offset times `star_rot_inv` (the texture is
drawn in galactic coordinates — see `simulation.md`). The offset is constant,
so the camera stays inertial; keeping the rig on the equatorial frame means
the re-orientation does not move existing scenarios' framing.

`Camera.longitude` / `Camera.latitude` are **inertial directions**, not
geographic coordinates. Do not move the camera into the ECEF/world frame.

`Camera::looking_toward(target, star_rot_inv, world_look, distance)` builds a
camera that orbits `target` and whose look axis points along a **world-frame**
direction (it maps the direction back through `star_rot_inv` into the inertial
rig). Scenarios use it with `ApplicationState::with_camera(sim, camera)` to frame
an event on launch (solar eclipse: Terra target aimed at `-sol_dir`; lunar
eclipse: Luna target aimed at Luna's center, so it launches orbiting the
Luna); the default constructor `::new` still gives the whole-Terra view.

## Render frame (floating origin) — all rendering is camera-target-local

A planet sits millions-to-billions of km from Terra — past f32 precision in
world-km, where forming an absolute position jitters/facets the body and the
camera swims. So **everything the GPU sees is in the render frame: positions
relative to the camera target's center** (`CameraTarget::render_origin()` — the
planet's center, or `Vec3::ZERO` for Terra/Luna). The GPU never handles an
absolute world position.

- The renderer computes the origin as
  `RenderState.camera_target.render_origin(&celestial)` (the sphere it derived
  from `RenderState.time`) and subtracts it on the **CPU** (in `prepare`) from
  every absolute body
  position (`sol_pos_world`, and each `BodyState.placement.pos_world` — Luna and
  each planet, derived from `CelestialSphere::at(time)`) before upload.
  `render_origin` is **not** a shader uniform — the shader is purely local.
- The orbited body's center IS the origin, so its uploaded position is a
  **bit-exact zero** (`pos - origin == 0`): it is drawn in pure local
  coordinates, which is what kills the jitter. (Even the f64->f32 ephemeris
  rounding cancels here, since the renderer re-derives the same `CelestialSphere`
  the camera target's center came from, the *same* f32 value.)
- `Camera::world_rig(celestial, c2w)` builds the rig via `world_frame_relative`:
  `(center - render_origin) + c2w*offset`, where `center` /`render_origin` are
  resolved from the passed `&CelestialSphere` — it never forms the absolute eye
  (`absolute_eye - render_origin` would cancel catastrophically, snapping the
  view translation to ~hundreds of km). For the orbited body
  `center == render_origin`, so the rig is just `c2w*offset`. The renderer then
  builds `view_proj` from the rig's eye + look-at point + up
  (`renderer::view_proj_reversed_z`).
- For Terra/Luna (`render_origin == 0`) the render frame **is** the absolute
  frame, so the camera geometry is **bit-identical** to the pre-planet renderer
  (verified: AE=0 headless A/B for Terra/Luna/eclipse). Passing the look-at
  *point* (not a re-normalized forward vector) is what preserves the bit
  identity. (Lighting differs by < 1 LSB: every pass derives the Sol direction
  from Sol *position* rather than a precomputed `sol_dir`.)

## Camera target (orbit Terra, Luna, a planet, or a free point)

The camera orbits a **`CameraTarget`** (an enum `Body(CelestialBody) |
Coordinate(Vec3)`, defined in `simulation`, a sanctioned `simulation`->
`application` data edge like `RenderState`). It is a pure **identity**: it does
NOT store the body's center. The position-dependent accessors take the sphere
(`center_world(&celestial)`, `render_origin(&celestial)`) and look the center up
from the ephemeris; the static ones (`mean_radius_km`, `surface_position`,
`geodetic_normal`) delegate through the `CelestialBody` identity to `terra` /
`luna` / `planet` with no sphere. For `Body`, the identity is `TerraSystem(Terra)`,
`TerraSystem(Luna)`, or a planet; orbiting Luna is `TerraSystem(Luna)`. The
`Coordinate` variant orbits a free world point with synthetic geometry (a
Terra-radius scale + a center look-at anchor) — future-proof scaffolding, not
wired into any scenario yet.
`Camera` holds a `target` field (identity only). The rig is built by
`world_frame_relative(&celestial, c2w)` in the render frame (see above): for
Terra/Luna it equals the absolute rig; for a planet (or coordinate) it is the
small local offset. The camera stays star-fixed while tracking the body's moving
ephemeris position (re-resolved from the sphere each frame). `same_kind` compares
the `CelestialBody` identity (two planet targets are equal only when the *same*
planet, so cycling Mars->Jupiter reframes; two coordinates always match);
`retarget(target, &celestial, c2w)` re-aims at any off-origin center.
The surface anchor and the distance/near/pan limits scale by
`target.mean_radius_km()`, so pan/tilt/zoom feel is the same fraction of
whichever body is orbited.

Each frame `ApplicationState::redraw` calls `Camera::retarget(target, &celestial,
c2w)` with `Simulation::camera_target()` (defaults to Terra; the eclipse
scenarios override it from a `TargetSelector` driven by the panel's Terra / Luna
keys). On a genuine **body switch** it reframes (distance = body default, tilt 0,
re-aimed at the target's center resolved from the sphere) and returns `true`, and
the application calls `Controller::reset_animation()` to cancel any in-flight
zoom/flick (which targets the old body's scale). The `headless` binary picks
the body directly: the `--scene` `camera.target` field is `"terra"` (default),
`"luna"`, or a planet (a center-free `CameraTargetSpec` in `src/headless.rs`;
the center is resolved from the ephemeris at render time).

## Backdrop anchoring

Star lookup is a function of the **camera-relative view direction**, not
absolute position on the celestial sphere (the stars are at infinity — no
parallax, no zoom dependence). Changing the *star* anchoring reintroduces
parallax between Sol and stars (a fixed bug).

**There is no Earth-fixed `sol_dir` in the render path.** Every lit pass derives
its Sol direction from `uniforms.sol_pos` (Sol in the render frame =
relative to the camera target): surfaces use `normalize(sol_pos - world_pos)`
(Terra `fs_main`; every impostor body incl. Luna `fs_planet`), and the backdrop Sol disc
(`fs_stars`) uses `normalize(sol_pos)` (the camera target's own direction to the
Sol). Sol is a solar-system object, so from a distant planet it is in a
wholly different direction than from Terra (e.g. ~160 deg away at Jupiter), and
this is what makes the disc agree with that planet's terminator. It is
parallax-free under local orbit/zoom (`sol_pos` is constant while orbiting one
body). For Terra/Luna it is the Terra->Sol direction as before (< 1 LSB diff,
position-derived vs the old precomputed unit `sol_dir`).

The star shell (`vs_stars`) is **centered on the camera** (`world = camera_pos +
normal * STARS_RADIUS_KM`), so it always encloses the eye and the camera-relative
direction is exactly the vertex normal — independent of camera position. This is
load-bearing for **non-Terra targets**: orbiting Luna puts the camera
~384,000 km from the Terra origin, far outside an origin-centered shell, which
dropped half the sky and Sol. For a Terra orbit the eye was always inside
the old origin-centered shell, where both formulations give the same per-pixel
direction, so the change is a no-op there (a re-render diff is < 1 LSB on a
handful of pixels).

## Luna occludes Terra's atmosphere

The atmosphere pass does not depth-test (it layers aerial perspective over the
Terra's own near disc, whose written depth is closer than the far-side
atmosphere shell). So `fs_atmosphere` explicitly drops fragments where the view
ray meets the **Luna** in front of the atmosphere entry (`ray_sphere` against the
Luna, `luna.x < shell.x`); otherwise the additive Terra-atmosphere glow bleeds
over the nearer Luna from a Luna-orbit view. Luna is ~384,000 km out, so from
a Terra orbit this never triggers.

## Associated constants (km)

The distance/default limits are **ratios of the target body's mean radius**
(`<radii>` consts on `Camera`, multiplied by `target.mean_radius_km()` in the
instance methods `min_distance`/`max_distance`/`default_distance`), so the old
tuned Terra feel is preserved and Luna gets the same feel at its own scale:
- `MIN_DISTANCE_RADII = 0.01` (Terra ~63.7 km, Luna ~17.4 km)
- `MAX_DISTANCE_RADII = 10.0` (Terra ~63710 km, Luna ~17374 km)
- `MAX_TILT = 80 deg`
- `DEFAULT_DISTANCE_RADII = 2.0` (Terra default ~12742 km); lat clamp +/- 89 deg.
  `clamp_distance` is now an **instance** method (limits depend on the target).

The **projection** constants live in `renderer` (the renderer rebuilds
`view_proj` from the camera rig via `view_proj_reversed_z`):
- `renderer::FOV_Y_DEG = 45 deg` (the camera's pan math reads it too)
- `renderer::NEAR_PLANE_RADII = 0.01`, scaled by the target's mean radius
- `renderer::FAR_PLANE_KM = 500_000` — the **floor** of the far plane, NOT a
  fixed far plane. It covers the Terra-Luna system (Luna at apogee ~406,700 km +
  camera distance; orbiting Luna, Terra ~384,400 km) and the camera-centered
  star shell (222,985 km). But `prepare` computes the actual far plane as
  `max(FAR_PLANE_KM, |camera_pos| + 2*radius)` so it always encloses the orbited
  body even when that body is large: a gas giant at max zoom-out has an
  eye-to-center of ~770,000 km (Jupiter), well past 500,000 km, and a fixed far
  plane would z-clip the whole disc away. Terra/Luna stay at exactly 500,000
  (their `|camera_pos| + 2*radius` is smaller), so their output is unchanged.
  A non-orbited planet sits beyond even the scaled far plane (billions of km);
  its impostor depth is clamped (`PLANET_MIN_DEPTH`) so it is not z-clipped,
  though it is sub-pixel in practice.

`view_proj_reversed_z` uses a **reversed-Z** projection: it post-multiplies a
Z-flip matrix onto `Mat4::perspective_rh` so the near plane maps to depth 1 and
the far plane to 0. This is paired with the renderer's `Depth32Float` buffer
cleared to `0.0` and a `Greater` depth test (see `shader.md`); across the huge
~64 km -> 500,000 km near/far span a forward-Z float buffer would have almost
no precision near Luna, so reversed-Z is load-bearing for the Terra-occludes-
Luna test. The clear value, compare op, and projection sign must all agree.
