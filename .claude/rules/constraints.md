# Hard constraints

- **`Features::empty()`** — no optional GPU features; textures uncompressed.
  Do not re-add any feature requirement. See `platform.md`.
- **No mipmaps** (`mip_level_count 1`). Known shimmer at far zoom; accepted.
- **8K textures at the portable limit.** `Limits::default()` guarantees
  `max_texture_dimension_2d = 8192` with zero headroom. Don't grow a texture
  past 8192 without raising the limit (narrows the platform matrix) or first
  downsizing the existing textures.
- **~670 MB VRAM** for 5 uncompressed 8K textures — accepted cost of the
  no-feature portability.
- **No `.cargo/config.toml`** — deleted when `intel_tex_2` was removed; its
  `-lstdc++` was its only purpose. Do not re-add.
- **Build requires a C compiler** (`ring` via `ureq` in `build.rs`, build-
  time only). Portable across all six targets. No pure-Rust workaround exists
  without replacing `ureq`'s TLS (swapping to `aws-lc-rs` also uses C).
- **WSLg flakiness**: transient libEGL/MESA errors on app launch — retry,
  not a code bug.
- **Windows `cargo add`**: can emit a bogus "found cargo.toml please rename"
  error — edit `Cargo.toml` directly and trust `cargo metadata`.
- **No `assets/` dir.** Everything in `OUT_DIR`, `include_bytes!`-ed at
  compile time. `build.rs` downloads verbatim into `OUT_DIR`; the LUTs are
  the only baked artifacts.
