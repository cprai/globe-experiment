# Deliberately rejected — do not re-add

- **Animated/time-varying ocean wave noise** — static by design (temporal
  stability + idle-is-free).
- **Day-side albedo saturation boost** and **`OCEAN_TINT`** water-darkening
  tint — both reverted; albedo is used as sampled.
- **Sun-attached celestial sphere** (the slider-era model) — replaced by
  ephemeris-driven (2026-06-18). Do not reintroduce the old non-physical
  model.
- **Noise frequency ramp toward the terminator** for the city-light dissolve
  — boils/fizzes under sun motion. `DITHER_SCALE` is fixed.
- **Terminator-varying emissive threshold** — rejected (blob popping). Use
  the uniform threshold + dither.
- **Per-brightness glow scaling** — rejected; uniform glow strength.
- **`thread::scope` parallel texture decode** — reverted in phase 1. The
  sanctioned runtime decode is rayon inside `GlobeRenderer::new`; the
  rejected thing is specifically the `thread::scope` design.
- **`iced`** — removed in phase 2. Do not reintroduce it.
- **`intel_tex_2` / BC7 transcode at build time** — removed in phase 14 for
  multiplatform support. See `backlog.md` for how to re-add it properly if
  ever needed.
- **`assets/` directory** — deleted in phase 12. Everything in `OUT_DIR`.
- **`set_datadir` / runtime `data/` dir for satkit** — removed in phase 9.
  The ephemeris and EOP are embedded; no runtime data directory is needed.
- **`Sun` sliders driving the sun position** — replaced by ephemeris-driven
  subsolar point (read-only in the panel). Do not add interactive sun
  positioning.
- **Fixed-half-life always-glide zoom** — failed (laggy during active scroll).
- **Fixed burst-gap split zoom** — failed (momentum tail crosses threshold).
