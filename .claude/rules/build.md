---
paths:
  - "crates/engine/build.rs"
  - "Cargo.toml"
  - "crates/engine/Cargo.toml"
---

# Build pipeline rules

- **Everything the runtime `include_bytes!`-es lands in `OUT_DIR`** (no
  `assets/` dir). `cargo::rerun-if-changed` is emitted per file, so deleting
  one triggers a re-download/re-bake. The asset list is the `EMBEDS` table in
  `build.rs` (engine's; `engine-astrodynamics` keeps its own satkit-only
  twin); the texture-filename<->body mapping is owned by
  `CelestialBody::maps()` and must stay in `IMPOSTOR_BODIES` order.
- **The atmosphere LUT bake runs unconditionally** (sub-second) so the tables
  never go stale after a constants tweak; the LUTs are the only baked
  artifacts — everything else downloads verbatim.
- **No build-side image decode or compression.** Textures decode at runtime;
  `intel_tex_2`/BC7 were removed for multiplatform support (see `backlog.md`
  before re-adding).
- **C toolchain required** (`ring` via `ureq`, build-time only). No pure-Rust
  workaround without replacing ureq's TLS.
- The `[profile.dev.package.*] opt-level = 3` overrides in `Cargo.toml` exist
  to speed the runtime decode of the 8K textures in dev builds.
