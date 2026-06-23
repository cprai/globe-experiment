# Platform compatibility (load-bearing — regressions are bugs)

- **Supported matrix: Windows/Linux/macOS x both x86_64 and aarch64** (six
  targets). Any regression narrows this matrix and is a bug, same as an
  accuracy regression.
- **`Features::empty()`** — no optional GPU features. Textures upload
  **uncompressed** (`Rgba8Unorm`/`Rgba8UnormSrgb`). Do not re-add
  `TEXTURE_COMPRESSION_BC` — it panics on Apple Silicon (Metal exposes only
  ASTC/ETC2, not BC/S3TC).
- **No host-arch or OS assumptions in the build.** `build.rs` and deps must
  compile natively on all six targets (including aarch64). No single-arch
  prebuilt build tools or platform-gated link flags without owner sign-off.
  `intel_tex_2` was deleted for this reason and must not return.
- **Dev sandbox is x86_64 Linux + lavapipe** — cannot prove macOS/aarch64
  behavior. Reason portability through against this rule; call out anything
  that needs hardware confirmation before shipping.

## Platform-specific invariants (found by testing)

- **winit event loop must run on the main thread.** `main` calls
  `application::run` directly. Moving it to a spawned thread panics on macOS.
- **`resumed()` re-entry guard.** winit can fire `resumed` more than once;
  `ApplicationState::resumed` guards with `if self.gfx.is_some()`.
- **Windows: no `RedrawRequested` to a hidden window.** This is why
  `ApplicationState::resumed()` calls `self.redraw()` directly rather than
  `request_redraw()` — the reveal code inside `redraw` would never run if we
  waited for the event. See `renderer.md`.
- **macOS: first `get_current_texture` may return `Occluded`** for a still-
  hidden window. The `Occluded`-before-`shown` guard shows the window and
  retries rather than deadlocking invisible. See `renderer.md`.
- **`linux_` ephemeris filename is byte-order, not OS.** JPL's little-endian
  DE440 works on all six targets (all are little-endian). Do not OS-gate it.
- **WSL + X11: winit defaults to Wayland, not X11.** Even when `DISPLAY` is
  set, winit picks Wayland if `WAYLAND_DISPLAY` is also present (common in
  WSL2). Vulkan may be absent or broken under Wayland in WSL; the GL/EGL
  backend is the reliable fallback. `Gfx::init` passes
  `OwnedDisplayHandle` to `InstanceDescriptor` so EGL can initialize; without
  this the app panics with "no GPU adapter found". See `renderer.md`.
