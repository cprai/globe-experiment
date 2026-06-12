# Globe

An interactive 3D Earth viewer, in the spirit of Google Earth, written
in Rust with [iced](https://iced.rs) for the GUI and
[wgpu](https://wgpu.rs) for rendering.

## Building and running

```sh
cargo run --release
```

That's it — the required textures (NASA-derived earth maps and a star
field from [Solar System Scope](https://www.solarsystemscope.com/textures/))
are downloaded automatically into `assets/` on the first build, so the
first build needs a network connection and takes a little longer.

## Controls

| Input | Action |
|-------|--------|
| Left mouse drag | Pan around the globe (flick to spin with inertia) |
| Scroll wheel | Zoom in and out |
| Right mouse drag | Tilt toward the horizon |
| Sliders (top left) | Move the sun: latitude = season, longitude = time of day |

## What it does

- A fully lit Earth: day and night sides with city lights, terrain
  relief, and a sun glint on the oceans.
- A physically based atmosphere with realistic sky colors — watch the
  terminator band glow orange at sunset, and the blue halo hug the
  planet's edge.
- The sun and a star backdrop that stay in sync as you move the camera
  and the time-of-day sliders.
- Smooth, Google-Earth-style navigation: panning follows the cursor at
  any zoom level, from full-globe spins down to country level.

## Project layout

- `src/main.rs` — the iced application: window, sliders, messages.
- `src/globe/` — the globe widget: camera, input handling, sun model,
  and the wgpu rendering pipelines.
- `shaders/globe.wgsl` — all shader code: surface, atmosphere, and
  star/sun backdrop.
- `build.rs` — downloads the textures on first build.
- `claude/PLAN.md` — the original implementation plan and milestones.
- `claude/OPTIMIZE.md` — notes on improving startup performance.
