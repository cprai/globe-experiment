---
paths:
  - "src/**/*.rs"
  - "build.rs"
  - "src/engine/shaders/scene.wgsl"
  - "scenes/**/*.py"
---

# Source format

- **All `.rs` files and `src/engine/shaders/scene.wgsl` are pure ASCII.** Replace:
  `—` -> `-`, `deg` for `°`, `+/-` for `±`, `~` for `≈`, `x`/`*` for `×`.
  Markdown docs (`.md`) may use Unicode.
- **`cargo +nightly fmt` after every `.rs` edit.** Nightly is required for
  the `wrap_comments` option in `rustfmt.toml`; plain stable `cargo fmt`
  silently skips it. Never hand-format. After reflow, **scan diffs for
  formula-breaking line breaks** and reword to keep formulas on one line.
- **`wgslfmt src/engine/shaders/scene.wgsl` after every shader edit.** Don't hand-format
  WGSL.
- **`ruff format` after every `scenes/*.py` edit** (the runtime scene
  scripts). Ruff is the formatting authority for Python; don't hand-format.
  See the `format-python` skill. (Type-check separately with `ty` — see the
  `check-python-ty` skill and `testing.md`.)
