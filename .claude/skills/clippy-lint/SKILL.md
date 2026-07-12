---
name: clippy-lint
description: Lint Rust with cargo clippy to catch misuse, redundancy, and footguns the bare compiler misses. Use after every .rs change; aim warning-free. Note clippy does not validate the WGSL shader.
---

# Lint Rust with clippy

Check correctness with clippy, not just `cargo build`. clippy catches
misuse, redundancy, and footguns the bare compiler misses. Run it heavily
and aim for warning-free.

## Tools
- `cargo clippy` (stable)

## Command
```sh
cargo clippy --release
```
Stricter (treat warnings as errors) once clean:
```sh
cargo clippy --release -- -D warnings
```

## Notes
- Run after **every** `.rs` change.
- Caveat: clippy (like `cargo build`) never compiles `src/engine/shaders/scene.wgsl` —
  it says nothing about the shader. Validate the shader separately with the
  `validate-wgsl-naga` skill.
- There is **no test suite and no CI** in this repo. Verification is clippy
  + the smoke test + manual interaction on native Windows.
