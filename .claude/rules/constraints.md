# Hard constraints

- **`Features::empty()`** — no optional GPU features; textures uncompressed.
  Do not re-add any feature requirement. See `platform.md`.
- **No mipmaps** (`mip_level_count 1`). Known shimmer at far zoom; accepted.
- **8K textures at the portable limit.** `Limits::default()` guarantees
  `max_texture_dimension_2d = 8192` with zero headroom. Don't grow a texture
  past 8192 without raising the limit (narrows the platform matrix) or first
  downsizing the existing textures.
- **`max_sampled_textures_per_shader_stage = 16`** (default limit). Group 0
  holds **4** sampled textures (3 LUTs, stars). Every body map (9 albedos +
  Terra's night/normal/specular) lives in the **per-body group-1 bind
  groups** (4 texture slots each, 1x1 dummies where a body has no optional
  map; one bound per body draw), used only by the impostor pipeline — worst
  fragment stage is 4 + 4 = 8, so nothing grows toward 16 and there is
  headroom (e.g. for Saturn's rings). Do not move body maps into group 0.
- **~1.5 GB VRAM**: ~140 MB for the 4 group-0 textures (8K stars + 3 LUTs)
  plus **~1.35 GB** for the twelve native-res impostor-body maps: ten 8K
  ~134 MB each (Terra's four + the Mercury/Venus/Mars/Jupiter/Saturn/Luna
  albedos) and two 2K ~8 MB each (Uranus/Neptune), plus a `Depth32Float`
  buffer at the window size. Same total as before the Terra-impostor fold
  (the identical 13 textures, redistributed from group 0 into group 1). The body maps upload at `SceneRenderer::new`, so
  **every** scene pays this, even ISS. Accepted cost of native-res +
  no-feature portability; the lever if it bites is downsizing the body
  textures to 4K/2K in `build.rs`.
- **No `.cargo/config.toml`** — deleted when `intel_tex_2` was removed; its
  `-lstdc++` was its only purpose. Do not re-add.
- **Build requires a C compiler** (`ring` via `ureq` in `build.rs`, build-
  time only). Portable across all six targets. No pure-Rust workaround exists
  without replacing `ureq`'s TLS (swapping to `aws-lc-rs` also uses C).
- **Build requires Python 3 with its dev library** (`pyo3`, unconditional —
  owner-approved 2026-07-07). Both binaries link libpython; only the `*_py`
  scenes ever initialize the interpreter. Override the probed interpreter
  with `PYO3_PYTHON` if needed.
- **No `assets/` dir, with ONE deliberate exception**: the repo-root
  `scenes/*.py` scene scripts are read at **runtime** (edit + relaunch, no
  rebuild — the point of the Python scenes). Resolved via
  `CARGO_MANIFEST_DIR` falling back to `./scenes` beside the binary.
- **WSLg flakiness**: transient libEGL/MESA errors on app launch — retry,
  not a code bug.
- **Windows `cargo add`**: can emit a bogus "found cargo.toml please rename"
  error — edit `Cargo.toml` directly and trust `cargo metadata`.
- **No `assets/` dir.** Everything in `OUT_DIR`, `include_bytes!`-ed at
  compile time. `build.rs` downloads verbatim into `OUT_DIR`; the LUTs are
  the only baked artifacts. (Runtime exception: the `scenes/*.py` scripts,
  above.)
