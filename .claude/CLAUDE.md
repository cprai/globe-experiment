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
Terra in world-space km, Hillaire-2020 atmosphere, star/Sol/**Luna** from JPL
DE440 ephemeris + real EOP, satellite TLE tracking via satkit SGP4, inertial
(star-fixed) camera that orbits a selectable **target** (Terra, Luna, or
any of the **seven planets** in the `solar_system` scenario via a body-selector
panel (one key per body); Luna in the eclipse scenarios via a Terra/Luna
panel; headless
`render` picks the body with `camera.target` "terra"/"luna"/"mars"/... ),
simulation clock (1x-100x, plays from launch). Luna is
a triaxial ellipsoid at true scale/distance, oriented by the full IAU lunar
rotation (correct near side + libration), lit by Sol, with **mutual
Terra/Luna eclipse shadows** (solar-eclipse spot on Terra, lunar-eclipse "blood-red
Luna"). The **seven planets** (`src/planet.rs`) are oblate ellipsoids at true
geocentric position/scale (DE440), oriented by the IAU planet rotation and
sun-lit with simple Lambert. Each is drawn **as a single shader impostor** (no
mesh): the CPU projects the planet center to screen space in `prepare` and the
GPU draws one camera-facing quad whose fragment shader ray-traces the oblate
ellipsoid (textured + Lambert-lit, writing per-fragment depth so planets occlude
each other and Terra occludes them). The trace is **distance-adaptive**:
perspective (eye-ray, reconstructed via `inv_view_proj`) for a near/orbited
planet, orthographic (parallel-ray, f32-safe) for a distant one — classified per
frame by apparent angular size. Because they sit millions-to-billions of km out
(past f32 precision), **all rendering is done in a camera-target-local "render
frame"**: positions are expressed relative to the camera target's center, so
the orbited body sits at a bit-exact zero and far planets do not jitter. The
renderer derives every body's position/orientation from the frame's **time**
(`CelestialSphere::at`); `RenderState` carries only the time, the camera rig, the
camera target, and satellite markers. There is no Earth-fixed origin or
`sol_dir` in the render path — every body is lit from Sol *position*. The Terra
surface/atmosphere/Luna/markers draw only when orbiting Terra/Luna; orbiting a
planet, only the planets + backdrop draw. For Terra/Luna (render origin at Terra)
geometry stays bit-identical. A **reversed-Z depth buffer** (Depth32Float) makes
Terra occlude Luna. **Past scenarios only** (before build date) — what makes full EOP accuracy
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
      "ui":[{"anchor":"top_left","offset":[10,10],"size":[340,190],
        "elements":[{"header":{"position":[0,0],"title":"Time / Subsolar"}},
                    {"readout":{"position":[0,26],"label":"UTC","value":"12:30:00"}},
                    {"dual_readout":{"position":[0,74],"left_label":"Lat",
                      "left_value":"-21","left_unit":"deg","right_label":"Lon",
                      "right_value":"-5","right_unit":"deg"}},
                    {"toggle":{"position":[0,124],"label":"Run","active":true}},
                    {"lamp":{"position":[150,126],"label":"Signal","status":"ok"}},
                    {"slider":{"position":[0,158],"value":0.5,"range":[0,4.6]}}]}]}'
```

First build: slow (~1.5 min extra), needs network. `build.rs` downloads 13
textures (JPEG/TIFF verbatim: Terra x4, stars, Luna, + 7 planets — five 8K,
two 2K), the JPL ephemeris (~98 MB), `EOP-All.csv`, and the three IERS-2010
tables into `OUT_DIR`; bakes 3 atmosphere LUTs as f16 KTX2. Subsequent builds reuse cached files. Delete a file in
`OUT_DIR` to re-download it. **VRAM** is now ~1.5 GB (the seven native-res
planet textures add ~686 MB; see `constraints.md`).

**WGSL is compiled by naga at runtime, not during `cargo build`.** Validate
after every shader edit:

```sh
naga --compact --capabilities none shaders/scene.wgsl
```
