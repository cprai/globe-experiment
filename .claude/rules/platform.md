# Platform compatibility (load-bearing — regressions are bugs)

- **Supported matrix: Windows/Linux/macOS x x86_64/aarch64** (six targets).
  Any regression narrows the matrix and is a bug.
- **`Features::empty()`** — do not re-add `TEXTURE_COMPRESSION_BC`: it panics
  on Apple Silicon (Metal exposes only ASTC/ETC2). Textures upload
  uncompressed.
- **No host-arch or OS assumptions in the build**: `build.rs` and deps must
  compile natively on all six targets — no prebuilt single-arch tools or
  platform-gated link flags without owner sign-off.
- **Every build machine needs Python 3 + dev library** (pyo3, unconditional).
  Windows/macOS builds need hardware confirmation.
- **Dev sandbox is x86_64 Linux + lavapipe** — it cannot prove macOS/aarch64
  behavior or input feel. Call out anything needing hardware confirmation
  before shipping.
- The `linux_` ephemeris filename is byte-order, not OS — little-endian DE440
  works on all six targets. Do not OS-gate it.

Windowing/startup invariants found by platform testing (Windows hidden-window
redraw, macOS Occluded-first-frame, main-thread event loop, WSL X11 forcing)
live in `renderer.md`.
