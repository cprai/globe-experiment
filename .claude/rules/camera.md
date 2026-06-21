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

`Camera.longitude` / `Camera.latitude` are **inertial directions**, not
geographic coordinates. Do not move the camera into the ECEF/world frame.

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
- `NEAR_PLANE ~63.7 km`, `FAR_PLANE ~318550 km` (50 * R)
- `MAX_TILT = 80 deg`
- Default distance `~12742 km` (2 * R); lat clamp +/- 89 deg

The `_rh` projection variants give wgpu's 0..1 depth (no depth buffer is
used; near/far only bound clipping, and the star shell must fit inside
`FAR_PLANE`).
