# Solar System

An astronomically-accurate, interactive 3D solar-system simulation with
satellite tracking, written
in Rust with [wgpu](https://wgpu.rs) for rendering, [winit](https://github.com/rust-windowing/winit)
for windowing and input, and [egui](https://github.com/emilk/egui) for
the control overlay. It simulates **past** scenes (events before the
build date), which is what lets it use a fixed, never-changing record of
Earth-orientation data for full accuracy.

## Building and running

```sh
cargo run --release
```

That's it - the required textures (NASA-derived Terra maps plus Luna and
planet maps and a star field from [Solar System Scope](https://www.solarsystemscope.com/textures/)),
the JPL DE440 planetary ephemeris, and CelesTrak's Earth-orientation
parameters (`EOP-All.csv`) are downloaded automatically (into the build's
`OUT_DIR`) on the
first build, so the first build needs a network connection and takes a little
longer (the ephemeris is ~98 MB). The textures, ephemeris, and EOP data are all
embedded into the binary, so the build is self-contained and needs no data
files at runtime - except the optional Python scene scripts below.

Building also requires **Python 3** with its development library ([pyo3](https://pyo3.rs)
embeds the interpreter for the Python-scripted scenes), alongside the usual C
compiler.

## Rendering a single frame (the `headless` binary)

Besides the interactive window, a separate `headless` binary renders one frame
to an image file and exits - no window or input - which is handy for
scripted/visual debugging of the renderer. The whole scene is a single
`--scene` JSON; only the output target stays as CLI flags:

```sh
cargo run --release --bin headless -- --output frame.png --width 1920 --height 1080 \
    --scene '{
      "simulation": {"datetime": "2024-01-15T12:30:00Z"},
      "camera": {"longitude": -75, "latitude": 40, "distance": 12742, "tilt": 0}
    }'
```

The `--scene` JSON has a `simulation` section (`datetime`, RFC3339 UTC, fixes the
celestial positions), a `camera` section (`longitude`/`latitude`/`distance` in
km/`tilt`, plus an optional `target` of `"terra"` (default), `"luna"`, or any
planet (`"mercury"`, ..., `"neptune"`) to orbit that body instead — frame it with
a `distance` scaled to the body), and an optional `ui`
section that overlays mock UI panels for debugging UI layouts headlessly (see
`src/engine/ui/spec.rs`'s `UiPanel`). Unknown JSON keys are rejected. `--width`/`--height` default to 1920x1080. The frame is
written as a PNG and a short summary (resolved datetime, camera, output path)
is printed. Unlike the interactive scenes, the datetime here is **not**
range-checked against the bundled Earth-orientation data - times outside the
bundled range (before 1962 or after the build date) render but silently lose
accuracy, so use a past, in-range datetime for a faithful frame.

## Controls

| Input | Action |
|-------|--------|
| Left mouse drag | Pan around the scene (flick to spin with inertia) |
| Scroll wheel | Zoom in and out |
| Right mouse drag | Tilt toward the horizon |
| Play/Pause + speed slider | Freeze time, or set how fast it passes (real time to 100x, exponential) |

## What it does

- A fully lit Terra: day and night sides with city lights, terrain
  relief, and a Sol glint on the oceans.
- A physically based atmosphere with realistic colors - watch the
  terminator band glow orange at sunset, and the blue halo hug the
  planet's edge.
- Astronomically accurate Sol and celestial sphere: Sol's position, Terra's
  orientation, and the star backdrop are computed from the JPL DE440
  ephemeris (via the [satkit](https://crates.io/crates/satkit) crate) for the
  current simulated time, so the day/night terminator and the stars track real
  astronomy as time advances. The galactic-coordinate Milky Way texture is
  re-oriented to the equatorial sky by a fixed galactic->equatorial rotation, so
  the Milky Way crosses the sky at its true angle. The camera is fixed relative
  to the stars, so Terra visibly rotates beneath it.
- An astronomically-placed Luna: positioned from the same JPL DE440 ephemeris
  at its true distance and scale, shaped as a triaxial ellipsoid, and oriented
  by the full IAU lunar rotation model so the correct near side faces Terra
  (with real libration). It is lit by Sol with a hard terminator and the
  right phase, and Terra and Luna cast shadows on each other: Luna's
  shadow darkens a spot on Terra during a solar eclipse, and Terra's
  shadow turns Luna a dim coppery red during a lunar eclipse (a "blood-red
  Luna"). A depth buffer makes Terra correctly occlude the more distant Luna.
- The whole solar system: the `solar_system` scene draws the seven planets
  (Mercury through Neptune) at their true DE440 positions and scale, shaped as
  triaxial ellipsoids with equal equatorial axes - their familiar oblate forms,
  Saturn and Jupiter visibly flattened - oriented by the IAU
  planet rotation and lit by Sol with the correct phase. A body-selector
  panel (one key per body, ordered by distance from Sol) flies the camera to
  and orbits any of Terra, Luna, or a planet. Every body - Terra, the seven
  planets, and Luna alike - is drawn as a shader
  impostor: a single camera-facing quad whose fragment shader ray-traces the
  lit, textured triaxial ellipsoid, with no mesh at all - so the silhouette,
  rotation, terminator, and texture stay faithful; the trace adapts with
  distance (a true perspective eye-ray for the close body you are orbiting, a
  parallel-ray approximation for the far ones), and the shading scales per
  body from plain sun-lit texture up to Terra's full look (terrain relief,
  ocean glint, city lights, atmosphere). Because the outer planets sit
  billions of km away - beyond single-precision range - the scene is rendered
  relative to the orbited body so it stays crisp. (Saturn's rings are not yet
  drawn.)
- Real Earth-orientation parameters: satellite positions use measured polar
  motion and UT1-UTC (CelesTrak's `EOP-All.csv`), so the ground track is
  accurate to sub-arcsecond. This holds for past dates within the bundled EOP
  record (1962 onward); the tool simulates past scenes only.
- Smooth map-style navigation: panning follows the cursor at
  any zoom level, from whole-Terra spins down to country level. The camera orbits
  a chosen body - always Terra for satellite scenes, either Terra
  or Luna in the eclipse scenes (a Terra / Luna selector in the panel),
  and any of nine bodies in the solar-system scene (a key per body in the
  panel), with pan/tilt/zoom scaled to whichever body is targeted.
- Real-world geometry: Terra is the WGS84 reference ellipsoid and the
  scene is modeled in kilometers, so it can host real-scale orbital
  simulation.
- Satellite tracking: each tracked object's TLE (the ISS and Hubble) is
  propagated with SGP4 and shown as a marker on Terra, together with its
  predicted orbit path - the orbit one full period ahead, drawn as a smooth
  star-fixed line that hides behind Terra and fades out as it closes on the
  satellite. The path is predicted per object by either analytic SGP4 or
  numerical propagation of the object's current position and velocity (the
  latter needs no TLE - it is what the manually-controlled satellite flies
  on); the ISS + Hubble scene deliberately mixes both. A simulation clock
  advances time (play/pause and an exponential real-time to 100x speed slider),
  so Sol, stars, and every satellite all move live; the panel shows the
  current datetime, the subsolar point, and each object's ground position.
- Manual orbit control: the `manual_control` scene starts one satellite
  from the ISS orbit and hands you the thrusters. Hold a key in the Burns
  panel - prograde / retrograde, normal / anti-normal, radial out / radial
  in - and a (deliberately game-strength) thrust integrates into the
  numerically-propagated orbit for as long as you hold it, while the
  predicted orbit path and the apoapsis / periapsis / speed readouts respond
  live. Burn hard enough and you escape (the closed path disappears) or
  come back down.
- Python-scripted UI panels: the `manual_control_py` and `solar_system_py`
  scenes are clones of their Rust siblings whose control panels are produced
  by Python scripts (`scenes/manual_control_py.py` /
  `scenes/solar_system_py.py`, read at launch - edit a script and relaunch,
  no rebuild). The scripts drive the live scene through an embedded `globe`
  module exposing the same instrument/panel/clock/selector API as Rust; the
  two scene pairs live side by side so the APIs can be compared.
