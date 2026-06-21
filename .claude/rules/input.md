---
paths:
  - "src/application/input.rs"
---

# Input controller rules

## Smoothed zoom (do not restructure)

**Do not restructure the smoothed-zoom glide/coast in `input.rs`.** Tune only
the named constants:
- `ZOOM_HALF_LIFE_MIN` / `ZOOM_HALF_LIFE_MAX` — adaptive glide half-life range
- `ZOOM_COAST_HALF_LIFE` — velocity-bridging coast decay
- `ZOOM_STOP_RATE` — settle threshold
- `WHEEL_GAP_CAP` — EMA cap on inter-event gap

**Rejected designs that must not return:**
- Fixed-half-life always-glide — laggy during active scroll.
- Fixed burst-gap split (instant if gap < 0.05 s, glide otherwise) — failed
  because a momentum tail starts dense and decays *through* the threshold.

## Design summary (why the current design exists)

The zoom is a **rate-adaptive glide with velocity bridging**. Wheel events
move a *target* distance; `tick_zoom` eases the camera toward it each frame
in log space (`delta = ticks * ln(0.9)`). Adaptive half-life = EMA of
inter-event gap, clamped to `[ZOOM_HALF_LIFE_MIN, ZOOM_HALF_LIFE_MAX]` — dense
events (~10 ms) get near-instant; sparse events (momentum tail) interpolate
across the gap. Velocity bridging (`Zoom.velocity` EMA) fills the stall at
finger-lift where momentum events haven't arrived yet; the next event repays
the bridged distance so total zoom equals exactly what the device sent.

## Cross-platform input

- **Both `LineDelta` and `PixelDelta` must be handled** in the `MouseWheel`
  arm. Dropping either kills scroll-zoom on half the matrix (Windows/X11 =
  `LineDelta`; macOS precision trackpads = `PixelDelta`).
- **`PixelDelta.y` / 60.0 divisor is unvalidated on real macOS hardware.**
  It sets the trackpad zoom rate and has only been felt on Windows/X11.
  Tune on a real Mac before changing it.
- **Tilt on right-drag** (`MouseButton::Right`) is awkward on a trackpad-only
  Mac (ctrl-/two-finger secondary-click). macOS input feel is unvalidated
  (no native hardware available in the dev sandbox).
- Input uses `WindowEvent` (`CursorMoved`), **not** `DeviceEvent` raw motion
  — deliberate: raw deltas are scaled/accelerated inconsistently across
  backends.
