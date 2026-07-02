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
| **ITRF** | Standard Earth-fixed ECEF: X = equator+prime-meridian, Y = 90°E, Z = north pole | Output of GCRF/TEME->Terra rotations |
| **world** | Project frame: Y = north, Z = prime meridian, X = 90°E; km; origin = Terra center | Renderer (mesh, camera, uniforms) |
| **celestial** | GCRF re-permuted to Y-up (equatorial); pole = celestial pole | Camera inertial rig (`star_rot_inv`) |
| **galactic** | celestial rotated by the static galactic->equatorial offset, so the galactic-drawn texture lines up with the equatorial equirect lookup | Star map sampling (`star_tex_rot_inv`) |

The **world** frame is ITRF with axes permuted: `world (X,Y,Z) = ITRF (Y,Z,X)`.
See `P` in `coordinates.md`.

## init_satkit — must seed all four (do not drop any seed)

**`init_satkit()` must seed the ephemeris, the EOP table, the three IERS
nutation/CIO tables, and the EGM96 gravity model** before any satkit use.
Call it once at the start of each scenario's `run()`.

```rust
satkit::jplephem::init_from_bytes(EPHEMERIS)        // DE440 OnceLock
satkit::earth_orientation_params::init_from_bytes(EOP)
satkit::earth_orientation_params::disable_eop_time_warning()
init_iers_table_from_bytes(IersTableId::Tab5A, TAB5A)   // CIP X series
init_iers_table_from_bytes(IersTableId::Tab5B, TAB5B)   // CIP Y series
init_iers_table_from_bytes(IersTableId::Tab5D, TAB5D)   // CIO locator s
satkit::earthgravity::init_from_bytes(GravityModel::EGM96, EGM96)
```

Where `EPHEMERIS`, `EOP`, `TAB5A`/`TAB5B`/`TAB5D`, and `EGM96` are embedded
via `include_bytes!` from `OUT_DIR`.

**Why every seed is required:**
1. **Accuracy**: real EOP (polar motion + UT1-UTC) makes the satellite's
   `qteme2itrf` sub-arcsec. Without the EOP seed, all frame transforms
   silently fall back to zeros for EOP.
2. **No stray dir**: every frame transform reads satkit's global EOP table on
   first use, and the full GCRF transforms additionally read the three IERS
   tables. Satkit's default lazy loader *creates an empty `satkit-data` dir
   next to the binary* as a side effect (`datadir()` calls `create_dir_all`)
   and the IERS resolver also `from_file(..).unwrap()`s, panicking if absent.
   Seeding all of them up front consumes the one-shot loads so that dir is
   never created.
3. **EGM96 has the same stray-dir failure mode**: the numerical orbit
   propagator (`orbitprop`, the `Propagation::Numerical` path arm) resolves
   the gravity model via `settings.gravity_model.get()` on every propagation,
   and satkit's lazy default runs `Gravity::from_file("EGM96.gfc")` out of
   the data dir (creating it; panicking/downloading if absent). A late seed
   fails with `AlreadyInitialized`. Seeded unconditionally — whether a scene
   contains numerically-propagated satellites is not knowable at init.

The IERS tables are needed because the celestial sphere now uses the **full
IERS-2010** GCRF<->ITRF transforms (`qgcrf2itrf`/`qitrf2gcrf`), not the
`*_approx` ones.

## Ephemeris-driven Sol & star map

Star map and Sol are **ephemeris-driven** — do not replace with a Sol-attached
rotation:
- `sol_dir`: `geocentric_pos(SolarSystem::Sun, time)` -> GCRF -> ITRF
  (full `qgcrf2itrf`) -> world via P -> normalize.
- `star_rot_inv = P * R_itrf2gcrf * P^T` (world -> equatorial celestial). This
  is the frame the camera rig is built from (see `camera.md`). As time
  advances, the celestial sphere rotates at the sidereal rate consistent with
  Sol.
