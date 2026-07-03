---
name: analyze-render
description: Render a single frame via the separate `headless` binary and inspect the PNG to get visual feedback on rendering changes (lighting, terminator, atmosphere, framing). Use after shader/atmosphere/renderer edits to see the actual output.
---

# Analyze a render (visual feedback loop)

Render one frame to a PNG with the `headless` binary, then **open the PNG**
(with the Read tool) to judge the look. This is how to get real visual feedback
in this environment: the windowed app can't be eyeballed here, but the
`headless` binary writes an image an agent can actually inspect. Pick a
datetime + camera that frames the feature you changed.

## Tools
- `cargo` (stable) to run the app
- the Read tool to open the resulting PNG

## Command
```sh
cargo run --release --bin headless -- --output /tmp/render.png --width 1280 --height 720 \
    --scene '{
      "simulation": {"datetime": "2024-01-01T12:00:00Z"},
      "camera": {"longitude": <deg>, "latitude": <deg>,
                 "distance": <km>, "tilt": <deg>}
    }'
```
The whole scene is one `--scene` JSON: a `simulation` section (`datetime`) and a
`camera` section, both **required** (there is no default camera; unknown keys are
rejected). `camera.distance` is in kilometers (Terra mean radius is ~6371 km;
~12742 km gives a whole-Terra view). An optional `camera.target` (`"terra"`,
default, `"luna"`, or a planet) picks the orbit body; with `"luna"` the
lon/lat/distance/tilt are relative to Luna's surface (mean radius ~1737 km, so
frame it with a much smaller `distance`, e.g. ~2500-3500 km); the camera always
looks at the orbited body's center, so lon/lat only pick which side you view.
The output target (`--output`/`--width`/`--height`) stays as CLI flags. An
optional `ui` section overlays mock UI panels (see the
`build-and-run`/`src/engine/ui/` docs) - not usually needed for look analysis. Then
open `/tmp/render.png` with the Read tool and describe / compare what you see.

The command also prints a summary to stdout (resolved datetime, camera with the
clamped distance, output path). It does NOT print the subsolar point - estimate
the subsolar longitude from the UTC time (`lon ~ (12 - UTC_hours) * 15` deg,
positive east; latitude follows the season, +/-23.4 deg) when planning where
the day side and terminator should fall in the frame.

## Framing tips
- **Terminator / day-night edge:** aim at a longitude near the subsolar
  longitude +/- 90 deg; a low tilt shows the edge across the disc.
- **Atmosphere limb / sky glow:** high `camera.tilt` (toward the horizon) so the
  limb fills the frame.
- **Night side (city lights):** a longitude on the dark hemisphere.
- **Before/after:** render the change to two paths and inspect both.

## IMPORTANT: no EOP time-bound checking in the headless binary
The headless binary does **not** validate the datetime against the bundled
Earth-orientation (EOP) data (unlike scenarios). Out-of-range times silently
degrade and will mislead a visual analysis:
- before ~1962-01-01: satkit falls back to zero EOP (the Sol/stars drift);
- after the last bundled EOP entry (~build date): satkit constant-extrapolates.

Choose an in-range **past** datetime (1962-01-01 .. build date) for an accurate
frame. This is the agent's responsibility - the tool will not warn you.

## What it does / does not validate
- **Does:** the rendered look/color/lighting. The PNG reproduces the on-screen,
  non-sRGB LDR output (the offscreen target is non-sRGB `Rgba8Unorm` exactly so
  the bytes match the window), so colors are trustworthy.
- **Does NOT:** interaction feel (pan/zoom/inertia) - that still needs a native
  windowed run. Satellite markers are absent in the headless binary by design;
  the egui UI is absent unless the scene supplies a `ui` section (mock panels
  for layout debugging, no live data).

## Cleanup
Delete any temp images you created once you are done inspecting them - they are
throwaway visual-feedback artifacts, not project files, and should not be left
behind (or accidentally committed). Prefer writing them under `/tmp` (e.g.
`/tmp/render.png`) so they stay out of the repo; if you wrote one inside the
repo, `rm` it after the analysis.

## Gotchas
- Needs a working GPU/driver, like the windowed app. In a headless box with no
  Vulkan/GL driver, adapter creation panics with `NotFound { active_backends:
  0x0, ... }` - that is a missing-driver environment, not a code bug.
- Two `XDG_RUNTIME_DIR is invalid or not set in the environment.` lines may
  print to stderr from the GPU stack init. They are harmless noise and do not
  affect the render or the saved PNG; ignore them.
- First build is slow and needs network (downloads textures + the ~98 MB
  ephemeris + EOP); subsequent builds reuse the cache.
