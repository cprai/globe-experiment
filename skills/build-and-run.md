# Skill: Build & run the app

Build and launch Globe to see a change live (look, interaction feel,
pipeline-binding correctness).

## Tools
- `cargo` (stable toolchain — the build/run toolchain is stable; only
  formatting uses nightly)

## Command
```sh
cargo run --release
```

To run a specific scenario (clap subcommand; a bare `scenario` lists them):
```sh
cargo run --release -- scenario iss_and_hubble
cargo run --release -- scenario iss
cargo run --release -- scenario          # lists available scenarios
```

## Notes / gotchas
- **First build needs network and is slow (~1.5 min extra).** `build.rs`
  downloads five textures (BC7-encoded in memory), the ~98 MB JPL DE440
  ephemeris, and CelesTrak's `EOP-All.csv` into `OUT_DIR`. Subsequent
  builds reuse the cache.
- A clean `cargo build` proves **nothing** about `shaders/globe.wgsl` — WGSL
  is compiled by naga at runtime, not during `cargo build`. Validate the
  shader separately (see `validate-wgsl-naga`).
- Look and interaction feel can only be judged on a **native Windows
  release build**; the WSLg dev environment here can't validate exact colors
  or feel.
- **WSLg flakiness:** launch intermittently fails with libEGL/MESA errors —
  transient, retry; not a code bug.
