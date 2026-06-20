# CLAUDE.md

Project rules, conventions, and constraints for **Globe**, an
astronomically-accurate satellite simulation tool (past scenarios only).
Companion file: `MEMORY.md` holds the technical reference (how each
subsystem works, the rendering/atmosphere math, the orbital & ephemeris
math — see its §16 for the full satellite/Sun/planet computation and how the
satkit crate is used — exact constants, file map, phase history). Read both. When `MEMORY.md` and the source disagree,
**the source wins** — the look-tuning constants in particular drift
between sessions.

This file supersedes the old `claude/phase{1,2,3}/*.md` docs, which have
been deleted and folded into here and `MEMORY.md`.

---

## What this is (one paragraph)

An **astronomically-accurate satellite simulation tool**, built on a
Google-Earth-style physically-lit 3D globe renderer. Rust (edition 2024),
**winit 0.30** window/event loop, **wgpu 29** (direct dependency) for
rendering, **egui 0.34** for the control overlay. It renders a
physically lit Earth (day/night, procedural city lights, normal-mapped
relief, GGX ocean glint), a Hillaire-2020 precomputed-LUT atmosphere, and
a star/sun backdrop, with an orbital pan/tilt/zoom camera that lives in an
**inertial (star-fixed) frame** — it holds still relative to the stars while
the Earth rotates beneath it. The geometry is
**physical**: the globe is the WGS84 reference ellipsoid and **world space is
in kilometers** (real-scale orbital simulation). The Sun
direction, Earth's orientation, and the star-map orientation are computed from
the **satkit** crate's **JPL DE440 ephemeris** + Earth-orientation transforms
for the current time (no more sun sliders). It tracks satellites: each
TLE is propagated with satkit's SGP4 (the tracked array is built by a scenario
and passed to the simulation). All of this is driven by a
**simulation clock** (play/pause, exponential 1x-100x real-time speed); each
satellite is drawn as a marker circle and everything animates as time
advances, with the clock's datetime shown in the UI.

**Scope & accuracy intent (load-bearing):** the goal is astronomical
accuracy, and the tool **only ever simulates scenarios that happened in the
past, before the build date** — never live or future times. That past-only
constraint is what makes full Earth-orientation accuracy achievable: real
Earth-orientation-parameter (EOP) data is a fixed, never-changing record for
any past date (see the EOP rules below and `MEMORY.md` §16.8). Treat any
accuracy regression (e.g. silently widening error, drifting frames) as a bug,
not a cosmetic issue. The crate is
named `globe-experiment`; **iced is no longer a dependency** (removed in
phase 2) — do not reintroduce it.

## Build & run

```sh
cargo run --release
```

- **First build needs a network connection** and is slow (~1.5 min extra):
  `build.rs` downloads five textures and BC7-encodes them **in memory** (the
  source image never hits disk), writing only the `*.ktx2` into `OUT_DIR`, and
  downloads the ~98 MB JPL ephemeris + CelesTrak's `EOP-All.csv` (~2-3 MB
  Earth-orientation params) into `OUT_DIR` verbatim (those must be on disk for
  `include_bytes!`). Subsequent builds reuse the cached `OUT_DIR` outputs; the
  `.ktx2` is the texture cache (`rerun-if-changed` points at it), so a present
  output is never re-downloaded. No `assets/` directory is created. (Delete a
  `OUT_DIR/*.ktx2` to re-download+re-encode that texture, or `OUT_DIR/EOP-All.csv`
  to pull a fresher EOP snapshot.)
- Smoke test: `timeout <n> cargo run 2>&1 | head` (or redirect to a file —
  pipe buffering can swallow output). wgpu validation errors panic in the
  first frames, so a clean 15-25 s run means pipelines/bindings are valid.
- **WGSL is compiled by naga at runtime** in `create_shader_module`, *not*
  during `cargo build`. A clean `cargo build` proves nothing about the
  shader — you must actually run it.

---

## Golden rules (do not violate without asking the owner)

