---
name: smoke-test
description: Run Globe headless for ~20 seconds to confirm wgpu pipelines and bindings are valid - validation errors panic in the first frames, so a clean run means they are valid. Use after renderer, pipeline, or binding changes.
---

# Smoke test (validate pipelines & bindings)

Run the app headless for a few seconds to confirm wgpu pipelines and
bindings are valid. wgpu validation errors **panic in the first frames**,
so a clean 15-25 s run means pipelines/bindings are valid.

## Tools
- `cargo` (stable)

## Command
```sh
timeout 20 cargo run --release 2>&1 | head
```
Or redirect to a file (pipe buffering can swallow output):
```sh
timeout 20 cargo run --release > /tmp/smoke.log 2>&1; head -40 /tmp/smoke.log
```

## What it does / does not catch
- **Catches:** invalid pipelines, bad bind-group layouts, uniform/buffer
  mismatches — anything wgpu validates at pipeline creation or first draw.
- **Does NOT catch:** shader compile/validation issues are caught here too
  (naga runs at runtime), but you should still run the `validate-wgsl-naga`
  skill after a shader edit for a precise line+caret error.
- **Does NOT catch:** look/color/interaction-feel correctness — that needs a
  real interactive run, ideally a native Windows release build.

## Pass criteria
- No panic in the first frames; process runs until the `timeout` kills it.
- Note: the clock **starts playing**, so the app renders continuously and
  does not idle on its own during the smoke window — that is expected.