- The embedded star texture (`8k_stars_milky_way.jpg`) is drawn in **galactic**
  coordinates (Milky Way horizontal, bulge centered), but `fs_stars` does an
  equatorial equirectangular lookup. So the *shader-facing* matrix is
  `star_tex_rot_inv = GALACTIC_OFFSET * star_rot_inv`, where `GALACTIC_OFFSET`
  (in `celestial_sphere.rs`) is the static IAU galactic->equatorial rotation
  brought into the permuted axes (`P R_EQU2GAL P^T`). Uploaded to the shader as
  `star_rot_inv`; the equatorial `star_rot_inv` stays on the camera rig only.

The Sol/star backdrop uses the full IERS-2010 transforms
(`qgcrf2itrf`/`qitrf2gcrf`, sub-arcsec): real UT1-UTC, polar motion, and the
IERS nutation/CIO tables. This matches the satellite's full `qteme2itrf`.

## Ephemeris-driven Luna (celestial_sphere.rs)

Luna is positioned and oriented per frame in `CelestialSphere::at`:
- **Position**: `geocentric_pos(SolarSystem::Moon, time)` (GCRF, meters) ->
  ITRF via the same `qgcrf2itrf` as Sol -> world via `P`, /1000 for km.
  Rendered at true scale/distance (~384,400 km), so it shows real angular size
  and Terra must occlude it via the depth buffer (`renderer.md`).
- **Orientation** (`lunar_body_to_gcrf`): the full **IAU/IAG 2009/2015 lunar
  rotation** series — pole `alpha0(T)`/`delta0(T)` + prime-meridian `W(d)` with
  the 13 libration arguments `E1..E13`. This gives the correct near side facing
  Terra *with* libration, not a rigid tidal lock. satkit exposes no lunar body
  frame, so this is implemented here. Time is TT days since J2000 via
  `as_jd_with_scale(TimeScale::TT)`.
- `luna_rot` (uploaded to the shader) = `P * R_gcrf2itrf * M_body2gcrf * P^T`.
  The `P^T` un-permutes the lunar *mesh's* project-convention axes (+Y north,
  +Z sub-Terra) into the standard (Z=pole) frame `M_body2gcrf` expects, so the
  mesh shares Terra's body convention. It is a pure rotation (normals need no
  transpose).
- Luna is a **triaxial ellipsoid** (`src/luna.rs`), the long axis toward
  Terra; the deviation from a sphere is ~1-3 km (imperceptible) but modeled.
- Test: `luna_near_side_faces_terra` asserts the sub-Terra point faces Terra
  within the optical libration (~8 deg), validating the model render-free.

## Ephemeris-driven planets (celestial_sphere.rs)

`CelestialSphere::at` also fills `sol_pos_world` (km, for planet lighting) and
assembles the seven planets into the `bodies: Vec<BodyState>` render list
(after Terra + Luna entries, in `planet::ALL` order), each per frame from
the same DE440:
- **Position**: `geocentric_pos(SolarSystem::X, time)` -> ITRF via the same
  `qgcrf2itrf` -> world via `P`, /1000 for km. True geocentric, so the outer
  planets are billions of km out (hence the **floating origin**, see `camera.md`).
- **Orientation** (`iau_body_to_gcrf`, the planet twin of `lunar_body_to_gcrf`
  without the libration series, sharing the `body_basis` helper): pole
  `alpha0(T)`/`delta0(T)` + prime meridian `W = W0 + W_dot*d` from the IAU
  constants in `src/planet.rs` (`w_dot` negative for the retrograde rotators
  Venus, Uranus). `planet_rot = P * R_gcrf2itrf * iau_body_to_gcrf * P^T`, same
  `P^T` un-permute as Luna.
- Planets are **oblate ellipsoids** (`src/planet.rs`: equatorial vs polar radii;
  Saturn/Jupiter visibly flattened), lit by simple Lambert with the Sol direction
  *at the planet*. The planet<->`SolarSystem` map + the IAU evaluation live in
  `celestial_sphere` so `planet.rs` stays satkit-free (like `terra`/`luna`).
