# CLAUDE.md

Project rules and conventions for **Globe**, an astronomically-accurate
satellite simulation tool (past scenarios only). Rules are in
`.claude/rules/` — topic files load at launch or when you open matching
files. Read all loaded rules before making changes.

**When `.claude/rules/*.md` and the source disagree, the source wins.**
Look-tuning constants in `globe.wgsl` in particular drift between sessions.

---

## What this is

Rust (edition 2024), winit 0.30, wgpu 29, egui 0.34. Physically-lit WGS84
globe in world-space km, Hillaire-2020 atmosphere, star/sun from JPL DE440
ephemeris + real EOP, satellite TLE tracking via satkit SGP4, inertial
(star-fixed) camera, simulation clock (1x-100x, plays from launch).
**Past scenarios only** (before build date) — what makes full EOP accuracy
attainable. The crate is named `globe-experiment`; `iced` is gone, do not
reintroduce it.

---

## Build & run

```sh
cargo run --release
cargo run --release -- render --datetime 2024-01-15T12:30:00Z \
    --longitude -75 --latitude 40 --distance 12742 --tilt 0 --output frame.png
# Overlay mock UI panels (debug UI layouts headlessly); JSON = Vec<ui::UiPanelSpec>:
cargo run --release -- render --datetime 2024-01-15T12:30:00Z \
    --longitude -75 --latitude 40 --distance 12742 --tilt 0 --output mock.png \
    --ui '[{"anchor":"top_left","offset":[10,10],"size":[300,130],
      "elements":[{"text":{"position":[0,0],"text":"Hi"}},
                  {"button":{"position":[0,60],"label":"Play"}},
                  {"slider":{"position":[0,94],"value":0.5,"range":[0,4.6]}}]}]'
```

First build: slow (~1.5 min extra), needs network. `build.rs` downloads 5
textures (JPEG/TIFF verbatim), the JPL ephemeris (~98 MB), and
`EOP-All.csv` into `OUT_DIR`; bakes 3 atmosphere LUTs as f16 KTX2.
Subsequent builds reuse cached files. Delete a file in `OUT_DIR` to
re-download it.

**WGSL is compiled by naga at runtime, not during `cargo build`.** Validate
after every shader edit:

```sh
naga --compact --capabilities none shaders/globe.wgsl
```
