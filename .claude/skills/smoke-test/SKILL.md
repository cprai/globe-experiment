---
name: smoke-test
description: Render one frame with the headless binary to confirm wgpu pipelines and bindings are valid - validation errors panic at pipeline creation or first draw, so a clean single-frame render means they are valid. Use after renderer, pipeline, or binding changes.
---

# Smoke test (validate pipelines & bindings)

Render a single frame with the `headless` binary. It builds the same shared
`renderer::SceneRenderer` as the windowed app — all pipelines and bind groups
are created in `SceneRenderer::new`, and wgpu validation errors **panic there
or at first draw** — so one clean frame means pipelines/bindings are valid.
No scene, no window, no display server needed (works in the dev sandbox
via lavapipe).

## Command

One call, one-line verdict; the log is shown only on failure:

```sh
cargo run -q --release -p engine --bin headless -- \
    --output /tmp/smoke.png --width 320 --height 240 --scene \
    '{"simulation":{"datetime":"2024-01-15T12:30:00Z"},"camera":{"longitude":-75,"latitude":40,"distance":12742,"tilt":0}}' \
    > /tmp/smoke.log 2>&1 \
  && echo "SMOKE PASS" \
  || { echo "SMOKE FAIL"; tail -30 /tmp/smoke.log; }; \
rm -f /tmp/smoke.png
```

## Do NOT analyze the image

The PNG is a byproduct, not the result. **Do not open, read, or analyze it**
(that wastes tokens) — the pass signal is the exit code alone; the command
deletes the image immediately. This is a smoke test looking only for obvious
errors (panics), not graphical quality. To actually inspect rendered output,
use the `analyze-render` skill instead.

## What it does / does not catch

- **Catches:** invalid pipelines, bad bind-group layouts, uniform/buffer
  mismatches, shader compile errors (naga runs at runtime) — anything wgpu
  validates at pipeline creation or first draw.
- **Does NOT catch:** the winit/`Gfx` surface-swapchain path, the windowed
  egui overlay, or tracked-body dot/trail draws (headless tracks no bodies)
  — those
  need a real windowed run on a machine with a display.
- **Does NOT catch:** look/color/interaction-feel correctness — that needs a
  real interactive run, ideally a native Windows release build. For shader
  errors with a precise line+caret, still run the `validate-wgsl-naga` skill.

## Pass criteria

- `SMOKE PASS` printed (headless exited 0). Nothing else to check.
- Benign noise: `error: XDG_RUNTIME_DIR is invalid or not set` lines in the
  log are expected in the sandbox and are NOT a failure.