- **f32 precision note.** `geocentric_pos` is exact (f64, ~meters; it never
  errors in range — `Result::Ok` for every planet). `nvec` then converts to f32,
  which at planetary distance quantizes the *absolute* position to ~6 km
  (Mercury) up to ~93 km (Saturn); Neptune's f32 ulp is ~539 km. This does NOT
  cause jitter: the renderer draws everything **relative to the camera target**
  (see `camera.md`/`renderer.md`), and the orbited body's relative position is a
  bit-exact zero because `render_origin` reuses the *same* f32 value. Far bodies
  are sub-pixel specks, so their position error is invisible. No f64 positions
  are needed.

## Satellite pipeline (satellite.rs)

Each `Satellite` is element-set agnostic — TLEs live in the scenarios, not
here. `from_tle(text)` parses a 3-line TLE. Position is **not stored**;
`state_at(time)` propagates on demand (`sgp4` needs `&mut TLE`). The parsed
`TLE` is retained because sgp4 caches its propagator inside it.

Pipeline per frame (in `state_at`):
1. `sgp4(&mut tle, &[time])` -> `DMatrix<f64>` 3xN, **meters, TEME**.
2. `qteme2itrf(&time) * teme` -> standard ECEF, meters (full transform, sub-arcsec with real EOP).
3. `ITRFCoord::from_vector(&itrf).to_geodetic_rad()` -> (lat, lon, hae).
4. `terra::surface_position(lat,lon) + terra::geodetic_normal(lat,lon) * alt_km`
   -> world-space km. (Goes through WGS84 helpers to guarantee the marker
   lands on the exact same ellipsoid the mesh uses.)

**Predicted orbit path (`orbit_path_inertial`)** — the renderer's one-period
twin, called from `SceneRenderer::prepare` with each marker's
`satellite::Propagation` and dispatching on it (a scene may mix both kinds;
`iss_and_hubble` deliberately does — ISS SGP4, Hubble numerical):
- **`Sgp4(Box<TLE>)`** (~65 us): one batch `sgp4(&mut tle, &times)` call for
  `segments + 1` samples across one period (`86400 / tle.mean_motion`
  seconds, mean motion is rev/day); TLE cloned inside because `sgp4` needs
  `&mut`. Samples rotate through the single `qteme2itrf(now)`.
- **`Numerical(OrbitState)`** (~0.4 ms): satkit `orbitprop` from GCRF initial
  conditions (`OrbitState { pos_gcrf_m, vel_gcrf_m_s }`, plain data, no TLE —
  the arm manually-controlled satellites fly on (`manual_control` feeds its
  live post-burn state; `iss_and_hubble`'s Hubble fills it from the marker's
  own SGP4 sample rotated by `qteme2gcrf`)). Period from
  `Kepler::from_pv(..).period()` (semi-major axis only, so circular/
  equatorial singularities cannot bite; e >= 1 — reachable by burning to
  escape — returns the **empty path**, which the renderer skips). One
  `orbitprop::propagate` over `[now, now + period]` with
  `PropSettings { use_spaceweather: false, ..default }` and `satprops: None`
  — drag/SRP only run with a `Some` satprops, so satkit's space-weather
  loader (not embedded) stays unreachable; EGM96 4x4 gravity + Sun/Moon
  third-body (embedded DE440) + tides + relativity stay on. All samples from
  the dense output in one `interp_batch`, rotated by the single
  `qgcrf2itrf(now)`.

Both arms share the frame treatment, which deliberately differs from the
marker: ALL inertial samples rotate through ONE current-time rotation (the
star-fixed inertial ellipse, not the per-sample-rotation ground-track curve),
then map ITRF -> world by the plain P permutation
(`world (x,y,z) = ITRF (y,z,x)`) — no geodetic round trip, which exists on
the marker only to land it on the exact mesh ellipsoid. Fast enough to
recompute every frame (no caching).

**TLE-free manual-control pipeline** (`satellite.rs`, all sharing the
`numerical_settings()` force model above):
- `propagate_numerical(state, from, to)` — one `orbitprop` step reading
  `PropagationResult.state_end`; the `manual_control` scenario's per-frame
  re-anchor of its stored `OrbitState` to the clock (burn delta-v is then
  added to the velocity, so it compounds into every later frame).
