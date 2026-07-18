---
paths:
  - "crates/engine/src/scene/**/*.rs"
---

# Simulation & satkit rules

## Reference frames

| Frame | Definition | Used for |
|---|---|---|
| **TEME** | True Equator Mean Equinox; SGP4's native quasi-inertial frame | SGP4 output |
| **GCRF/ICRF** | inertial; Z = celestial pole, X ~ vernal equinox | ephemeris output; star frame |
| **ITRF** | standard Earth-fixed ECEF | output of GCRF/TEME->Terra rotations |
| **world (heliocentric)** | project axes (Y = north, Z = prime meridian, X = 90°E); km; origin = Sol | `CelestialSphere` body centers (f64) |
| **world (geocentric)** | same axes; origin = Terra center | tracked bodies, WGS84 surface, Terra render frame |
| **celestial** | GCRF re-permuted to Y-up | camera inertial rig (`star_rot_inv`) |
| **galactic** | celestial rotated by the static galactic->equatorial offset | star map sampling (`star_tex_rot_inv`) |

Both world frames are ITRF permuted by `P` (see `coordinates.md`); they
differ only by a translation. Everything is f64 until GPU upload
(`coordinates.md` has the why).

## init_satkit — must seed all four (do not drop any seed)

`init_satkit()` seeds the DE440 ephemeris, the EOP table (+
`disable_eop_time_warning`), the three IERS tables (Tab5A/5B/5D: CIP X/Y +
CIO locator), and the EGM96 gravity model from embedded bytes, once per
`run()`. Why every seed is required:

1. **Accuracy**: real EOP makes `qteme2itrf` sub-arcsec; without the seed,
   transforms silently use zeros. The IERS tables back the full IERS-2010
   `qgcrf2itrf`/`qitrf2gcrf` the celestial sphere uses (not `*_approx`).
2. **No stray dir**: satkit's lazy loaders *create a `satkit-data` dir next
   to the binary* on first use and panic/download if files are absent.
   Pre-seeding consumes the one-shot loads.
3. **EGM96 same failure mode**, via `orbitprop` resolving the gravity model
   on every propagation; a late seed fails with `AlreadyInitialized`, so it
   is seeded unconditionally.

`engine-astrodynamics::init` seeds the same set-once stores from its own
embedded copies — the two initializers must never run in one process
(nothing links both crates today).

## Ephemeris-driven bodies (celestial_sphere.rs)

- Position chain per body: `geocentric_pos` (GCRF, m) -> ITRF via
  `qgcrf2itrf` -> world via `P`, /1000 -> heliocentric (`- sol_geo`), all
  f64. Sol itself is the origin (`sol_pos_world = ZERO`).
- **Star map + Sol are ephemeris-driven** — do not replace with a
  Sol-attached rotation. `star_rot_inv = P * R_itrf2gcrf * P^T` (world ->
  equatorial celestial, the camera-rig frame). The embedded star texture is
  drawn in **galactic** coordinates, so the shader-facing matrix is
  `star_tex_rot_inv = GALACTIC_OFFSET * star_rot_inv` (static IAU
  galactic->equatorial rotation in the permuted axes).
- **Luna orientation** is the full IAU/IAG lunar rotation series — pole
  `alpha0(T)`/`delta0(T)` + prime meridian `W(d)` with the 13 libration
  arguments `E1..E13` — giving the correct near side *with* libration.
  satkit exposes no lunar body frame, so it is implemented here. Time is TT
  days since J2000. Test: `luna_near_side_faces_terra`.
- **Planet orientation** (`iau_body_to_gcrf`): same model minus the libration
  series; IAU pole + `W = W0 + W_dot*d` constants live in `planet.rs`
  (`w_dot` negative for retrograde Venus/Uranus). The planet<->`SolarSystem`
  map and IAU evaluation live here so `planet.rs` stays satkit-free.

## Tracked-body pipeline (body / orbital_body / kinematic_body)

