# Hard constraints

- **`Features::empty()`** — no optional GPU features; textures uncompressed.
  Do not re-add any feature requirement. See `platform.md`.
- **No mipmaps** (`mip_level_count 1`). Known shimmer at far zoom; accepted.
- **8K textures at the portable limit.** `Limits::default()` guarantees
  `max_texture_dimension_2d = 8192` with zero headroom. Don't grow a texture
  past 8192 without raising the limit (narrows the platform matrix) or first
  downsizing the existing textures.
- **`max_sampled_textures_per_shader_stage = 16`** (default limit). Group 0
  holds **9** sampled textures (Earth x4, 3 LUTs, stars, Moon). The seven planet
  textures deliberately live in a **separate group-1 bind group** (one bound per
  planet draw), used only by the planet pipeline, so group 0 never grows toward
  16 and there is headroom (e.g. for Saturn's rings). Do not move planet
  textures into group 0.
- **~1.5 GB VRAM**: ~800 MB for the 9 group-0 textures (6 uncompressed 8K +
  3 LUTs) plus **~686 MB** for the seven native-res planet textures (five 8K
  ~134 MB each + two 2K ~8 MB each), plus a `Depth32Float` buffer at the window
  size. The planet textures upload at `SceneRenderer::new`, so **every** scenario
  pays this, even ISS. Accepted cost of native-res + no-feature portability; the
  lever if it bites is downsizing the planet textures to 4K/2K in `build.rs`.
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
