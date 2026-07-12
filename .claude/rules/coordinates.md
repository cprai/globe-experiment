---
paths:
  - "src/**/*.rs"
  - "src/engine/shaders/scene.wgsl"
---

# Coordinate system & mapping

## World frame

- **World space in km; +Y = north pole; lat0/lon0 -> +Z; +X = 90 deg E** —
  ITRF/ECEF with axes permuted so north is +Y. The geocentric variant (Terra
  at origin) hosts satellites, WGS84 helpers, and the Terra render frame; the
  `CelestialSphere` uses the same axes with a **heliocentric** origin (Sol at
  origin, Terra at `-sol_geo`). Full frames table in `simulation.md`.
- **f64 everywhere; f32 only at the GPU/egui boundary.** Heliocentric
  magnitudes (1.5e8 to billions of km) overflow f32 when subtracting the
  render origin back to a local offset — an f32 subtraction cancels
  catastrophically (Luna would shift ~16 km). Do not introduce intermediate
  f32 casts into computation paths.
- Body constants + surface helpers: `src/engine/planet.rs` (single source of
  truth; Terra is a table row, no terra/luna modules).

## Axis permutation P (world <-> ECEF)

`P(x,y,z) = (y,z,x)` maps standard ECEF/GCRF (Z = pole) into the world frame
(Y = north). `P^T = P^(-1)` (a proper rotation). For geodetic `(lat, lon)`
the standard ECEF unit vector `(cos_lat*cos_lon, cos_lat*sin_lon, sin_lat)`
becomes `(cos_lat*sin_lon, sin_lat, cos_lat*cos_lon)` =
`geodetic_normal(lat, lon)` — so P is consistent with every WGS84 helper and
every satkit result.

## Surface geometry

- **Latitude convention is shape-driven**: a spheroid of revolution (rx == rz
  — Terra and every planet) uses **geodetic** latitude (WGS84 prime-vertical
  formulation); the genuinely triaxial Luna uses parametric latitude.
  `geodetic_normal` is the ellipsoid gradient `(x/rx^2, y/ry^2, z/rz^2)`.
- There is NO mesh: the impostor shader ray-traces each ellipsoid and derives
  the normal as the gradient `normalize(p/radii)` in unit-sphere space
  (identical to the geodetic definition for a spheroid).
- The atmosphere is a SPHERE (not the WGS84 ellipsoid) — the LUT model
  assumes spherical symmetry; do not try to make it ellipsoidal.

## Body frames

Every body shares Terra's convention: +Y = north (rotation pole), prime
meridian (Luna: mean sub-Terra point) -> +Z, +X = 90 deg east. Each placement
rotation is `P * R_gcrf2itrf * M_body2gcrf * P^T` — the `P^T` un-permutes the
project-convention body axes into the standard (Z=pole) frame the IAU models
expect. Pure rotations; normals need no transpose.

## View & screen handedness (screen-aligned quads)

Everything is right-handed; view space looks down -Z. With `forward =
normalize(look_at - eye)`:

- **screen-right = `cross(forward, up)`**, **screen-up = `cross(right,
  forward)`**. `cross(up, forward)` is the NEGATIVE of screen-right — a
  left-handed basis that renders any screen-constructed image horizontally
  mirrored (this exact bug shipped in the orthographic impostor branch,
  fixed 2026-07-06).
- Reversed-Z touches only rows 2/3 of `view_proj`, and `perspective_rh`
  scales rows 0/1 positively, so **row 0 xyz is a positive multiple of the
  camera's world right, row 1 of its up** — how a shader recovers the camera
  basis with no extra uniform. Do not substitute a world-Y-derived basis: it
  rolls the image whenever the camera up is not north-aligned.

## Equirectangular UV mapping

Inverse-only, per fragment: `u = atan2(p.x, p.z)/(2*pi) + 0.5` from the
ray-traced hit point (no seam smear), `v = acos(n_body.y)/pi` from the
GEODETIC normal's y (so texture latitude is geodetic, matching
`surface_position`; longitude stays position-derived — a normal-derived
longitude would warp on triaxial Luna). Sampler repeats on U (dateline),
clamps on V (poles).
