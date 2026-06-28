---
paths:
  - "src/application/camera.rs"
---

# Camera rules

## Inertial (star-fixed) frame

**Camera is in the inertial (star-fixed) frame.** The rig is built in the
celestial frame and rotated into the world by:

```
celestial_to_world = star_rot_inv.transpose()
```

This is `SimulationState::celestial_to_world()`, applied in
`ApplicationState::redraw` via `camera.view_proj(aspect, c2w)` and
`camera.eye(c2w)`.

Because `star_rot_inv * celestial_to_world = I`, a rig held constant in the
celestial frame yields a constant star lookup direction — **stars are locked
to the camera while the ECEF globe spins underneath**.

The rig is built from the **equatorial** `star_rot_inv` (the GCRF frame). The
shader samples the star texture with a *different* matrix, `star_tex_rot_inv`
= a static galactic->equatorial offset times `star_rot_inv` (the texture is
drawn in galactic coordinates — see `simulation.md`). The offset is constant,
so the camera stays inertial; keeping the rig on the equatorial frame means
the re-orientation does not move existing scenarios' framing.

`Camera.longitude` / `Camera.latitude` are **inertial directions**, not
geographic coordinates. Do not move the camera into the ECEF/world frame.

`Camera::looking_toward(star_rot_inv, world_look, distance)` builds a camera
whose look axis points along a **world-frame** direction (it maps the direction
back through `star_rot_inv` into the inertial rig). Scenarios use it with
`ApplicationState::with_camera(sim, camera)` to frame an event on launch (e.g.
the eclipse scenarios aim at `-sun_dir` / `moon_pos_world`); the default
constructor `::new` still gives the full-globe view.

## Backdrop anchoring

Star lookup and sun disc are functions of the **camera-relative view
direction** (`world_pos - camera_pos`), not absolute position on the celestial
sphere. This keeps sun and stars locked at any orbit/zoom (the backdrop is at
infinity — no parallax, no zoom dependence). Changing this reintroduces
parallax between sun and stars (a fixed bug).

## Associated constants (km)

Constants are written as `<radii> * earth::MEAN_RADIUS_KM` so the old tuned
feel is preserved:
- `FOV_Y = 45 deg`
- `MIN_DISTANCE ~63.7 km` (0.01 * R)
- `MAX_DISTANCE ~63710 km` (10 * R)
- `NEAR_PLANE ~63.7 km` (0.01 * R); `FAR_PLANE = 500_000 km` (a fixed value,
  NOT a multiple of R) — it must enclose the Moon at lunar apogee (~406,700 km)
  plus the camera distance; the star shell (222,985 km) is well inside it.
- `MAX_TILT = 80 deg`
- Default distance `~12742 km` (2 * R); lat clamp +/- 89 deg

`view_proj` uses a **reversed-Z** projection: it post-multiplies a Z-flip
matrix onto `Mat4::perspective_rh` so the near plane maps to depth 1 and the
far plane to 0. This is paired with the renderer's `Depth32Float` buffer
cleared to `0.0` and a `Greater` depth test (see `shader.md`); across the huge
~64 km -> 500,000 km near/far span a forward-Z float buffer would have almost
no precision near the Moon, so reversed-Z is load-bearing for the Earth-occludes-
Moon test. The clear value, compare op, and projection sign must all agree.
