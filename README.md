# Globe

An interactive 3D Earth viewer, in the spirit of Google Earth, written
in Rust with [wgpu](https://wgpu.rs) for rendering, [winit](https://github.com/rust-windowing/winit)
for windowing and input, and [egui](https://github.com/emilk/egui) for
the control overlay.

## Building and running

```sh
cargo run --release
```

That's it - the required textures (NASA-derived earth maps and a star
field from [Solar System Scope](https://www.solarsystemscope.com/textures/))
are downloaded automatically into `assets/` on the first build, so the
first build needs a network connection and takes a little longer.

## Controls

| Input | Action |
|-------|--------|
| Left mouse drag | Pan around the globe (flick to spin with inertia) |
| Scroll wheel | Zoom in and out |
| Right mouse drag | Tilt toward the horizon |
| Sun sliders (top left) | Move the sun: latitude = season, longitude = time of day |
| Play/Pause + speed slider | Freeze time, or set how fast it passes (real time to 100x, exponential) |

## What it does

- A fully lit Earth: day and night sides with city lights, terrain
  relief, and a sun glint on the oceans.
- A physically based atmosphere with realistic sky colors - watch the
  terminator band glow orange at sunset, and the blue halo hug the
  planet's edge.
- The sun and a star backdrop that stay in sync as you move the camera
  and the time-of-day sliders.
- Smooth, Google-Earth-style navigation: panning follows the cursor at
  any zoom level, from full-globe spins down to country level.
- Real-world geometry: the globe is the WGS84 reference ellipsoid and the
  scene is modeled in kilometers, so it can host real-scale orbital
  simulation.
- Satellite tracking: a TLE (the ISS) is propagated with SGP4 (via the
  [satkit](https://crates.io/crates/satkit) crate) and shown as a marker on
  the globe. A simulation clock advances time (play/pause and a real-time to
  exponential real-time to 100x speed slider), so the marker orbits live; the panel shows the current
  datetime and the station's ground position.