### Source format
- **All `.rs` files and `shaders/globe.wgsl` are pure ASCII**, comments and
  strings included (`—`→`-`, `°`→`deg`, `±`→`+/-`, `≈`→`~`, `×`→`x`/`*`).
  Keep new source ASCII-only. (Markdown docs like this one and `MEMORY.md`
  may use Unicode for math readability.)

### Constants that are duplicated and MUST stay in sync
- The **atmosphere medium + geometry constants exist twice**: in
  `build.rs`'s inline `mod atmosphere` (the LUT bake) and in
  `shaders/globe.wgsl` (the geometric twins + `MIE_G`). Change one, change
  the other, or the baked LUTs and the shader that samples them diverge.
- The **inscatter LUT parameterization is implemented independently twice**
  and must match exactly: the **split row mapping** (`v`: lower half =
  ground-hitting rays, upper half = limb rays), the **reference-point
  choice** (ground hit vs. closest approach), and the **Bruneton
  transmittance mapping** all appear in both `build.rs::bake_inscatter` /
  `bake_transmittance` / `sample_transmittance` and in
  `globe.wgsl::fs_atmosphere` / `sun_transmittance`. A change on one side
  silently corrupts the atmosphere.

### Surface / display
- **Surface format must be non-sRGB** (`Gfx::init` picks `find(|f|
  !f.is_srgb())`) and **present mode `AutoVsync`**. `get_default_config`'s
  defaults are *not* correct here (it picks an sRGB format and Mailbox on
  DX12) — these two overrides are deliberate. See `MEMORY.md` for why.
- **Every look-tuning constant in `globe.wgsl` is calibrated to the
  non-sRGB surface.** Moving to a colorimetrically correct sRGB target
  invalidates all of them and requires re-tuning the entire shader.
- **No HDR, no bloom.** Output is LDR straight to the swapchain. "Glow" and
  "brightness" everywhere (sun disc, city lights) are the LDR cheat: a
  clipped-white core inside an additive soft falloff. A real bloom pass is
  an explicitly **declined** follow-up — do not assume it exists.

### Rendering invariants
- **No depth buffer anywhere.** Draw order does all occlusion: stars →
  surface → atmosphere → satellite markers, in that order, into one render
  pass. This only works because the scene is convex spheres. Do not add
  geometry that breaks that assumption without also adding a depth
  attachment. The **markers are screen-space overlays** drawn last as a single
  **instanced** draw (one instance per tracked satellite): each is a
  constant-pixel circle whose quad is generated from the vertex index and whose
  world position + visibility come from a per-instance marker buffer,
  alpha-blended. They have no depth, so occlusion behind the globe is decided
  **on the CPU** per marker (`marker_occluded` in `src/simulation/mod.rs`, a ray
  vs. mean-radius sphere; one `SatelliteMarker { position_km, visible }` per
  object in `RenderState.markers`) and passed to the shader as a per-instance
  visible flag.
- **Idle = zero GPU work.** `application::run` sets `ControlFlow::Wait`; frames are
  driven *only* by targeted `window.request_redraw()` (input changed the
  camera, inertia/zoom glide coasting, **the simulation clock running**, egui
  repaint, resize, surface recovery). **Never add an unconditional vsync
  render loop.** Note: the clock is another "animating" source and **starts
  playing** (owner's choice), so the app renders continuously from launch and
  is only idle once the clock is **paused** — this is still condition-gated
  `request_redraw`, not an unconditional loop.
- **The terminator / night-side darkening must use the GEOMETRIC normal**
  (`cos_sun = dot(n_geo, sun)`, the `daylight` smoothstep), never the
  bump-mapped normal `n`. Bump detail on the day/night edge speckles it.
- **Star map and Sun are ephemeris-driven** (`src/simulation/celestial_sphere.rs`). The Sun's
  direction comes from the JPL DE440 ephemeris and the star map is oriented by
  Earth's real GCRF↔ITRF attitude, both for the current clock time. (This
  reverses the earlier deliberately-non-physical "celestial sphere rigidly attached to the
  sun" model — the owner asked for the astronomically-correct version on
  2026-06-18. The old slider-driven `Sun` struct is gone.) `star_rot_inv`
  uploaded to the shader is now `P · R_itrf→gcrf · Pᵀ` (P = the standard-ECEF
  → world-frame permutation); do not replace it with a sun-attached rotation.
