---
name: analyze-render
description: Render a single frame headless via the `render` CLI mode and inspect the PNG to get visual feedback on rendering changes (lighting, terminator, atmosphere, framing). Use after shader/atmosphere/renderer edits to see the actual output.
---

# Analyze a render (visual feedback loop)

Render one frame to a PNG with `render` mode, then **open the PNG** (with the
Read tool) to judge the look. This is how to get real visual feedback in this
environment: the windowed app can't be eyeballed here, but the headless
`render` mode writes an image an agent can actually inspect. Pick a datetime +
camera that frames the feature you changed.

## Tools
- `cargo` (stable) to run the app
- the Read tool to open the resulting PNG

## Command
```sh
cargo run --release -- render \
    --datetime 2024-01-01T12:00:00Z \
    --longitude <deg> --latitude <deg> --distance <km> --tilt <deg> \
    --width 1280 --height 720 \
    --output /tmp/render.png
```
All four camera flags (`--longitude`, `--latitude`, `--distance`, `--tilt`) are
**required** - there is no default camera. `--distance` is in kilometers (Earth
mean radius is ~6371 km; ~12742 km gives a full-globe view). Then open
`/tmp/render.png` with the Read tool and describe / compare what you see.

The command also prints a summary to stdout (resolved datetime, subsolar
lat/lon, camera, output path) - read it for context on where the day side and
terminator should fall in the frame.

## Framing tips
- **Terminator / day-night edge:** aim at a longitude near the subsolar
  longitude +/- 90 deg; a low tilt shows the edge across the disc.
- **Atmosphere limb / sky glow:** high `--tilt` (toward the horizon) so the limb
  fills the frame.
- **Night side (city lights):** a longitude on the dark hemisphere.
- **Before/after:** render the change to two paths and inspect both.

## IMPORTANT: no EOP time-bound checking in render mode
Render mode does **not** validate the datetime against the bundled
Earth-orientation (EOP) data (unlike scenarios). Out-of-range times silently
degrade and will mislead a visual analysis:
- before ~1962-01-01: satkit falls back to zero EOP (the Sun/stars drift);
- after the last bundled EOP entry (~build date): satkit constant-extrapolates.

Choose an in-range **past** datetime (1962-01-01 .. build date) for an accurate
frame. This is the agent's responsibility - the tool will not warn you.

## What it does / does not validate
- **Does:** the rendered look/color/lighting. The PNG reproduces the on-screen,
  non-sRGB LDR output (the offscreen target is non-sRGB `Rgba8Unorm` exactly so
  the bytes match the window), so colors are trustworthy.
- **Does NOT:** interaction feel (pan/zoom/inertia) - that still needs a native
  windowed run. Markers and the egui UI are absent in render mode by design.

## Gotchas
- Needs a working GPU/driver, like the windowed app. In a headless box with no
  Vulkan/GL driver, adapter creation panics with `NotFound { active_backends:
  0x0, ... }` - that is a missing-driver environment, not a code bug.
- First build is slow and needs network (downloads textures + the ~98 MB
  ephemeris + EOP); subsequent builds reuse the cache.
