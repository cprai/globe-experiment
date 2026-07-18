---
name: validate-wgsl-naga
description: Statically validate crates/engine/src/shaders/scene.wgsl with the naga CLI (naga --compact --capabilities none). This is the authoritative shader check - the same naga the app links through wgpu. Use after every shader edit; a clean cargo build says nothing about the shader.
---

# Validate WGSL with the naga CLI (authoritative)

Statically validate `crates/engine/src/shaders/scene.wgsl` after **every** shader edit. This
is the authoritative static check: the CLI is the **same naga** the app
links through wgpu (CLI 29.x <-> the `wgpu`/`naga` 29.x in `Cargo.lock`), so
it runs the exact frontend + IR validator that would otherwise only fire at
runtime. A clean `cargo build`/`clippy` says nothing about the shader —
naga only runs at app runtime.

## Tools
- `naga` CLI (keep its version aligned with the linked naga — currently
  29.x; bump it when wgpu bumps)

## Command (strictest invocation)
```sh
naga --compact --capabilities none crates/engine/src/shaders/scene.wgsl
```

## What the flags buy
- **No output file** => *validate only* (don't emit a translation).
- Default `--validate` is already every `ValidationFlags` bit, so it is left
  implicit (auto-tracks any flags naga adds — don't hardcode the `63`
  bitmask).
- `--compact` runs a second validation pass over the compacted IR.
- `--capabilities none` forbids every *optional* capability (float16,
  subgroup ops, dual-source blending, ...). The shader uses only baseline
  features, so `none` passes today and turns any future capability-gated
  feature into a deliberate, visible decision.

## Reading the result
- `Validation successful` + exit 0 = good.
- Parse/type/validation errors print with a **line + caret** and exit 1. It
  catches real semantics, not just syntax (e.g. `let x: f32 = 1u;` is
  reported as a type mismatch).

## Important
- This static check is necessary but **not sufficient**: it cannot see
  look or pipeline-binding correctness. You must still actually run the app
  (see the `build-and-run` / `smoke-test` skills) after a shader change.
