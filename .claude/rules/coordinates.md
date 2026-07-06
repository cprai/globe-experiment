---
paths:
  - "src/**/*.rs"
  - "shaders/scene.wgsl"
---

# Coordinate system & mapping

## World frame

- **WGS84 ellipsoid at origin; world space in km; +Y = north pole; lon0/lat0
  -> +Z; +X = 90 deg E.** This is ITRF/ECEF with axes permuted so north is
  +Y.
- Constants and helpers in `src/engine/planet.rs` (Terra is a row of the
  shared per-body table; there is no terra module) — single source of truth;
  the impostor renderer, the camera, and the satellite pipeline all call it.

## Axis permutation P (world <-> ECEF)

`P(x,y,z) = (y,z,x)` — maps standard ECEF/GCRF (Z = pole) into the world
frame (Y = north). As a `glam::Mat3` it is `from_cols((0,0,1),(1,0,0),(0,1,0))`.
`P^T = P^(-1)` (it is a proper rotation).

Why: for geodetic `(lat, lon)` the standard ECEF unit vector is
`(cos_lat*cos_lon, cos_lat*sin_lon, sin_lat)`; applying P gives
`(cos_lat*sin_lon, sin_lat, cos_lat*cos_lon)` = `planet::geodetic_normal(TERRA, lat, lon)`.
So P is consistent with every WGS84 helper and with every satkit result.

## Surface geometry

- **Latitude convention is shape-driven** (`planet::surface_position` /
  `geodetic_normal`): a spheroid of revolution (rx == rz — Terra and every
  planet) treats latitude as **geodetic** (the WGS84 prime-vertical
  formulation; for Terra this is bit-for-bit the old terra-module math, and
  satellite geodetic coordinates land on the same ellipsoid), while the
  triaxial body (Luna) uses parametric latitude. The geodetic normal at
  `(lat, lon)` is `(cos_lat*sin_lon, sin_lat, cos_lat*cos_lon)` — same
  direction as a sphere — for spheroids; the ellipsoid gradient for Luna.
- There is NO mesh anywhere: the impostor fragment shader ray-traces each
  body's ellipsoid and derives the geodetic normal as the ellipsoid gradient
  `normalize(p/radii)` in unit-sphere space (identical to the geodetic
  definition for a spheroid).
- The atmosphere is a SPHERE (radius `PLANET_RADIUS_KM 6360` /
  `ATMOSPHERE_TOP_KM 6460`), traced analytically in `fs_atmosphere` — not the
  WGS84 ellipsoid. Do not try to make the LUT model ellipsoidal. The geometric
  constants must match across `build.rs mod atmosphere`, `shaders/scene.wgsl`,
  and `renderer::ATMOSPHERE_TOP_KM` (the quad sizing twin).

## Luna body frame

- Luna is an entry in the shared per-body table (`src/engine/planet.rs` -
  there is no separate luna module): a **triaxial** ellipsoid
  (semi-axes ~1737.4 / 1735.7 / 1734.5 km) in the **same body convention** as
  Terra — +Y = north (rotation pole), selenographic lon0/lat0 (the mean
  sub-Terra point) -> +Z, +X = 90 deg east. The shared `surface_position`
  scales the
  sphere direction per axis; `geodetic_normal` is the ellipsoid gradient
  (`x/rx^2, y/ry^2, z/rz^2`), not radial.
- Luna has NO mesh: it is drawn as a **shader impostor** like the planets -
  `fs_planet` ray-traces this triaxial ellipsoid in the body frame and the
  per-body uniform's `rot` (the lunar placement rotation: ephemeris Earth
  orientation composed with the IAU lunar rotation; see `simulation.md`)
  orients the traced point + normal into the world. The `P^T` inside that
  rotation converts the body's
  project-convention axes into the standard (Z=pole) frame the IAU model uses.

## Planet body frames

- In the same `src/engine/planet.rs` table: each of the 7
  planets is a **triaxial ellipsoid with equal +X/+Z axes** - its familiar
  oblate spheroid (equatorial radius on +X/+Z,
  polar radius on +Y), in the same triaxial formulation as Luna
  (`radii_km() -> Vec3`) - in the **same body convention** — +Y = north (rotation
  pole), prime meridian lon0/lat0 -> +Z, +X = 90 deg east. `surface_position`
  scales the sphere direction per axis; `geodetic_normal` is the gradient
  (`x/rx^2, y/ry^2, z/rz^2`), not radial. Planets are drawn as **shader
  impostors** (no mesh): `fs_planet` ray-traces this same triaxial ellipsoid
  (the uniform's `radii` vec3) and derives the geodetic normal + the
  equirectangular UV analytically, matching the convention above.
- Oriented into the world per frame by `planet_rot` = `P * R_gcrf2itrf *
  iau_body_to_gcrf * P^T` (same `P^T` un-permute as Luna; see `simulation.md`);
  the impostor applies it as `planet.rot` to the traced surface point + normal.

## View & screen handedness (camera basis, screen-aligned quads)

Everything is **right-handed**: the world frame above, glam's cross products,
and the view space built by `Mat4::look_at_rh` (camera looks down **-Z** in
view space). With `forward = normalize(look_at - eye)` and `up` the camera up:

- **screen-right (world direction that maps to NDC +x) = `cross(forward, up)`**
- **screen-up = `cross(right, forward)`**

`cross(up, forward)` is the **negative** of screen-right — a left-handed basis
that renders any screen-space-constructed image **horizontally mirrored**.
This exact bug shipped in `fs_planet`'s orthographic branch (fixed 2026-07-06:
every distant impostor body drew flipped, terminator on the wrong side, and
the perspective<->orthographic switch visibly snapped). When building a basis
that maps quad/screen offsets to world offsets, always use the order above.

- **NDC**: +x right, +y up (wgpu), depth **reversed-Z** (near = 1, far = 0;
  see `shader.md`).
- The reversed-Z flip touches only rows 2/3 of `view_proj`, and
  `perspective_rh` scales rows 0/1 by positive factors, so **row 0 xyz is a
  positive multiple of the camera's world right, row 1 of its up**. This is
  how a shader can recover the true camera basis with no extra uniform —
  `fs_planet`'s orthographic branch takes row 0, re-orthogonalizes it against
  the body's ray direction, and takes `up = cross(right, dir)`. Do not
  substitute a world-Y-derived reference basis: it rolls the image whenever
  the camera up is not north-aligned.

## Equirectangular UV mapping

- Inverse-only (there is no meshed forward mapping anymore), computed **per
  fragment**:
  - `fs_planet`: `u = atan2(p1.x, p1.z)/(2*pi) + 0.5` from the ray-traced hit
    point (exact for every body; no seam-interpolation smear), and `v =
    acos(n_body.y)/pi` from the GEODETIC NORMAL's y — for a spheroid
    `n_body.y = sin(geodetic lat)`, so the texture latitude is geodetic,
    matching the CPU-side `surface_position` convention (longitude stays
    position-derived: a normal-derived longitude would warp on the triaxial
    Luna).
  - `fs_stars`: `d = star_tex_rot_inv * view_dir` per fragment, `u/v` from `d`
    as above but with `v = acos(d.y)/pi` (a direction, not an ellipsoid). The
    texture is drawn in **galactic** coordinates, so `star_tex_rot_inv`
    carries a static galactic->equatorial offset on top of the equatorial
    orientation (see `simulation.md`).
- Sampler repeats on U (dateline wrap), clamps on V (poles).
