---
paths:
  - "src/**/*.rs"
  - "shaders/globe.wgsl"
---

# Coordinate system & mapping

## World frame

- **WGS84 ellipsoid at origin; world space in km; +Y = north pole; lon0/lat0
  -> +Z; +X = 90 deg E.** This is ITRF/ECEF with axes permuted so north is
  +Y.
- Constants and helpers in `src/earth.rs` — single source of truth; mesh
  and camera both call it.

## Axis permutation P (world <-> ECEF)

`P(x,y,z) = (y,z,x)` — maps standard ECEF/GCRF (Z = pole) into the world
frame (Y = north). As a `glam::Mat3` it is `from_cols((0,0,1),(1,0,0),(0,1,0))`.
`P^T = P^(-1)` (it is a proper rotation).

Why: for geodetic `(lat, lon)` the standard ECEF unit vector is
`(cos_lat*cos_lon, cos_lat*sin_lon, sin_lat)`; applying P gives
`(cos_lat*sin_lon, sin_lat, cos_lat*cos_lon)` = `earth::geodetic_normal(lat,lon)`.
So P is consistent with every WGS84 helper and with every satkit result.

## Surface geometry

- Mesh carries explicit **geodetic normals** (not `normalize(position)` on an
  ellipsoid). The normal at `(lat, lon)` is
  `(cos_lat*sin_lon, sin_lat, cos_lat*cos_lon)` — same direction as a sphere.
- `earth::surface_position(lat, lon)` is the WGS84 ellipsoid point in km.
- Atmosphere and star shells use the unit normal * their radii — spherical,
  not ellipsoid position. Do not try to make the LUT model ellipsoidal.
- Atmosphere spherical constants: `PLANET_RADIUS_KM 6360`, `ATMOSPHERE_TOP_KM
  6460`. These must match between `build.rs mod atmosphere` and
  `shaders/globe.wgsl`.

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
