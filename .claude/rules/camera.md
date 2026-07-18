---
paths:
  - "crates/engine/src/camera/**/*.rs"
---

# Camera rules

`mod.rs`: the `CameraControl`/`CameraView` trait pair + input vocabulary.
`ptz.rs`: `PtzCamera` (rig + all input/animation state) + `ScenePtzCamera`
(three accessors, supplied per scene by `#[derive(ScenePtzCamera)]`; blanket
impls supply `CameraControl`, and `CameraView`/`frame_state` for scenes with
the standard clock + body traits — scenes implement neither by hand). Input
rules live in `input.md`.

## Inertial (star-fixed) frame

The rig is built in the celestial frame and rotated into the world by
`celestial_to_world = star_rot_inv.transpose()`. Because `star_rot_inv *
celestial_to_world = I`, a rig held constant in the celestial frame yields a
constant star lookup direction — **stars are locked to the camera while the
ECEF Terra spins underneath**.

- The rig uses the **equatorial** `star_rot_inv` (GCRF). The shader's star
  texture matrix is different (`star_tex_rot_inv`, adds a static
  galactic->equatorial offset — see `simulation.md`); the offset is constant,
  so the camera stays inertial.
- `PtzCamera.longitude`/`latitude` are **inertial directions**, not
  geography. Do not move the camera into the ECEF/world frame.
- `looking_toward` seeds a launch framing whose look axis is a world-frame
  direction (it maps the direction back through `star_rot_inv`).

## Render frame (floating origin) — canonical statement

Bodies sit millions-to-billions of km out, past f32 precision in world-km:
forming an absolute f32 position jitters/facets the body and the camera
swims. So **everything the GPU sees is relative to the camera target's
center** (`CameraTarget::render_origin`):

- The renderer subtracts the origin on the **CPU, in f64**, from every
  absolute body position before upload. There is no `render_origin` uniform —
  the shader is purely local.
- The orbited body's uploaded position is a **bit-exact zero** (the renderer
  re-derives the same f64 `CelestialSphere` the target's center came from, so
  the subtraction cancels exactly). That is what kills the jitter.
- `world_rig` builds the eye as `(center - render_origin) + c2w*offset` —
  never `absolute_eye - render_origin`, which cancels catastrophically.
- For a Terra/Luna target the render frame is the Terra-centered (old
  geocentric) frame: the `CelestialSphere` is heliocentric, but subtracting
  Terra's center undoes that.
- Keep passing the look-at *point* (not a re-normalized forward vector) so
  the renderer's `look_at_rh` reproduces the exact implied view.

## Camera target

`CameraTarget` is a pure **identity** (`Body(CelestialBody)` |
`Coordinate(DVec3)`); it stores no center — position-dependent accessors take
the sphere. The scene owns it; every camera call that scales by or centers on
the orbited body takes `&CameraTarget`, so rig state and orbit subject cannot
drift apart. On a genuine body switch (`!same_kind`) the scene calls
`PtzCamera::reframe` (default distance, tilt 0, re-aimed, in-flight animation
dropped — it targets the old body's scale) before storing the new target.
The `Coordinate` variant is future-proof scaffolding, not wired into any
scene yet.

Distance/default limits are **ratios of the target's mean radius** (`*_RADII`
consts on `PtzCamera`), so pan/tilt/zoom feel is the same fraction of
whichever body is orbited. Projection consts (FOV, near, far) live in
`renderer`; the far plane is a floor the renderer grows to enclose a large
orbited body (see `renderer.md`).

## Backdrop anchoring

Star lookup is a function of the **camera-relative view direction**, never
absolute position (the backdrop is at infinity — no parallax, no zoom
dependence). Changing the star anchoring reintroduces parallax between Sol
and stars (a fixed bug). There is **no Earth-fixed `sol_dir`** anywhere:
every lit pass derives its Sol direction from the render-frame `sol_pos`
(`normalize(sol_pos - world_pos)` for surfaces, `normalize(sol_pos)` for the
backdrop disc), which is what makes the Sol disc agree with each planet's
terminator from any vantage.
