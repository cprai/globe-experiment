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
- Constants and helpers in `src/terra.rs` — single source of truth; mesh
  and camera both call it.

## Axis permutation P (world <-> ECEF)

`P(x,y,z) = (y,z,x)` — maps standard ECEF/GCRF (Z = pole) into the world
frame (Y = north). As a `glam::Mat3` it is `from_cols((0,0,1),(1,0,0),(0,1,0))`.
`P^T = P^(-1)` (it is a proper rotation).

Why: for geodetic `(lat, lon)` the standard ECEF unit vector is
`(cos_lat*cos_lon, cos_lat*sin_lon, sin_lat)`; applying P gives
`(cos_lat*sin_lon, sin_lat, cos_lat*cos_lon)` = `terra::geodetic_normal(lat,lon)`.
So P is consistent with every WGS84 helper and with every satkit result.

## Surface geometry

- Mesh carries explicit **geodetic normals** (not `normalize(position)` on an
  ellipsoid). The normal at `(lat, lon)` is
  `(cos_lat*sin_lon, sin_lat, cos_lat*cos_lon)` — same direction as a sphere.
- `terra::surface_position(lat, lon)` is the WGS84 ellipsoid point in km.
- Atmosphere and star shells use the unit normal * their radii — spherical,
  not ellipsoid position. Do not try to make the LUT model ellipsoidal.
- Atmosphere spherical constants: `PLANET_RADIUS_KM 6360`, `ATMOSPHERE_TOP_KM
  6460`. These must match between `build.rs mod atmosphere` and
  `shaders/scene.wgsl`.

## Luna body frame

- `src/luna.rs` is the lunar twin of `terra.rs`: a **triaxial** ellipsoid
  (semi-axes ~1737.4 / 1735.7 / 1734.5 km) in the **same body convention** as
  Terra — +Y = north (rotation pole), selenographic lon0/lat0 (the mean
  sub-Terra point) -> +Z, +X = 90 deg east. `surface_position` scales the
  sphere direction per axis; `geodetic_normal` is the ellipsoid gradient
  (`x/rx^2, y/ry^2, z/rz^2`), not radial.
- The mesh is built in this body frame and oriented into the world per frame by
  `luna_rot` (ephemeris Earth orientation composed with the IAU lunar rotation;
  see `simulation.md`). The `P^T` inside `luna_rot` converts the mesh's
  project-convention axes into the standard (Z=pole) frame the IAU model uses.

## Planet body frames

- `src/planet.rs` is the multi-body twin of `terra.rs`/`luna.rs`: each of the 7
  planets is an **oblate ellipsoid of revolution** (equatorial radius on +X/+Z,
  polar radius on +Y) in the **same body convention** — +Y = north (rotation
  pole), prime meridian lon0/lat0 -> +Z, +X = 90 deg east. `surface_position`
  scales the sphere direction per axis; `geodetic_normal` is the gradient
  (`x/req^2, y/rpol^2, z/req^2`), not radial.
- Oriented into the world per frame by `planet_rot` = `P * R_gcrf2itrf *
  iau_body_to_gcrf * P^T` (same `P^T` un-permute as Luna; see `simulation.md`).

## Equirectangular UV mapping

- Forward (mesh): `u = (lon+180)/360`, `v = 0` at north -> `v = 1` at south.
  Sampler repeats on U (dateline wrap), clamps on V (poles). Seam column
  duplicated.
- Inverse (`fs_stars`, by direction `d`): `u = atan2(d.x, d.z)/(2*pi) + 0.5`,
  `v = acos(d.y)/pi`. Computed **per fragment** — interpolating `u` across a
  triangle crossing the +/-180 seam would smear the entire texture. Here `d`
  is in the star texture's frame: `d = star_tex_rot_inv * view_dir`. The
  texture is drawn in **galactic** coordinates, so `star_tex_rot_inv` carries a
  static galactic->equatorial offset on top of the equatorial orientation (see
  `simulation.md`).