- **`init_satkit()` must seed satkit's EOP table at startup, not just the
  ephemeris.** Every satkit frame transform reads its global
  Earth-orientation-parameter (EOP) table on first use — even the `*_approx`
  ones, because `gmst` does a UT1 conversion that consults it, and `qteme2itrf`
  reads polar motion. satkit's default EOP loader *resolves a data directory
  and creates an empty `satkit-data` dir next to the binary* as a side effect.
  So `init_satkit` pre-seeds the EOP singleton via
  `earth_orientation_params::init_from_bytes(...)` (plus
  `disable_eop_time_warning()`); this consumes satkit's one-shot lazy load so
  the dir is never created. **Do not drop the EOP seed** — the stray
  `satkit-data` dir comes back. (Run from a clean dir to verify none appears.)
  - **Content: real EOP, bundled** — CelesTrak's `EOP-All.csv`, embedded the
    same way as the ephemeris (`build.rs` downloads it straight into `OUT_DIR`;
    `celestial_sphere.rs` `include_bytes!`-es it as `EOP` and feeds it to
    `init_from_bytes`). So polar motion + UT1-UTC are real, not zeros.
  - **What consumes it.** The **satellite** path (`qteme2itrf`) is the full,
    non-`approx` transform: it applies real polar motion (via `qitrf2tirs`) and
    real UT1-UTC (via `gmst`), so the simulated ground track is sub-arcsec. The
    **celestial-sphere** path still uses the `*_approx` transforms; those pick up real
    UT1-UTC through `gmst` but still neglect polar motion (~0.3") and use
    approximate nutation (~1"). That residual is sub-arcsec and only affects the
    Sun/star *backdrop*, so it's left as-is.
  - **Going fully IERS-2010 for the celestial sphere is a bigger job — don't assume it's a
    one-liner.** Switching the celestial sphere to `qgcrf2itrf`/`qitrf2gcrf` additionally
    needs satkit's IERS nutation tables (Tab5A/5B/5D), which it reads from the
    data dir via `ierstable` and **`.unwrap()`s** — i.e. it would `panic` (and
    re-create `satkit-data`) unless those tables are *also* bundled and seeded
    (`ierstable::init_from_bytes`). Not done; not needed for the satellite.
  - **Validity:** EOP is bounded (≈1962 → build date); see the scenario-bounds
    note under Conventions and `MEMORY.md` §16.8/§16.9. Past-only scenarios make
    the bundled snapshot permanently valid (history doesn't change).
- **The camera is in the inertial (star-fixed) frame** (owner-requested
  2026-06-18). The `Camera` rig (in `src/application/camera.rs`) is built around
  the origin as before but interpreted in the celestial frame, then rotated into
  the Earth-fixed world by **`celestial_to_world = celestial_sphere.star_rot_inv.transpose()`**.
  That rotation is produced by `SimulationState::celestial_to_world()` and applied
  in `ApplicationState::redraw` (the camera lives in `application`, so it resolves
  `camera.eye`/`camera.view_proj` there and passes the finished `eye`/`view_proj`
  into `SimulationState::frame_state`). Net effect: the camera does not rotate
  with the Earth — the globe spins under a star-locked view, so
  `Camera.longitude/latitude` are an **inertial** look direction, not geography.
  Don't move the camera back into the ECEF/world frame.
- **Backdrop anchoring**: both the star lookup and the sun disc are
  functions of the **camera-relative view direction** (`world − camera_pos`),
  not of position on the celestial sphere. Anchoring either to the celestial-sphere
  surface reintroduces parallax between sun and stars (a fixed bug). Do not
  regress it.

### Startup / window
- **The first frame must render via a direct `self.redraw()` call** from
  `ApplicationState::resumed()` (in `src/application/mod.rs`), never
  `request_redraw()`. The window starts hidden (`with_visible(false)`) and is
  revealed after the first `present()`; Windows does not deliver
  `RedrawRequested` to a hidden window, so a requested redraw would never fire
  and the window would stay invisible forever. The reveal is now driven by the
  `FrameOutcome` that `Gfx::update` returns (`Presented`/`Occluded` reveal the
  window; the `Occluded` first-frame guard also re-requests a redraw) — the
  renderer never touches the window. **Do not "simplify" either.**

### Input feel
- **Do not restructure the smoothed-zoom glide/coast in `input.rs`.** It
  was iterated ~5 times with the owner (including a full remove-then-revert).
  Tune only the named constants (`ZOOM_HALF_LIFE_MIN/MAX`,
  `ZOOM_COAST_HALF_LIFE`, `ZOOM_STOP_RATE`). Rejected designs that must not
  be reintroduced: fixed-half-life always-glide, and a fixed burst-gap
  split (instant if gap < threshold, glide otherwise). See `MEMORY.md`.

### Tuning discipline
- **Look-tuning constants drift between sessions.** Always read
  `globe.wgsl` for current values; never trust a doc's numbers. The
  `MEMORY.md` snapshot is dated and will go stale.
- **Tune and feel-test on a native Windows release build.** The WSLg dev
  environment here cannot validate exact colors or interaction feel.

### Documentation
- **Keep all documentation current with the code, in the same change** —
  code comments, `CLAUDE.md`, `MEMORY.md`, and `README.md`. When you change
  behavior, structure, an interface, or a documented constant, update the
  matching comment and doc section as part of that change, not "later."
  Stale docs are treated as bugs (this repo has had several — e.g. comments
  that pointed at a deleted `atmosphere.rs`, or a "sequential loading" note
  that survived the switch to rayon).
- **One allowed exception**: the dated live-constant snapshot in `MEMORY.md`
  §13 (and specific look-tuning numbers) may lag owner tuning — for live
  values the **source is authoritative** (see Tuning discipline above).
  Everything else — architecture, behavior, the rules/conventions here, the
  file map — must stay accurate.

---

## Deliberately reverted / rejected — do not re-add without asking

These were implemented and then removed (or considered and declined) by
the owner. Re-introducing them silently is a regression.

- **Animated/time-varying ocean wave noise** — made static on purpose
  (temporal stability + idle-is-free). The wave noise is fixed.
- **Day-side albedo saturation boost** and an **`OCEAN_TINT`
  water-darkening tint** — both reverted; albedo is used as sampled.
- **The astronomically-correct star model** — was rejected during the
  slider era, but **adopted on 2026-06-18** via the JPL ephemerides (see the
  "Star map and Sun are ephemeris-driven" rule above). The old non-physical
  sun-attached celestial sphere is the thing not to reintroduce now.
- **Noise *frequency* ramp toward the terminator** for the city-light
  dissolve — rejected because it boils/fizzes under sun motion. The dither
  uses a **fixed** `DITHER_SCALE` for a coherent wipe.
- **Terminator-varying emissive threshold** — rejected (night-map city
  cores are clipped plateaus, so a rising threshold pops whole blobs
  instead of shrinking them). Use the uniform threshold + dither.
- **Per-brightness glow scaling** — rejected; surviving city pixels glow at
  uniform strength.
- A **`thread::scope` parallel texture *decode* + LUT bake** at startup —
  reverted in phase 1. NOTE: this is *not* the same as the current
  rayon parallelization of `GlobeRenderer::new` (the private scene builder
  inside `Gfx::init`: module compile + KTX2 uploads + pipeline compiles), which
  is **intentional** and was added on
  explicit request after decode/bake moved to build time. Do not confuse
  the two; do not "re-revert" the rayon code.

---

## TODO / backlog (not scheduled — optional, pick up later)

Deliberate "someday, maybe" items. None of these is committed work or a bug;
they're parked here so we can refer back when looking for extra things to do.
Adding to this list does **not** authorize doing it — confirm with the owner
first. (Concrete engineering follow-ups also live in `MEMORY.md` §14; this
section is for the larger, explicitly-deferred ideas.)

- **Full IERS-2010 Earth orientation for the celestial sphere.** Switch the Sun/star
  backdrop in `src/simulation/celestial_sphere.rs` from the `*_approx` transforms
  (`qgcrf2itrf_approx` / `qitrf2gcrf_approx`) to the full
  `qgcrf2itrf` / `qitrf2gcrf`, closing the residual ~1" error (real polar
  motion + IERS-2010 nutation instead of approximate nutation / neglected
  polar motion). The satellite path is already full-accuracy; only the
  backdrop is approximate, and at ~1" it's already sub-pixel, so this is a
  consistency/correctness nicety, not a visible fix.
  - **Feasibility (verified against satkit 0.18.1):** the nutation tables can
    be seeded as singletons from baked-in bytes, exactly like the ephemeris
    and EOP. satkit exposes `frametransform::init_iers_table_from_bytes(id,
    bytes)` (re-export of `ierstable::init_from_bytes`) over three
    `OnceLock<IERSTable>` singletons keyed by `IersTableId::{Tab5A, Tab5B,
    Tab5D}`. Seed all three in `celestial_sphere::init_satkit()` *before* the first celestial-sphere
    transform — otherwise `table()`'s lazy `get_or_init` wins and
    `IERSTable::from_file(...).unwrap()` resolves a data dir (recreating the
    stray `satkit-data` dir), tries to download from
    `https://storage.googleapis.com/astrokit-astro-data/`, and panics if the
    file is absent (same failure mode the EOP seed already prevents).
  - **Real work beyond seeding:** bundle three small text files
    (`tab5.2a.txt`, `tab5.2b.txt`, `tab5.2d.txt` — KB each, negligible binary
    cost) via `build.rs` (`EMBEDS` table → `OUT_DIR`) and
    `include_bytes!` them in `celestial_sphere.rs`; flip the two transform calls to the
    non-`approx` versions; and update every "celestial sphere is `*_approx`" claim in
    `CLAUDE.md`, `MEMORY.md`, and the `celestial_sphere.rs` / `init_satkit` doc-comments in
    the same change.

---

## Conventions

### Coordinate & mapping (used consistently everywhere — see `MEMORY.md` for formulas)
- Globe is the **WGS84 reference ellipsoid at the origin**; **world space is
  kilometers** (camera distance, mesh vertices, `camera_pos` uniform — all
  km). **+Y is north.** Longitude 0°, latitude 0° faces **+Z**; +X is east at
  the prime meridian. The physical constants and the `surface_position` /
  `geodetic_normal` helpers live in `src/earth.rs` (a top-level shared module,
  the single source of truth; mesh **and** camera build their geometry from it).
- **Load-bearing identity (post-WGS84)**: on an ellipsoid the surface normal
  is **no longer** `normalize(position)`, so the mesh now carries an explicit
  per-vertex **geodetic normal**. That normal happens to equal the old
  unit-sphere direction (same lat/lon structure), which is what keeps the
  shader's analytic east/north tangent frame and the surface-anchored
  city-light noise (`n_geo * DITHER_SCALE`) valid unchanged. The atmosphere
  and star passes **reuse that unit normal** (×`ATMOSPHERE_TOP_KM` /
  ×`STARS_RADIUS_KM`) to build their spheres — they must stay spherical, so
  they must **not** use the ellipsoid `position`.
- Equirectangular UVs: `u = (lon+180)/360` (wraps; sampler repeats on U),
  `v = 0` at north pole → `1` at south (sampler clamps on V). The mesh
  duplicates the seam column so U wraps cleanly.
- **All passes now work in kilometers.** The atmosphere fragment shader no
  longer multiplies by `PLANET_RADIUS_KM` (world is already km). The
  scattering model itself stays **spherical** (`PLANET_RADIUS_KM` 6360 /
  `ATMOSPHERE_TOP_KM` 6460, baked into the LUTs); the visible surface is the
  WGS84 ellipsoid (6357–6378 km), so it can poke a few km past the 6360 km
  atmosphere "ground" near the equator — a small, intentional approximation
  (the atmosphere was always spherical). Do not try to make the LUT model
  ellipsoidal; it relies on spherical symmetry.

### Code style
- Match the surrounding code: dense, explanatory comments that capture
  *why* (especially the non-obvious GPU/winit/precision reasons), small
  focused structs, descriptive names. The existing files are the style
  guide.
- The code is organized into five top-level modules plus a shared `earth`:
  **`application`** (windowing, the winit event loop, the camera + input, and
  per-frame redraw orchestration; owns the `SimulationState` and the renderer),
  **`simulation`** (`SimulationState` + the astronomical math; produces a
  `RenderState`; no winit/wgpu/egui and no `Camera` type), **`renderer`** (the
  `Gfx` struct: GPU setup + per-frame `update`), **`ui`** (the egui panel
  logic), **`earth`** (`src/earth.rs`: shared WGS84 constants/helpers), and
  **`scenarios`** (`src/scenarios/`: one module per past scenario, each with a
  `run()` that seeds satkit, assembles the tracked objects, builds the
  `SimulationState`/`ApplicationState`, and hands off to `application::run`).
  `main.rs` is tiny: it uses **clap** to parse the CLI (`scenario <name>`
  subcommand) and dispatches to the matching `scenarios::*::run`; it does no
  setup itself. See `REFACTOR_PLAN.md` for the module-boundary rationale.
- **Run `cargo fmt` after every code change** (`.rs` edits). rustfmt with
  the default config is the sole formatting authority — don't hand-format,
  and keep diffs limited to real changes. (`cargo fmt` does not touch
  `shaders/globe.wgsl`; format WGSL by hand to match the surrounding code.)

### Where things live
- **Shader look knobs**: top of `shaders/globe.wgsl` (`const` block).
- **Atmosphere medium constants**: `build.rs` `mod atmosphere` (bake side)
  *and* `shaders/globe.wgsl` (geometric twins).
- **Input feel constants**: top of `src/application/input.rs`.
- **Earth physical constants** (WGS84 axes, eccentricity, mean radius, GM,
  rotation rate) + the `surface_position`/`geodetic_normal` helpers:
  `src/earth.rs` (top-level shared module).
- **Camera limits**: `Camera` associated consts in `src/application/camera.rs`
  (in km, expressed as `<radii> * earth::MEAN_RADIUS_KM` to preserve the feel).
- **Render orchestration / windowing**: `src/application/mod.rs`
  (`ApplicationState` + the winit `ApplicationHandler` + `run`). It resolves the
  camera, calls `SimulationState::frame_state` (which returns the frame's
  `RenderState` + `TelemetryState` together), runs the egui panel from that
  telemetry, then calls `Gfx::update`.
- **Renderer**: `src/renderer/mod.rs` (`Gfx`: `init`/`resize`/`viewport`/
  `update`; `FrameOutcome`; `UiFrame`; the private `GlobeRenderer` scene + the
  `Uniforms` packing; `MARKER_RADIUS_PX`). Mesh in `src/renderer/mesh.rs`.
- **Simulation state / RenderState / TelemetryState**: `src/simulation/mod.rs`
  (`SimulationState` composes clock / `Vec<Satellite>` / celestial_sphere; `advance`,
  `celestial_to_world`, `frame_state` -> `(RenderState, TelemetryState)` where
  both carry a per-satellite `Vec` (`markers` / `satellites`); `marker_occluded`).
  `SimulationState::new(satellites)` takes the tracked list and starts the clock
  at the **first** satellite's epoch (panics if the list is empty).
- **Satellite tracking**: `src/simulation/satellite.rs` (TLE parse + SGP4 +
  frame conversion to a world-space km point). Each `Satellite` is one tracked
  object storing only the `tle` + `name`; the position is **not** stored -
  `state_at(time)` propagates it on demand (returning a `SatelliteState`)
  wherever it's needed, so nothing goes stale as the clock advances. This module
  is element-set agnostic - it carries no TLE data; `Satellite::from_tle(text)`
  parses whatever a scenario hands it. **The TLEs live in the scenario** that
  uses them: the `ISS_TLE`/`HST_TLE` **inline source literals** (`concat!` of
  the three TLE lines each; not `include_str!`, so a fresh checkout needs no
  data file) are `const`s in `src/scenarios/iss_and_hubble.rs`, which assembles
  the tracked array (`vec![Satellite::from_tle(ISS_TLE), ...]`) and passes it
  into `SimulationState::new`, not built inside the simulation. The HST set is
  flagged in-source as approximate (real orbit shape, made-up phase). Marker
  colors live in the marker shader.
- **Simulation clock**: `src/simulation/clock.rs` (`Clock`: advances by
  wall-clock dt x multiplier, play/pause, speed bounds `MIN/MAX_MULTIPLIER`).
  Starts at the TLE epoch.
- **Sun / Earth orientation / star map**: `src/simulation/celestial_sphere.rs`
  (`CelestialSphere::at(time)` → `sun_dir`, `star_rot_inv`, subsolar lat/lon) via satkit's
  JPL ephemeris + `qgcrf2itrf_approx`/`qitrf2gcrf_approx`. `celestial_sphere::init_satkit()`
  (called once via the `simulation::init()` wrapper at the start of a scenario's
  `run`, before any `CelestialSphere` is built) seeds satkit's global state from
  embedded bytes: the JPL ephemeris (`jplephem::init_from_bytes`) **and** the
  real EOP table (`earth_orientation_params::init_from_bytes` of the bundled
  `EOP-All.csv`) — see the "`init_satkit()` must seed satkit's EOP table" golden
  rule above. The EOP file lives alongside the ephemeris in `OUT_DIR`
  (`OUT_DIR/EOP-All.csv`).
- All baked/transcoded assets land in `OUT_DIR` and are `include_bytes!`-ed,
  **including** the two satkit data files that are bundled verbatim: the JPL
  ephemeris (`linux_p1550p2650.440`, ~98 MB) and `EOP-All.csv` (~2-3 MB).
  `build.rs` downloads both straight into `OUT_DIR` (the `EMBEDS` table);
  `celestial_sphere.rs` embeds each with `include_bytes!`. No `assets/` dir, no
  runtime data file.

### Scenarios & valid time range (read before adding a scenario)
- **Scenarios live in `src/scenarios/`** — one module per scenario, each with a
  `run()` that pins the simulation to a specific **past** event (a satellite/TLE
  + a time window). `main.rs` parses the CLI with **clap** and dispatches to one
  (`globe-experiment scenario <name>`); the name is an **optional** positional,
  so a bare `scenario` lists the available scenarios (via `list_scenarios`,
  driven off the `ScenarioName` `ValueEnum` so it can't drift) instead of
  erroring. Today there are two: `scenarios::iss_and_hubble` (CLI token
  `iss_and_hubble`: ISS + HST) and `scenarios::iss` (CLI token `iss`: ISS only).
  Each **owns its own inline TLE element set `const`s** and assembles them into
  the tracked array, with a clock that starts at the first satellite's epoch; the
  `ISS_TLE` literal is **deliberately duplicated** across the two files (each
  scenario owns its TLE data — do not factor it into a shared const). Add a
  scenario by adding a module and a `ScenarioName` variant in `main.rs`.
- **Every scenario's time window must fall inside the valid EOP range** so the
  astronomical accuracy goal actually holds. That range is bounded on **both**
  ends:
  - **Lower bound: 1962-01-01.** Measured EOP simply does not exist before
    then (it's the start of the IERS series, not a satkit limitation). For
    earlier dates satkit returns no EOP and silently falls back to zeros — i.e.
    accuracy quietly degrades to the `*_approx` level, *and the satellite era
    starts in 1957*, so most realistic scenarios are fine, but verify.
  - **Upper bound: the build date** (more precisely, the last entry of the
    bundled `EOP-All.csv`). Past dates only — never live/future. Beyond the
    last entry satkit does **constant extrapolation** of the final value, which
    silently degrades. The "past-only, before build date" rule keeps you below
    this bound by construction.
- **So: when you add a scenario, check its start and end epochs against the
  bundled EOP file's `[first_entry .. last_entry]` MJD range** (and against
  1962 for the lower bound). The bundled `EOP-All.csv`'s last data row is
  the concrete upper bound; the first row (1962-01-01, MJD 37665) is the lower.
  If a scenario can't be brought in-range, it does not meet the accuracy bar —
  flag it rather than shipping a silently-degraded result.

---

## Hard constraints (environment & platform)

- **Device requires `Features::TEXTURE_COMPRESSION_BC`** (for the BC7
  textures). Universal on desktop GPUs; works under WSLg lavapipe too.
- **No mipmaps** (`mip_level_count 1` on every texture). Known shimmer at
  far zoom; the city-light dither can twinkle/alias when blobs shrink
  sub-pixel at low zoom (no MSAA either). Mitigations are documented but
  not implemented.
- **Large binary**: ~160 MB of BC7 + ~0.6 MB LUTs + the ~98 MB JPL ephemeris +
  ~2-3 MB EOP are embedded via `include_bytes!`, so the binary is large
  (~260 MB) and links slowly. Runtime file loading is a known, unimplemented
  follow-up.
- **JPL ephemeris and EOP are embedded, not runtime files.** `build.rs`
  downloads `linux_p1550p2650.440` (DE440, ~98 MB) from JPL and `EOP-All.csv`
  (Earth-orientation params) from CelesTrak on the first build straight into
  `OUT_DIR` (adds to the already-network-dependent first build);
  `celestial_sphere.rs` embeds each with `include_bytes!` and
  loads them via `jplephem::init_from_bytes` / `earth_orientation_params::
  init_from_bytes`. So there is **no runtime data file** - the binary is
  self-contained - but a fresh checkout still needs network for the first build.
- **Earth orientation: satellite is full EOP; celestial sphere is `*_approx` (~1").** With
  real EOP bundled, the satellite's `qteme2itrf` applies real polar motion +
  UT1-UTC (sub-arcsec). The Sun/star backdrop still uses the `*_approx`
  transforms (real UT1-UTC via `gmst`, but polar motion neglected + approximate
  nutation, ~1") — fine for a backdrop. Going full IERS-2010 for the celestial sphere would
  *additionally* require bundling satkit's IERS nutation tables (Tab5A/5B/5D,
  which `ierstable` `.unwrap()`s from the data dir) — not done. Because the tool
  is **past-only**, the bundled EOP is valid forever for in-range dates. Keep
  `init_satkit`'s EOP seed regardless: satkit would otherwise resolve an EOP
  data dir on first use and create a stray `satkit-data` dir.
- **`.cargo/config.toml`** adds `-lstdc++` on `x86_64-unknown-linux-gnu`
  **only** — `intel_tex_2`'s prebuilt ISPC objects need the GCC C++
  personality. MSVC on Windows is unaffected; do not make it
  cross-platform.
- **WSLg flakiness**: app launch intermittently fails with libEGL/MESA
  errors — transient, retry; not a code bug. Not present on native Windows.
- **Windows mount tooling**: `cargo add` can emit a bogus "found cargo.toml
  please rename" error on the case-insensitive mount — edit `Cargo.toml`
  directly and trust `cargo metadata`.

---

## Testing & verification

- There is **no test suite and no CI**. Verification is the smoke test
  above plus manual interaction on native Windows.
- **Check correctness with `cargo clippy`, not just `cargo build`** — run
  it heavily and aim for warning-free. clippy catches misuse, redundancy,
  and footguns the bare compiler misses. Caveat: neither command validates
  `shaders/globe.wgsl` — naga compiles WGSL only at runtime, so a clean
  build/clippy says nothing about the shader; you must run the app.
- Manual pass to run after risky changes: pan, flick (inertia), zoom to
  min/max, tilt to clamp, play/pause + speed slider (watch the Sun, stars,
  and satellite advance together), window resize, minimize/restore. Confirm
  idle (paused) renders **zero** frames.
- After atmosphere-constant or mapping changes, verify **both** the bake
  (`build.rs`) and shader (`globe.wgsl`) sides and re-run — bit-identical
  output is the goal when the change is meant to be neutral.
