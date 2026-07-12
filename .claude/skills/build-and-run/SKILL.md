---
name: build-and-run
description: Build and run the Solar System app with cargo run --release to see a change live - look, interaction feel, and pipeline-binding correctness. Use when asked to run, launch, start, or visually confirm the app, including running a specific scene.
---

# Build & run the app

Build and launch the Solar System app to see a change live (look, interaction feel,
pipeline-binding correctness).

## Tools
- `cargo` (stable toolchain — the build/run toolchain is stable; only
  formatting uses nightly)

## Command
```sh
cargo run --release
```

To run a specific scene (clap subcommand; a bare `scene` lists them):
```sh
cargo run --release -- scene iss_and_hubble
cargo run --release -- scene iss
cargo run --release -- scene          # lists available scenes
```

## Notes / gotchas
- **First build needs network and is slow (~1.5 min extra).** `build.rs`
  downloads five textures (BC7-encoded in memory), the ~98 MB JPL DE440
  ephemeris, and CelesTrak's `EOP-All.csv` into `OUT_DIR`. Subsequent
  builds reuse the cache.
- A clean `cargo build` proves **nothing** about `src/engine/shaders/scene.wgsl` — WGSL
  is compiled by naga at runtime, not during `cargo build`. Validate the
  shader separately (see the `validate-wgsl-naga` skill).
- Look and interaction feel can only be judged on a **native Windows
  release build**; the WSLg dev environment here can't validate exact colors
  or feel.
- **WSLg flakiness:** launch intermittently fails with libEGL/MESA errors —
  transient, retry; not a code bug.
