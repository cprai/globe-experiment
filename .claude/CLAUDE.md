# CLAUDE.md

Project rules and conventions for **Solar System**, an astronomically-accurate
solar-system renderer with satellite tracking (past scenes only). Rules are in
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
**manually-controlled satellite** flies on: the `manual_control` scene
seeds one object from the ISS TLE, re-anchors its GCRF state vector to the
clock each frame, and its bottom-center **Burns panel** of six hold-to-fire
keys (prograde/retrograde, normal/anti-normal, radial out/in) integrates a
game-strength thrust into the velocity while held, with apo/peri/speed
readouts; a scene may mix both backends, and `iss_and_hubble`
does), rendering the star-fixed
inertial ellipse as a thick depth-tested line whose tail fades out sharply
near one full orbit), inertial
(star-fixed) camera that orbits a selectable **target** (Terra, Luna, or
any of the **seven planets** in the `solar_system` scene via a body-selector
panel (one key per body); Luna in the eclipse scenes via a Terra/Luna
panel; the `headless`
binary picks the body with `camera.target` "terra"/"luna"/"mars"/... ),
simulation clock (1x-100x, plays from launch). Luna is
a triaxial ellipsoid at true scale/distance, oriented by the full IAU lunar
rotation (correct near side + libration), lit by Sol, with **mutual
Terra/Luna eclipse shadows** (solar-eclipse spot on Terra, lunar-eclipse "blood-red
Luna"). The **seven planets** (`src/engine/planet.rs`) are triaxial ellipsoids
(equal equatorial axes — their familiar oblate forms) at true
position/scale (DE440, heliocentric-framed), oriented by the IAU planet rotation and
sun-lit with simple Lambert. **EVERY body — Terra, the seven planets, and
Luna — is drawn as a single shader impostor** (no mesh anywhere in the
engine): the CPU projects the body center to screen space in `prepare` and the
GPU draws one camera-facing quad whose fragment shader ray-traces the triaxial
ellipsoid (Terra = the WGS84 spheroid), writing per-fragment depth so bodies
occlude each other. Shading is **data-driven per body** (`planet::Maps` +
feature flags in the per-body uniform): a bare-albedo body gets plain
hard-terminator Lambert; Terra's row carries night/normal/specular maps +
`has_atmosphere`, lighting up the full look (normal-map relief, GGX ocean
glint, transmittance-tinted sunlight, emissive city lights) in the same
`fs_planet`. The trace is **distance-adaptive**:
perspective (eye-ray, reconstructed via `inv_view_proj`) for a near/orbited
body, orthographic (parallel-ray, f32-safe) for a distant one — classified per
frame by apparent angular size. **Same-system eclipse shadows are generic**:
each impostor's uniform carries an occluder list filled from
`CelestialBody::same_system` (Terra shadows Luna — the blood-red lunar
eclipse — and Luna shadows Terra — the solar-eclipse spot; a future moon
system self-shadows with no renderer change), with a
per-body Sol angular radius for the penumbra. Because bodies sit
millions-to-billions of km
out (past f32 precision), **all rendering is done in a camera-target-local
"render frame"**: positions are expressed relative to the camera target's
center, so
the orbited body sits at a bit-exact zero and far planets do not jitter. (The
`CelestialSphere` itself is **heliocentric** — Sol at the origin, Terra at
`-sol_geo` — and stores body placements as **f64** (`DVec3` centers, `DMat3`
rotations); **all CPU-side computation is f64 end to end** — celestial sphere,
the camera (`PtzCamera` rig + input, `DVec3`/`DQuat`), satellite pipeline, and
the renderer's `prepare`/view-projection math (`DMat4`) — with f32 appearing only
at the GPU uniform/instance upload and in egui-facing readouts. f64 is
required: an f32 heliocentric-minus-origin cancels catastrophically.) The
renderer derives every body's position/orientation from the frame's **time**
(`CelestialSphere::at`); `RenderState` carries only the time, the camera rig, the
camera target, and satellite markers. There is no Earth-fixed origin or
`sol_dir` in the render path — every body is lit from Sol *position*. All nine
bodies draw from every vantage; the **atmosphere** (a screen quad, drawn when
a `has_atmosphere` body sits at the render origin — Terra/Luna targets today)
and the **satellite overlays** (orbit paths + markers, Terra-frame positions)
are the only gated passes. For Terra/Luna (render origin at Terra)
geometry stays Terra-local. A **reversed-Z depth buffer** (Depth32Float) makes
Terra occlude Luna. **Past scenes only** (before build date) — what makes full EOP accuracy
attainable. The crate is named `globe-experiment`; `iced` is gone, do not
reintroduce it. **Saturn's rings are not yet rendered** (deferred).

**Python scene scripting (pyo3 0.29, embedded CPython — unconditional
dependency, every build needs Python 3 dev headers):** the `manual_control_py`
and `solar_system_py` scenes are clones of their Rust siblings whose
`UIDrawable::get_drawables` delegates to a script whose path is the scene's
**required `--script` argument** (`scene manual_control_py --script
scenes/manual_control_py.py`; each scene is its own clap subcommand with its
own `Args` struct, so the non-Python scenes reject `--script` natively; the
repo ships the reference scripts in the repo-root `scenes/` directory),
**read at runtime** — edit + relaunch, no rebuild (the one deliberate
exception to "everything embedded"). The script imports the embedded `globe` module
(instruments, `Panel`/`PanelAnchor`, `Interactive*` twins holding Python
callables, `Clock` (registered only for its `MIN`/`MAX_MULTIPLIER`
classattrs), `BodySelector`, readout types — the dual Rust/Python UI
API) and receives the live scene, itself a `#[pyclass]` (see `scenes.md`)
whose `paused`/`multiplier`/`datetime_label()` properties — the Python face
of the `SceneClock` trait API (`engine::scene::clock`, which holds all the
clock logic as trait default methods; `Clock` itself is plain data) — are
how a script reads and drives the clock (no `Clock` instance crosses into
Python).
Both scene pairs live side by side so the two APIs can be compared.

The crate builds **two binaries over one shared `src/engine/`** (no lib
crate): `globe-experiment` (`src/main.rs`, the windowed app + scenes) and
`headless` (`src/headless.rs`, the single-frame PNG renderer). Both bin roots
declare `mod engine;` (everything used to run the app: `application`, `camera`,
`planet`, `py`, `renderer`, `scene`, `ui`); the trees differ
only at the top level — `scenes` exists only in the main tree, `offscreen`
only in the headless tree. The headless binary compiles (but never calls) the
winit-bound `engine::application` and links libpython without ever
initializing the interpreter; its crate-level `allow(dead_code)` covers
both.

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
`OUT_DIR` to re-download it. **VRAM** is ~1.5 GB (the twelve native-res
impostor-body maps — 9 albedos + Terra's night/normal/specular, all group 1 —
total ~1.35 GB; group 0 keeps only the stars map + the 3 LUTs; see
`constraints.md`).

**WGSL is compiled by naga at runtime, not during `cargo build`.** Validate
after every shader edit:

```sh
naga --compact --capabilities none shaders/scene.wgsl
```
