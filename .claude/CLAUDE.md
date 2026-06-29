# CLAUDE.md

Project rules and conventions for **Solar System**, an astronomically-accurate
solar-system renderer with satellite tracking (past scenarios only). Rules are in
`.claude/rules/` — topic files load at launch or when you open matching
files. Read all loaded rules before making changes.

**When `.claude/rules/*.md` and the source disagree, the source wins.**
Look-tuning constants in `scene.wgsl` in particular drift between sessions.

**After completing all changes, print a one-line commit message describing the
currently uncommitted changes — as an example only. Do NOT make any commits.**

---

## What this is

Rust (edition 2024), winit 0.30, wgpu 29, egui 0.34. Physically-lit WGS84
Terra in world-space km, Hillaire-2020 atmosphere, star/sol/**luna** from JPL
DE440 ephemeris + real EOP, satellite TLE tracking via satkit SGP4, inertial
(star-fixed) camera that orbits a selectable **target** (Terra, Luna, or
any of the **seven planets** in the `solar_system` scenario via a body-selector
panel (one key per body); Luna in the eclipse scenarios via a TERRA/LUNA
panel; headless
`render` picks the body with `camera.target` "terra"/"luna"/"mars"/... ),
simulation clock (1x-100x, plays from launch). Luna is
a triaxial ellipsoid at true scale/distance, oriented by the full IAU lunar
rotation (correct near side + libration), lit by Sol, with **mutual
Terra/Luna eclipse shadows** (solar-eclipse spot on Terra, lunar-eclipse "blood-red
Luna"). The **seven planets** (`src/planet.rs`) are oblate ellipsoids at true
geocentric position/scale (DE440), oriented by the IAU planet rotation and
sun-lit with simple Lambert. Each is drawn **as a mesh only when it is large
enough on screen** (apparent angular diameter `>=` a threshold, classified per
frame in `prepare`); a planet smaller than that — every planet from Terra, and
the non-orbited planets generally — is drawn as a **billboard impostor**: a
camera-facing quad whose fragment shader ray-traces the same oblate ellipsoid
(orthographic / parallel-ray, f32-safe at distance), still textured + Lambert-lit,
so the silhouette/terminator/texture stay faithful while the mesh draws are
skipped. Because they sit millions-to-billions of km out
(past f32 precision), **all rendering is done in a camera-target-local "render
frame"**: every position is uploaded relative to the camera target's center, so
the orbited body sits at a bit-exact zero and far planets do not jitter. There
is no Earth-fixed origin or `sol_dir` in the render path — every body is lit from
Sol *position*. The Terra surface/atmosphere/Luna/markers draw only when
orbiting Terra/Luna; orbiting a planet, only the planets + backdrop draw. For
Terra/Luna (render origin at Terra) geometry stays bit-identical. A
**reversed-Z depth buffer** (Depth32Float) makes Terra occlude Luna. **Past scenarios only** (before build date) — what makes full EOP accuracy
attainable. The crate is named `globe-experiment`; `iced` is gone, do not
reintroduce it. **Saturn's rings are not yet rendered** (deferred).

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

First build: slow (~1.5 min extra), needs network. `build.rs` downloads 13
textures (JPEG verbatim: Terra x4, stars, Luna, + 7 planets — five 8K, two 2K),
the JPL ephemeris (~98 MB), and `EOP-All.csv` into `OUT_DIR`; bakes 3 atmosphere
LUTs as f16 KTX2. Subsequent builds reuse cached files. Delete a file in
`OUT_DIR` to re-download it. **VRAM** is now ~1.5 GB (the seven native-res
planet textures add ~686 MB; see `constraints.md`).

**WGSL is compiled by naga at runtime, not during `cargo build`.** Validate
after every shader edit:

```sh
naga --compact --capabilities none shaders/scene.wgsl
```
