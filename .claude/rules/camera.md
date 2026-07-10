---
paths:
  - "src/engine/camera/**/*.rs"
---

# Camera rules

The `engine::camera` module is a directory: `mod.rs` holds the
**`CameraControl` + `CameraView` trait pair** (implemented by every scene;
`CameraControl` = the no-op-defaulted input methods + `tick` + `cursor_hint`,
`CameraView` = `frame_state`) + the winit-free input vocabulary
(`PointerButton`/`ScrollDelta`/`CursorHint`); `ptz.rs` holds **`PtzCamera`**,
the interactive pan/tilt/zoom implementation scenes embed, plus
**`ScenePtzCamera`** — the three-accessor hookup trait (`camera()`/
`camera_mut()`/`camera_target()`, the `SceneClock` pattern) whose blanket
`impl<S: ScenePtzCamera> CameraControl for S` forwards every input event
into the embedded camera, so a scene writes no forwarding block. Every
scene implements it — the `*_py` wrappers keep their camera as a plain
wrapper field outside the scene pyclass (a script has no camera surface)
precisely so they can; a scene that must diverge implements
`CameraControl` directly instead.
The rig rules below are about `PtzCamera`; input rules live in `input.md`.

## Inertial (star-fixed) frame

**The camera is in the inertial (star-fixed) frame.** The rig is built in the
celestial frame and rotated into the world by:

```
celestial_to_world = star_rot_inv.transpose()
```

