---
paths:
  - "src/**/*.rs"
  - "build.rs"
  - "shaders/globe.wgsl"
---

# Source format

- **All `.rs` files and `shaders/globe.wgsl` are pure ASCII.** Replace:
  `—` -> `-`, `deg` for `°`, `+/-` for `±`, `~` for `≈`, `x`/`*` for `×`.
  Markdown docs (`.md`) may use Unicode.
- **`cargo +nightly fmt` after every `.rs` edit.** Nightly is required for
  the `wrap_comments` option in `rustfmt.toml`; plain stable `cargo fmt`
  silently skips it. Never hand-format. After reflow, **scan diffs for
  formula-breaking line breaks** and reword to keep formulas on one line.
- **`wgslfmt shaders/globe.wgsl` after every shader edit.** Don't hand-format
  WGSL.
