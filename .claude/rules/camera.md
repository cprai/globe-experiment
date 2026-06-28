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

`Camera::looking_toward(target, star_rot_inv, world_look, distance)` builds a
camera that orbits `target` and whose look axis points along a **world-frame**
direction (it maps the direction back through `star_rot_inv` into the inertial
rig). Scenarios use it with `ApplicationState::with_camera(sim, camera)` to frame
an event on launch (solar eclipse: Earth target aimed at `-sun_dir`; lunar
eclipse: Moon target aimed at `moon_pos_world`, so it launches orbiting the
Moon); the default constructor `::new` still gives the full-globe Earth view.

## Camera target (orbit Earth or Moon)

The camera orbits a **`CameraTarget`** (`{ Earth, Moon { center_world } }`,
defined in `simulation`, plain data + geometry accessors delegating to `earth` /
`moon` — a sanctioned `simulation`->`application` data edge like `RenderState`).
`Camera` holds a `target` field. The rig math is unchanged for Earth (center =
origin, output bit-identical); for the Moon `world_frame` offsets the rig by the
body center (`eye_world = center + c2w*eye_offset`), so the camera orbits the
Moon and stays star-fixed while tracking the Moon's moving ephemeris position.
The surface anchor and the distance/near/pan limits scale by
`target.mean_radius_km()`, so pan/tilt/zoom feel is the same fraction of
whichever body is orbited.

Each frame `ApplicationState::redraw` calls `Camera::retarget(target, c2w)` with
`Simulation::camera_target()` (defaults to Earth; the eclipse scenarios override
it from a `TargetSelector` driven by the panel's EARTH / MOON keys). `retarget`
always refreshes the Moon center; on a genuine **body switch** it reframes
(distance = body default, tilt 0, Moon re-aimed at the near side) and returns
`true`, and the application calls `Controller::reset_animation()` to cancel any
in-flight zoom/flick (which targets the old body's scale). The headless `render`
path picks the body directly: the `--scene` `camera.target` field is
`"earth"` (default) or `"moon"` (a center-free `CameraTargetSpec` in
`snapshot.rs`; the Moon center is filled from the ephemeris at render time).

## Backdrop anchoring

Star lookup and sun disc are functions of the **camera-relative view
direction**, not absolute position on the celestial sphere. This keeps sun and
stars locked at any orbit/zoom (the backdrop is at infinity — no parallax, no
zoom dependence). Changing this reintroduces parallax between sun and stars (a
fixed bug).

The star shell (`vs_stars`) is **centered on the camera** (`world = camera_pos +
normal * STARS_RADIUS_KM`), so it always encloses the eye and the camera-relative
direction is exactly the vertex normal — independent of camera position. This is
load-bearing for **non-Earth targets**: orbiting the Moon puts the camera
~384,000 km from the Earth origin, far outside an origin-centered shell, which
dropped half the sky and the Sun. For an Earth orbit the eye was always inside
the old origin-centered shell, where both formulations give the same per-pixel
direction, so the change is a no-op there (a re-render diff is < 1 LSB on a
handful of pixels).

## Moon occludes the Earth's atmosphere

The atmosphere pass does not depth-test (it layers aerial perspective over the
Earth's own near disc, whose written depth is closer than the far-side
atmosphere shell). So `fs_atmosphere` explicitly drops fragments where the view
ray meets the **Moon** in front of the atmosphere entry (`ray_sphere` against the
Moon, `moon.x < shell.x`); otherwise the additive Earth-atmosphere glow bleeds
over the nearer Moon from a Moon-orbit view. The Moon is ~384,000 km out, so from
an Earth orbit this never triggers.

## Associated constants (km)

The distance/near/default limits are **ratios of the target body's mean radius**
(`<radii>` consts on `Camera`, multiplied by `target.mean_radius_km()` in the
instance methods `min_distance`/`max_distance`/`near_plane`/`default_distance`),
so the old tuned Earth feel is preserved and the Moon gets the same feel at its
own scale:
- `FOV_Y = 45 deg`
- `MIN_DISTANCE_RADII = 0.01` (Earth ~63.7 km, Moon ~17.4 km)
- `MAX_DISTANCE_RADII = 10.0` (Earth ~63710 km, Moon ~17374 km)
- `NEAR_PLANE_RADII = 0.01`; `FAR_PLANE = 500_000 km` (a fixed value, NOT a
  radius multiple) — it must enclose the Moon at lunar apogee (~406,700 km) plus
  the camera distance, and (orbiting the Moon) the Earth ~384,400 km away; the
  star shell (222,985 km) is well inside it.
- `MAX_TILT = 80 deg`
- `DEFAULT_DISTANCE_RADII = 2.0` (Earth default ~12742 km); lat clamp +/- 89 deg.
  `clamp_distance` is now an **instance** method (limits depend on the target).

`view_proj` uses a **reversed-Z** projection: it post-multiplies a Z-flip
matrix onto `Mat4::perspective_rh` so the near plane maps to depth 1 and the
far plane to 0. This is paired with the renderer's `Depth32Float` buffer
cleared to `0.0` and a `Greater` depth test (see `shader.md`); across the huge
~64 km -> 500,000 km near/far span a forward-Z float buffer would have almost
no precision near the Moon, so reversed-Z is load-bearing for the Earth-occludes-
Moon test. The clear value, compare op, and projection sign must all agree.