Each scene's `CameraView::frame_state` derives `celestial_to_world =
star_rot_inv.transpose()` from the celestial sphere it evaluates **on the
spot** at the frame's clock instant (`CelestialSphere::at(&now)` — a pure
function of time; no scene stores a sphere), resolves its
scene-owned camera target (reframing the camera on a genuine body switch),
and calls `camera.world_rig(&target, celestial, c2w)` (which returns the eye,
look-at point, and up in the render frame; the renderer rebuilds the
projection from them). The camera takes the sphere so it can resolve the
target's moving center (see "Camera target" below). The application never
touches the sphere; it only consumes the finished `RenderState`.

Because `star_rot_inv * celestial_to_world = I`, a rig held constant in the
celestial frame yields a constant star lookup direction — **stars are locked
to the camera while the ECEF Terra spins underneath**.

The rig is built from the **equatorial** `star_rot_inv` (the GCRF frame). The
shader samples the star texture with a *different* matrix, `star_tex_rot_inv`
= a static galactic->equatorial offset times `star_rot_inv` (the texture is
drawn in galactic coordinates — see `simulation.md`). The offset is constant,
so the camera stays inertial; keeping the rig on the equatorial frame means
the re-orientation does not move existing scenes' framing.

`PtzCamera.longitude` / `PtzCamera.latitude` are **inertial directions**, not
geographic coordinates. Do not move the camera into the ECEF/world frame.

`PtzCamera::looking_toward(&target, star_rot_inv, world_look, distance)` builds
a camera that orbits `target` and whose look axis points along a
**world-frame** direction (it maps the direction back through `star_rot_inv`
into the inertial rig). A scene seeds its `camera: PtzCamera` field with it
in `new()` to frame an event on launch (solar eclipse: Terra target aimed at
`-sol_dir`; lunar eclipse: Luna target aimed at Luna's center, so it launches
orbiting the Luna); `PtzCamera::default()` still gives the whole-Terra view.
(`ApplicationState::with_camera` no longer exists - the scene owns the
camera.)

## Render frame (floating origin) — all rendering is camera-target-local

A planet sits millions-to-billions of km from Terra — past f32 precision in
world-km, where forming an absolute position jitters/facets the body and the
camera swims. So **everything the GPU sees is in the render frame: positions
relative to the camera target's center** (`CameraTarget::render_origin()` — the
planet's center, or Terra's center for Terra/Luna). The GPU never handles an
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
  coordinates, which is what kills the jitter. (The renderer re-derives the
  same `CelestialSphere` the camera target's center came from - the *same* f64
  value - so the subtraction cancels exactly.)
- `PtzCamera::world_rig(&target, celestial, c2w)` builds the rig via
  `world_frame_relative`:
  `(center - render_origin) + c2w*offset`, where `center` /`render_origin` are
  resolved from the passed `&CelestialSphere` — it never forms the absolute eye
  (`absolute_eye - render_origin` would cancel catastrophically, snapping the
  view translation to ~hundreds of km). For the orbited body
  `center == render_origin`, so the rig is just `c2w*offset`. The renderer then
  builds `view_proj` from the rig's eye + look-at point + up
  (`renderer::view_proj_reversed_z`).
- For Terra/Luna (`render_origin` = Terra's center) the render frame is the
  Terra-centered frame (the old geocentric world frame - the `CelestialSphere`
  is now heliocentric, but subtracting Terra's center undoes that). The whole
  rig is **f64** (`DVec3`/`DQuat`; the `PtzCamera` pose fields themselves are
  f64) and the renderer builds view/projection in `DMat4`, casting to f32 only
  at uniform upload. Keep passing the look-at *point* (not a re-normalized
  forward vector) so the renderer's `look_at_rh` reproduces exactly the view
  the camera implied.

## Camera target (orbit Terra, Luna, a planet, or a free point)

The camera orbits a **`CameraTarget`** (an enum `Body(CelestialBody) |
Coordinate(DVec3)`, defined in `scene`, a sanctioned `scene`->
`application` data edge like `RenderState`). It is a pure **identity**: it does
NOT store the body's center. The position-dependent accessors take the sphere
(`center_world(&celestial)`, `render_origin(&celestial)`) and look the center up
from the ephemeris; the static ones (`mean_radius_km`, `surface_position`,
`geodetic_normal`) delegate through the `CelestialBody` identity to `planet`
(the shared per-body table, Terra's row included) with no sphere. For `Body`, the identity is `TerraSystem(Terra)`,
`TerraSystem(Luna)`, or a planet; orbiting Luna is `TerraSystem(Luna)`. The
`Coordinate` variant orbits a free world point with synthetic geometry (a
Terra-radius scale + a center look-at anchor) — future-proof scaffolding, not
wired into any scene yet.
**The scene owns the target**: each scene struct holds a `camera_target:
CameraTarget` field (identity only); `PtzCamera` stores NO target. Every
camera call that scales by or centers on the orbited body takes a
`&CameraTarget` parameter (`world_rig`, `reframe`, `pointer_move` / `scroll`
/ `tick`, `new` / `looking_toward`), so the rig state and the orbit subject
cannot drift apart across the scene/camera boundary. The rig is built by
`world_frame_relative(&target, &celestial, c2w)` in the render frame (see
above): for Terra/Luna it equals the absolute rig; for a planet (or
coordinate) it is the small local offset. The camera stays star-fixed while
tracking the body's moving ephemeris position (re-resolved from the sphere
on every `world_rig` call). `same_kind` compares the `CelestialBody`
identity (two planet targets are equal only when the *same* planet, so
cycling Mars->Jupiter reframes; two coordinates always match).
The surface anchor and the distance/near/pan limits scale by
`target.mean_radius_km()`, so pan/tilt/zoom feel is the same fraction of
whichever body is orbited.

Each frame the scene's `CameraView::frame_state` resolves its target — the
fixed Terra `camera_target` for the satellite scenes; the eclipse /
solar-system scenes refresh `camera_target` from their
`TargetSelector`/`BodySelector`, driven by the panel keys. On a genuine
**body switch** (detected scene-side via
`!self.camera_target.same_kind(&target)`) the scene calls
`PtzCamera::reframe(&target, &celestial, c2w)` — distance = body default,
tilt 0, re-aimed at the target's center resolved from the sphere, and any
in-flight zoom/flick dropped (it targets the old body's scale) — before
storing the new target in `camera_target`. A same-body frame needs no camera
call at all (the moving center is resolved inside `world_rig`). The
`headless` binary picks
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
(every body's impostor, `fs_planet`), and the backdrop Sol disc
(`fs_stars`) uses `normalize(sol_pos)` (the camera target's own direction to the
Sol). Sol is a solar-system object, so from a distant planet it is in a
wholly different direction than from Terra (e.g. ~160 deg away at Jupiter), and
this is what makes the disc agree with that planet's terminator. It is
parallax-free under local orbit/zoom (`sol_pos` is constant while orbiting one
body). For Terra/Luna it is the Terra->Sol direction as before (< 1 LSB diff,
position-derived vs the old precomputed unit `sol_dir`).

The star backdrop (`vs_stars`/`fs_stars`) is a **full-screen quad** whose
fragment shader reconstructs the per-pixel view direction from NDC via
`inv_view_proj` — trivially camera-centered and independent of camera
position. This is load-bearing for **non-Terra targets**: any origin-anchored
shell would exclude a Luna-orbit eye (~384,000 km from the Terra origin),
which dropped half the sky and Sol.

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
(`<radii>` consts on `PtzCamera` in `src/engine/camera/ptz.rs`, multiplied by
`target.mean_radius_km()` in the helper methods
`min_distance`/`max_distance`/`default_distance`, which take the
`&CameraTarget` the scene passes in), so the old tuned Terra
feel is preserved and Luna gets the same feel at its own scale:
- `MIN_DISTANCE_RADII = 0.01` (Terra ~63.7 km, Luna ~17.4 km)
- `MAX_DISTANCE_RADII = 10.0` (Terra ~63710 km, Luna ~17374 km)
- `MAX_TILT = 80 deg`
- `DEFAULT_DISTANCE_RADII = 2.0` (Terra default ~12742 km); lat clamp +/- 89 deg.
  `clamp_distance` is private and takes the target (the limits depend on it)
  - external construction clamps via `PtzCamera::new` / `looking_toward`.

The **projection** constants live in `renderer` (the renderer rebuilds
`view_proj` from the camera rig via `view_proj_reversed_z`):
- `renderer::FOV_Y_DEG = 45 deg` (the camera's pan math reads it too)
- `renderer::NEAR_PLANE_RADII = 0.01`, scaled by the target's mean radius
- `renderer::FAR_PLANE_KM = 500_000` — the **floor** of the far plane, NOT a
  fixed far plane. It covers the Terra-Luna system (Luna at apogee ~406,700 km +
  camera distance; orbiting Luna, Terra ~384,400 km). But `prepare` computes
  the actual far plane as
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