- `resolve_orbit(state, time)` — GCRF -> ITRF via `qgcrf2itrf(time)`, then
  the same geodetic round trip as the SGP4 marker (shared `state_from_itrf`
  tail), producing an identical `SatelliteState`. Pure frame change: the
  state must already be propagated to `time`.
- `orbit_shape(state)` — osculating apo/peri altitudes (above the *mean*
  radius, a spherical readout convenience) + speed from `Kepler::from_pv`;
  `None` for e >= 1 (panel shows dashes).
- Validated render-free by `numerical_pipeline_holds_circular_leo`
  (`cargo test`), which drives all three on a constructed circular LEO with
  no TLE. Tests seed satkit through the `Once`-guarded
  `init_satkit_for_tests` (the ephemeris seed is set-once per process and
  the test binary runs every test in one process).

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

**Ephemeris bodies**: `geocentric_pos(SolarSystem::{Sol,Luna,Mercury..Neptune},
tm)` returns GCRF `Vector3` in meters (`None` outside DE440 range) — `SolarSystem`
exposes **every** planet, so planet positions are free (no extra ephemeris).
**Time scales**: `as_jd_with_scale(TimeScale::TT)` gives TT Julian days (J2000 =
2451545.0 TT); used by the IAU lunar + planet rotations.
satkit exposes **no lunar body-fixed frame** — the IAU rotation is implemented
in `celestial_sphere.rs`.

**Frame transforms (frametransform module)**:
- `qteme2itrf(tm)` — full, reads EOP (real polar motion + UT1-UTC).
- `qgcrf2itrf(tm)` / `qitrf2gcrf(tm)` — full IERS-2010, sub-arcsec; read EOP +
  the three IERS nutation/CIO tables. Used by the celestial sphere. The tables
  are seeded in `init_satkit` via
  `init_iers_table_from_bytes(IersTableId::{Tab5A,Tab5B,Tab5D}, ..)`.
- `qteme2gcrf(tm)` — `qitrf2gcrf_approx * qteme2itrf` (~arcsec, ~30 m at LEO);
  bootstraps the marker's GCRF `OrbitState` from the SGP4 TEME sample.
  Rotating the TEME *velocity* by the same quaternion is correct (both frames
  quasi-inertial, no omega-cross term).
- `qgcrf2itrf_approx(tm)` / `qitrf2gcrf_approx(tm)` — IAU-76/FK5, ~1 arcsec;
  read UT1-UTC via `gmst` but neglect polar motion + use approximate nutation.
  Not used directly (only inside `qteme2gcrf` / orbitprop internals).
- Apply quaternion to vector: `q * v`; `Quaternion` is `Copy`.

**Numerical propagation (orbitprop module)**: `orbitprop::propagate(&SimpleState
/* Matrix<6,1>: GCRF x,y,z m + vx,vy,vz m/s */, &begin, &end, &PropSettings,
satprops)` -> `PropagationResult` with dense output; sample arbitrary times
with `result.interp_batch(&[Instant])` (one propagate + one batch, never a
per-sample loop). `SimpleState::zeros()` then index `s[0..=5]`. Drag + solar
radiation pressure run ONLY when `satprops` is `Some` with nonzero
coefficients — `None` keeps NRLMSISE/space-weather unreachable. **Gravity
model gotcha**: `propagate` resolves `settings.gravity_model.get()` on every
call — EGM96 must be pre-seeded (see init_satkit above). `Kepler::from_pv(r,
v)` (GCRF m, m/s) -> osculating elements; `.period()` seconds.

**EOP valid range**: 1962-01-01 to last `EOP-All.csv` entry (~build date).
Out-of-range returns `None` -> zeros. Pre-seeding the EOP is what stops
`satkit-data` from being created; the binary is fully offline.

**`data_found() -> bool` gotcha**: checks for the *full* satkit data bundle
(EOP + space weather), so it returns `false` even when just the ephemeris is
present. Do not gate ephemeris use on it — use `geocentric_pos` directly and
let it succeed or fail.
