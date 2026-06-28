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
globe in world-space km, Hillaire-2020 atmosphere, star/sun/**moon** from JPL
DE440 ephemeris + real EOP, satellite TLE tracking via satkit SGP4, inertial
(star-fixed) camera that orbits a selectable **target** (Earth, or the Moon in
the eclipse scenarios via an EARTH/MOON panel; headless `render` picks the body
with `camera.target` "earth"/"moon"), simulation clock (1x-100x, plays from
launch). The Moon is
a triaxial ellipsoid at true scale/distance, oriented by the full IAU lunar
rotation (correct near side + libration), lit by the Sun, with **mutual
Earth/Moon eclipse shadows** (solar-eclipse spot on Earth, lunar-eclipse "blood
moon"). A **reversed-Z depth buffer** (Depth32Float) makes Earth occlude the
Moon. **Past scenarios only** (before build date) — what makes full EOP accuracy
attainable. The crate is named `globe-experiment`; `iced` is gone, do not
reintroduce it.

---

## Build & run

```sh
cargo run --release
# `render` takes ONE --scene JSON (simulation + camera + optional ui); the
# output target (--output, --width, --height) stays as CLI flags. Unknown JSON
# keys are rejected (deny_unknown_fields).
cargo run --release -- render --output frame.png --scene \
    '{"simulation":{"datetime":"2024-01-15T12:30:00Z"},
      "camera":{"longitude":-75,"latitude":40,"distance":12742,"tilt":0}}'
# Add a "ui" section (Vec<ui::UiPanel>) to overlay mock UI panels for
# headless UI-layout debugging:
cargo run --release -- render --output mock.png --scene \
    '{"simulation":{"datetime":"2024-01-15T12:30:00Z"},
      "camera":{"longitude":-75,"latitude":40,"distance":12742,"tilt":0},
      "ui":[{"anchor":"top_left","offset":[10,10],"size":[340,148],
        "elements":[{"header":{"position":[0,0],"title":"Time / Subsolar"}},
                    {"readout":{"position":[0,26],"label":"UTC","value":"12:30:00"}},
                    {"dual_readout":{"position":[0,52],"left_label":"Lat",
                      "left_value":"-21 deg","right_label":"Lon","right_value":"-5 deg"}},
                    {"toggle":{"position":[0,84],"label":"Run","active":true}},
                    {"lamp":{"position":[150,86],"label":"Signal","status":"ok"}},
                    {"slider":{"position":[0,114],"value":0.5,"range":[0,4.6]}}]}]}'
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