Element sets (TLEs) live in the scenes, not here. Position is never stored —
propagated on demand (`sgp4` needs `&mut TLE`; the parsed TLE is retained
because sgp4 caches its propagator inside it). Two body kinds, held directly
by the scenes as per-kind `Vec` fields: **`OrbitalBody`** (TLE + SGP4) and
**`KinematicBody`** (GCRF `OrbitState` + epoch, numerical `orbitprop`,
seedable once from a TLE, self-re-anchoring on every query, `orbit_shape`
for apo/peri/speed — `None` for e >= 1). Each scene's `frame_state` converts
its bodies to `TrackedBody` through the provided `Scene::tracked_bodies`
(over the derived `SceneOrbitalBodies`/`SceneKinematicBodies` accessors: dot
from `state_at`, `trail`, visibility from `body_occluded`); the bodies' one
mutator is `KinematicBody::apply_thrust`
(advance + `vel += dir * accel * dt`; dt <= 0 = paused = no-op) — see
`architecture.md` invariants.

Dot chain: SGP4 (m, TEME) -> `qteme2itrf` (kinematic: GCRF ->
`qgcrf2itrf`) -> geodetic -> world via `surface_position + geodetic_normal *
alt`. The geodetic round trip is deliberate: it guarantees the dot lands on
the exact WGS84 ellipsoid the Terra impostor traces.

**Predicted trail** (each body's `trail`, one period ahead, recomputed every
frame into `TrackedBody.trail` — cheap: batch SGP4 ~65 us, numerical
~0.4 ms):
- Both kinds rotate ALL inertial samples through ONE current-time rotation
  (the star-fixed inertial ellipse, not a ground track), then map ITRF ->
  world by the plain `P` permutation — no geodetic round trip, which exists
  on the dot only.
- The kinematic kind gets its period from `Kepler::from_pv(..).period()`
  (semi-major axis only, so circular/equatorial singularities cannot bite);
  e >= 1 (escape, reachable by burning) returns the empty trail.
- `PropSettings` keep `satprops: None` and no space weather — drag/SRP only
  run with `Some` satprops, keeping satkit's non-embedded space-weather
  loader unreachable; EGM96 + Sun/Moon third-body stay on.

Render-free tests: `numerical_pipeline_holds_circular_leo` & co. in
kinematic_body.rs, `orbital_body_holds_leo` in orbital_body.rs.

## satkit API quick reference (verified against v0.18)

- `satkit::Instant` (µs since Unix epoch, UTC, `Copy`);
  `Duration::from_seconds(f64)`; `Vector3::new([[x],[y],[z]])` (column-major,
  NOT three scalars); use `satkit::SolarSystem` (the jplephem one is
  private).
- satkit ITRF is X = prime meridian, Y = 90°E, Z = north; bridge to world via
  `P` or a geodetic round trip. SGP4/`ITRFCoord` are in **meters**.
- `geocentric_pos(SolarSystem::.., tm)` covers Sol, Luna, and every planet
  (GCRF meters; `None` outside DE440 range). `as_jd_with_scale(TimeScale::TT)`
  for the IAU rotations.
- Transforms: `qteme2itrf` (full, reads EOP); `qgcrf2itrf`/`qitrf2gcrf`
  (full IERS-2010, sub-arcsec); `qteme2gcrf` (~arcsec — fine for
  bootstrapping an `OrbitState` from an SGP4 sample; rotating the TEME
  velocity by the same quaternion is correct, both frames quasi-inertial).
  Apply as `q * v`.
- `orbitprop::propagate(&SimpleState, &begin, &end, &settings, satprops)`
  returns dense output — sample with `result.interp_batch(&[Instant])`, one
  propagate + one batch, never a per-sample loop.
- **EOP valid range: 1962-01-01 to the last `EOP-All.csv` entry (~build
  date).** Below 1962 EOP lookups silently return zeros; above the last
  entry satkit silently extrapolates. See `scenes.md` for the scene-epoch
  rule.
- `data_found()` gotcha: checks the *full* data bundle, so it returns `false`
  even when the ephemeris is present — do not gate on it.
