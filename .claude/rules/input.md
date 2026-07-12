---
paths:
  - "engine/src/camera/ptz.rs"
  - "engine/src/application/mod.rs"
---

# Input rules

All input state and response live in `PtzCamera` (winit press events carry no
position, so presses use the position last given to `pointer_move`). The
application keeps no input state — `translate_camera_event` maps each winit
event onto one `CameraControl` call, statelessly.

## Smoothed zoom (do not restructure; tune only the named constants)

The zoom is a **rate-adaptive glide with velocity bridging**: scroll events
move a *target* distance; `tick_zoom` eases toward it each frame in log
space. Adaptive half-life = EMA of inter-event gap — dense events get
near-instant response, sparse momentum-tail events interpolate across the
gap. Velocity bridging fills the stall at finger-lift; the next event repays
the bridged distance so total zoom equals exactly what the device sent.

Rejected designs that must not return: fixed-half-life always-glide (laggy
during active scroll) and fixed burst-gap split (a momentum tail decays
*through* the threshold).

## Cross-platform input

- Both winit wheel variants must map onto `ScrollDelta` (`LineDelta` ->
  `Lines`, `PixelDelta` -> `Pixels`) — dropping either kills scroll-zoom on
  half the platform matrix. Per-variant feel lives camera-side.
- The `PixelDelta.y / 60.0` trackpad divisor and right-drag tilt are
  **unvalidated on real macOS hardware** — tune on a real Mac before
  changing.
- Input uses `WindowEvent::CursorMoved`, **not** `DeviceEvent` raw motion —
  raw deltas are scaled/accelerated inconsistently across backends.
