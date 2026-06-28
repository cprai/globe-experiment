---
paths:
  - "src/simulation/**/*.rs"
---

# Simulation & satkit rules

## Reference frames

| Frame | Definition | Used for |
|---|---|---|
| **TEME** | True Equator Mean Equinox; SGP4's native quasi-inertial frame | SGP4 output (satellite) |
| **GCRF/ICRF** | Geocentric Celestial Reference Frame; inertial; Z = celestial pole, X ~ vernal equinox | JPL ephemeris output; star (celestial) frame |
| **ITRF** | Standard Earth-fixed ECEF: X = equator+prime-meridian, Y = 90°E, Z = north pole | Output of GCRF/TEME->Earth rotations |
| **world** | Project frame: Y = north, Z = prime meridian, X = 90°E; km; origin = Earth center | Renderer (mesh, camera, uniforms) |
| **celestial** | GCRF re-permuted to Y-up (equatorial); pole = celestial pole | Camera inertial rig (`star_rot_inv`) |
| **galactic** | celestial rotated by the static galactic->equatorial offset, so the galactic-drawn texture lines up with the equatorial equirect lookup | Star map sampling (`star_tex_rot_inv`) |

The **world** frame is ITRF with axes permuted: `world (X,Y,Z) = ITRF (Y,Z,X)`.
See `P` in `coordinates.md`.

## init_satkit — must seed both (do not drop either seed)

**`init_satkit()` must seed both the ephemeris and the EOP table** before
any satkit use. Call it once at the start of each scenario's `run()`.

```rust
satkit::jplephem::init_from_bytes(EPHEMERIS)        // DE440 OnceLock
satkit::earth_orientation_params::init_from_bytes(EOP)
satkit::earth_orientation_params::disable_eop_time_warning()
```

Where `EPHEMERIS` and `EOP` are embedded via `include_bytes!` from `OUT_DIR`.

**Why both seeds are required:**
1. **Accuracy**: real EOP (polar motion + UT1-UTC) makes the satellite's
   `qteme2itrf` sub-arcsec. Without the EOP seed, all frame transforms
   silently fall back to zeros for EOP.
2. **No stray dir**: every frame transform (including the `*_approx` ones)
   reads satkit's global EOP table on first use. Satkit's default lazy loader
   *creates an empty `satkit-data` dir next to the binary* as a side effect
   (`datadir()` calls `create_dir_all`). Seeding up front consumes the
   one-shot `DEFAULT_LOAD_ONCE` so that dir is never created.

**Full IERS-2010 for the celestial sphere** additionally needs the IERS
nutation tables (Tab5A/5B/5D) bundled and seeded — not done. Without seeding
those, the full (non-approx) GCRF transforms would panic + recreate
`satkit-data`. See `backlog.md`.

## Ephemeris-driven Sun & star map

Star map and Sun are **ephemeris-driven** — do not replace with a sun-attached
rotation:
- `sun_dir`: `geocentric_pos(SolarSystem::Sun, time)` -> GCRF -> ITRF (approx)
  -> world via P -> normalize.
- `star_rot_inv = P * R_itrf2gcrf * P^T` (world -> equatorial celestial). This
  is the frame the camera rig is built from (see `camera.md`). As time
  advances, the celestial sphere rotates at the sidereal rate consistent with
  the Sun.
- The embedded star texture (`8k_stars_milky_way.jpg`) is drawn in **galactic**
  coordinates (Milky Way horizontal, bulge centered), but `fs_stars` does an
  equatorial equirectangular lookup. So the *shader-facing* matrix is
  `star_tex_rot_inv = GALACTIC_OFFSET * star_rot_inv`, where `GALACTIC_OFFSET`
  (in `celestial_sphere.rs`) is the static IAU galactic->equatorial rotation
  brought into the permuted axes (`P R_EQU2GAL P^T`). Uploaded to the shader as
  `star_rot_inv`; the equatorial `star_rot_inv` stays on the camera rig only.

The Sun/star backdrop uses `*_approx` transforms (~1 arcsec): real UT1-UTC
via `gmst` but approximate nutation and no polar motion. The satellite uses
the full `qteme2itrf` (sub-arcsec).

## Satellite pipeline (satellite.rs)

Each `Satellite` is element-set agnostic — TLEs live in the scenarios, not
here. `from_tle(text)` parses a 3-line TLE. Position is **not stored**;
`state_at(time)` propagates on demand (`sgp4` needs `&mut TLE`). The parsed
`TLE` is retained because sgp4 caches its propagator inside it.

Pipeline per frame (in `state_at`):
1. `sgp4(&mut tle, &[time])` -> `DMatrix<f64>` 3xN, **meters, TEME**.
2. `qteme2itrf(&time) * teme` -> standard ECEF, meters (full transform, sub-arcsec with real EOP).
3. `ITRFCoord::from_vector(&itrf).to_geodetic_rad()` -> (lat, lon, hae).
4. `earth::surface_position(lat,lon) + earth::geodetic_normal(lat,lon) * alt_km`
   -> world-space km. (Goes through WGS84 helpers to guarantee the marker
   lands on the exact same ellipsoid the mesh uses.)

## satkit API quick reference (verified against v0.18.1)

**Types**: `satkit::Instant` (µs since Unix epoch, UTC, `Copy`),
`satkit::Duration` (`from_seconds(f64)`), `satkit::Vector3::new([[x],[y],[z]])`
(column-major — NOT three scalars), `satkit::SolarSystem` (not `jplephem::SolarSystem`
which is private).

**Axis convention**: satkit ITRF/ECEF is X = prime meridian, Y = 90°E, Z =
north. Our world is X = 90°E, Y = north, Z = prime meridian. Bridge via P
(see `coordinates.md`) or via geodetic lat/lon/alt (what `satellite.rs` does).

**Units**: SGP4 output and `ITRFCoord` are in **meters**. Divide by 1000 for
km world space.

**Frame transforms (frametransform module)**:
- `qteme2itrf(tm)` — full, reads EOP (real polar motion + UT1-UTC).
- `qgcrf2itrf_approx(tm)` / `qitrf2gcrf_approx(tm)` — IAU-76/FK5, ~1 arcsec;
  read UT1-UTC via `gmst` but neglect polar motion + use approximate nutation.
- Full `qgcrf2itrf`/`qitrf2gcrf` (IERS-2010) additionally needs
  `ierstable::init_from_bytes` for Tab5A/5B/5D — not done.
- Apply quaternion to vector: `q * v`; `Quaternion` is `Copy`.

**EOP valid range**: 1962-01-01 to last `EOP-All.csv` entry (~build date).
Out-of-range returns `None` -> zeros. Pre-seeding the EOP is what stops
`satkit-data` from being created; the binary is fully offline.

**`data_found() -> bool` gotcha**: checks for the *full* satkit data bundle
(EOP + space weather), so it returns `false` even when just the ephemeris is
present. Do not gate ephemeris use on it — use `geocentric_pos` directly and
let it succeed or fail.
