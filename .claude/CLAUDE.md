# CLAUDE.md

Project rules and conventions for **Solar System**, an astronomically-accurate
solar-system renderer with satellite tracking (past scenarios only). Rules are in
`.claude/rules/` — topic files load at launch or when you open matching
files. Read all loaded rules before making changes.

**When `.claude/rules/*.md` and the source disagree, the source wins.**
Look-tuning constants in `scene.wgsl` in particular drift between sessions.

**After completing all changes, print a one-line commit message describing the
currently uncommitted changes — as an example only. Do NOT make any commits.**

## Code search: use the codebase-memory MCP (see the `codebase-search` skill)

Whenever code needs to be searched, or before planning/making major code
changes, **prefer the `codebase-memory-mcp` knowledge graph over grep/glob/
whole-file reads** — it returns exact symbols, callers, and precise source
ranges, saving agent token cost. **Re-index the project before searching, and
re-index again after any code change** (indexing is fast at this project's
size, so do it liberally). The `codebase-search` skill has the tool cheat
sheet and the re-index commands. Plain Grep/Read remain right for non-Rust
files (`scene.wgsl`, configs, docs), and always `Read` a file before editing.

---

## What this is

Rust (edition 2024), winit 0.30, wgpu 29, egui 0.34. Physically-lit WGS84
Terra in world-space km, Hillaire-2020 atmosphere, star/Sol/**Luna** from JPL
DE440 ephemeris + real EOP, satellite TLE tracking via satkit SGP4 (each
tracked satellite also draws its **predicted orbit path**: the marker carries
a `Propagation` - a cloned TLE element set or a GCRF state vector - and the
renderer propagates it one period ahead with the matching backend (analytic
SGP4, or numerical satkit `orbitprop` needing no TLE - what the
**manually-controlled satellite** flies on: the `manual_control` scenario
seeds one object from the ISS TLE, re-anchors its GCRF state vector to the
clock each frame, and its bottom-center **Burns panel** of six hold-to-fire
keys (prograde/retrograde, normal/anti-normal, radial out/in) integrates a
game-strength thrust into the velocity while held, with apo/peri/speed
readouts; a scene may mix both backends, and `iss_and_hubble`
does), rendering the star-fixed
inertial ellipse as a thick depth-tested line whose tail fades out sharply
near one full orbit), inertial
(star-fixed) camera that orbits a selectable **target** (Terra, Luna, or
any of the **seven planets** in the `solar_system` scenario via a body-selector
panel (one key per body); Luna in the eclipse scenarios via a Terra/Luna
panel; the `headless`
binary picks the body with `camera.target` "terra"/"luna"/"mars"/... ),
simulation clock (1x-100x, plays from launch). Luna is
a triaxial ellipsoid at true scale/distance, oriented by the full IAU lunar
rotation (correct near side + libration), lit by Sol, with **mutual
Terra/Luna eclipse shadows** (solar-eclipse spot on Terra, lunar-eclipse "blood-red
Luna"). The **seven planets** (`src/engine/planet.rs`) are oblate ellipsoids at true
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

The crate builds **two binaries over one shared `src/engine/`** (no lib
crate): `globe-experiment` (`src/main.rs`, the windowed app + scenarios) and
`headless` (`src/headless.rs`, the single-frame PNG renderer). Both bin roots
declare `mod engine;` (everything used to run the app: `application`, `camera`,
`luna`, `planet`, `renderer`, `simulation`, `terra`, `ui`); the trees differ
only at the top level — `scenarios` exists only in the main tree, `offscreen`
only in the headless tree. The headless binary compiles (but never calls) the
winit-bound `engine::application`; its crate-level `allow(dead_code)` covers
that.

---

## Build & run

```sh
cargo run --release          # the windowed app (default-run = globe-experiment)
# The separate `headless` binary renders one frame to a PNG (flat flags, no
# subcommand). It takes ONE --scene JSON (simulation + camera + optional ui);
# the output target (--output, --width, --height) stays as CLI flags. Unknown
# JSON keys are rejected (deny_unknown_fields).
cargo run --release --bin headless -- --output frame.png --scene \
    '{"simulation":{"datetime":"2024-01-15T12:30:00Z"},
      "camera":{"longitude":-75,"latitude":40,"distance":12742,"tilt":0}}'
# Add a "ui" section (Vec<ui::UiPanel>) to overlay mock UI panels for
# headless UI-layout debugging. A panel is a corner anchor + "rows" (outer
# array = top-to-bottom rows, inner = left-to-right instruments); taffy
# computes all positions and the panel size, so the JSON carries no pixels:
cargo run --release --bin headless -- --output mock.png --scene \
    '{"simulation":{"datetime":"2024-01-15T12:30:00Z"},
      "camera":{"longitude":-75,"latitude":40,"distance":12742,"tilt":0},
      "ui":[{"anchor":"top_left","rows":[
        [{"header":{"title":"Time / Subsolar"}}],
        [{"readout":{"label":"UTC","value":"12:30:00"}}],
        [{"dual_readout":{"left_label":"Lat","left_value":"-21",
          "left_unit":"deg","right_label":"Lon","right_value":"-5",
          "right_unit":"deg"}}],
        [{"toggle":{"label":"Run","active":true}},
         {"lamp":{"label":"Signal","status":"ok"}}],
        [{"slider":{"value":0.5,"range":[0,4.6]}}]]}]}'
```

First build: slow (~1.5 min extra), needs network. `build.rs` downloads 13
textures (JPEG/TIFF verbatim: Terra x4, stars, Luna, + 7 planets — five 8K,
two 2K), the JPL ephemeris (~98 MB), `EOP-All.csv`, the three IERS-2010
tables, and the EGM96 gravity coefficients (~5.4 MB, for the numerical orbit
propagator) into `OUT_DIR`; bakes 3 atmosphere LUTs as f16 KTX2. Subsequent builds reuse cached files. Delete a file in
`OUT_DIR` to re-download it. **VRAM** is now ~1.5 GB (the seven native-res
planet textures add ~686 MB; see `constraints.md`).

**WGSL is compiled by naga at runtime, not during `cargo build`.** Validate
after every shader edit:

```sh
naga --compact --capabilities none shaders/scene.wgsl
```
