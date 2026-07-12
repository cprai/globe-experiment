# Hard constraints

- **`Features::empty()`** — no optional GPU features; textures uncompressed.
  See `platform.md` for why.
- **No mipmaps** (`mip_level_count 1`). Known shimmer at far zoom; accepted.
- **8K textures are at the portable limit** (`Limits::default()` guarantees
  `max_texture_dimension_2d = 8192`, zero headroom). Don't grow a texture
  past 8192.
- **Sampled-texture budget**: group 0 holds 4 sampled textures; every body
  map lives in the per-body group-1 bind groups (4 slots each, 1x1 dummies
  where absent), so the worst fragment stage sees 8 of the portable
  16-per-stage limit. Do not move body maps into group 0.
- **~1.5 GB VRAM** (native-res uncompressed body maps; every scene pays it,
  since maps upload in `SceneRenderer::new`). Accepted; the lever if it bites
  is downsizing textures in `build.rs` (see `backlog.md`).
- **No `.cargo/config.toml`** — do not re-add (existed only for the removed
  `intel_tex_2`).
- **Build requires a C compiler** (build-time TLS) **and Python 3 with dev
  library** (pyo3 embeds the interpreter, unconditional — owner-approved
  2026-07-07; `PYO3_PYTHON` overrides the probed interpreter).
- **No `assets/` dir** — everything in `OUT_DIR`, `include_bytes!`-ed. ONE
  deliberate exception: Python-paneled scenes (none shipped right now — see
  `scenes.md`) read their script at runtime from the path given via
  `--script`.
- **WSLg flakiness**: transient libEGL/MESA errors on launch — retry, not a
  code bug.
- **Windows `cargo add`** can emit a bogus "found cargo.toml please rename"
  error — edit `Cargo.toml` directly and trust `cargo metadata`.
