# Globe

An astronomically-accurate satellite simulation tool, built on an
interactive 3D Earth renderer in the spirit of Google Earth, written
in Rust with [wgpu](https://wgpu.rs) for rendering, [winit](https://github.com/rust-windowing/winit)
for windowing and input, and [egui](https://github.com/emilk/egui) for
the control overlay. It simulates **past** scenarios (events before the
build date), which is what lets it use a fixed, never-changing record of
Earth-orientation data for full accuracy.

## Building and running

```sh
cargo run --release
```

That's it - the required textures (NASA-derived earth maps and a star
field from [Solar System Scope](https://www.solarsystemscope.com/textures/)),
the JPL DE440 planetary ephemeris, and CelesTrak's Earth-orientation
parameters (`EOP-All.csv`) are downloaded automatically (into the build's
`OUT_DIR`) on the
first build, so the first build needs a network connection and takes a little
longer (the ephemeris is ~98 MB). The textures, ephemeris, and EOP data are all
embedded into the binary, so the build is self-contained and needs no data
files at runtime.

## Rendering a single frame (headless)

Besides the interactive window, the tool can render one frame to an image file
and exit - no window or input - which is handy for scripted/visual debugging of
the renderer. The whole scene is a single `--scene` JSON; only the output target
stays as CLI flags:

```sh
cargo run --release -- render --output frame.png --width 1920 --height 1080 \
    --scene '{
      "simulation": {"datetime": "2024-01-15T12:30:00Z"},
      "camera": {"longitude": -75, "latitude": 40, "distance": 12742, "tilt": 0}
    }'
```

The `--scene` JSON has a `simulation` section (`datetime`, RFC3339 UTC, fixes the
celestial positions), a `camera` section (`longitude`/`latitude`/`distance` in
km/`tilt`, plus an optional `target` of `"earth"` (default), `"moon"`, or any
planet (`"mercury"`, ..., `"neptune"`) to orbit that body instead — frame it with
a `distance` scaled to the body), and an optional `ui`
section that overlays mock UI panels for debugging UI layouts headlessly (see
`src/ui/spec.rs`'s `UiPanel`). Unknown JSON keys are rejected. `--width`/`--height` default to 1920x1080. The frame is
written as a PNG and a short summary (resolved datetime, subsolar point, camera)
is printed. Unlike the interactive scenarios, the datetime here is **not**
range-checked against the bundled Earth-orientation data - times outside the
bundled range (before 1962 or after the build date) render but silently lose
accuracy, so use a past, in-range datetime for a faithful frame.

## Controls

| Input | Action |
|-------|--------|
| Left mouse drag | Pan around the globe (flick to spin with inertia) |
| Scroll wheel | Zoom in and out |
| Right mouse drag | Tilt toward the horizon |
| Play/Pause + speed slider | Freeze time, or set how fast it passes (real time to 100x, exponential) |

## What it does

- A fully lit Earth: day and night sides with city lights, terrain
  relief, and a sun glint on the oceans.
- A physically based atmosphere with realistic colors - watch the
  terminator band glow orange at sunset, and the blue halo hug the
  planet's edge.
- Astronomically accurate Sun and celestial sphere: the Sun's position, Earth's
  orientation, and the star backdrop are computed from the JPL DE440
  ephemeris (via the [satkit](https://crates.io/crates/satkit) crate) for the
  current simulated time, so the day/night terminator and the stars track real
  astronomy as time advances. The galactic-coordinate Milky Way texture is
  re-oriented to the equatorial sky by a fixed galactic->equatorial rotation, so
  the Milky Way crosses the sky at its true angle. The camera is fixed relative
  to the stars, so the Earth visibly rotates beneath it.
- An astronomically-placed Moon: positioned from the same JPL DE440 ephemeris
  at its true distance and scale, shaped as a triaxial ellipsoid, and oriented
  by the full IAU lunar rotation model so the correct near side faces Earth
  (with real libration). It is lit by the Sun with a hard terminator and the
  right phase, and the Earth and Moon cast shadows on each other: the Moon's
  shadow darkens a spot on the Earth during a solar eclipse, and the Earth's
  shadow turns the Moon a dim coppery red during a lunar eclipse (a "blood
  moon"). A depth buffer makes the Earth correctly occlude the more distant Moon.
- The whole solar system: the `solar_system` scenario draws the seven planets
  (Mercury through Neptune) at their true DE440 positions and scale, shaped as
  oblate ellipsoids (Saturn and Jupiter visibly flattened), oriented by the IAU
  planet rotation and lit by the Sun with the correct phase. A body-selector
  panel (one key per body, ordered by distance from the Sun) flies the camera to
  and orbits any of Earth, the Moon, or a planet. Because the
  outer planets sit billions of km away - beyond single-precision range - the
  scene is rendered relative to the orbited body so it stays crisp. (Saturn's
  rings are not yet drawn.)
- Real Earth-orientation parameters: satellite positions use measured polar
  motion and UT1-UTC (CelesTrak's `EOP-All.csv`), so the ground track is
  accurate to sub-arcsecond. This holds for past dates within the bundled EOP
  record (1962 onward); the tool simulates past scenarios only.
- Smooth, Google-Earth-style navigation: panning follows the cursor at
  any zoom level, from full-globe spins down to country level. The camera orbits
  a chosen body - always the Earth for satellite scenarios, either the Earth
  or the Moon in the eclipse scenarios (an EARTH / MOON selector in the panel),
  and any of nine bodies in the solar-system scenario (a key per body in the
  panel), with pan/tilt/zoom scaled to whichever body is targeted.
- Real-world geometry: the globe is the WGS84 reference ellipsoid and the
  scene is modeled in kilometers, so it can host real-scale orbital
  simulation.
- Satellite tracking: each tracked object's TLE (the ISS and Hubble) is
  propagated with SGP4 and shown as a marker on the globe. A simulation clock
  advances time (play/pause and an exponential real-time to 100x speed slider),
  so the Sun, stars, and every satellite all move live; the panel shows the
  current datetime, the subsolar point, and each object's ground position.
