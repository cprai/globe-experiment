# CLAUDE.md

Project rules for **Solar System**, an astronomically-accurate solar-system
renderer with satellite tracking (past scenes only). Topic rules live in
`.claude/rules/` — some load at launch, path-scoped ones when you open
matching files. Read loaded rules before making changes.

**When rules/docs and the source disagree, the source wins.**

**After completing all changes, print a one-line commit message describing
the currently uncommitted changes — as an example only. Do NOT commit.**

## Code search: prefer symbol search over grep/whole-file reads

The `codebase-search` skill has the routing table. In order:

1. **serena** (LSP-backed, live — no index) for Rust, WGSL
   (`src/engine/shaders/scene.wgsl`), and Python (`scenes/*.py`) symbol lookup,
   references, and structure.
2. **codebase-memory-mcp** for what serena cannot do: call-graph tracing,
   Cypher/aggregation queries, complexity hotspots, dependency-crate source.
   It is a snapshot — re-index before use.

Plain Grep/Read for non-code files; always `Read` a file before editing.

## What this is

Rust (edition 2024) on winit + wgpu + egui/egui_taffy, satkit
(SGP4/ephemeris/EOP), pyo3 (embedded CPython, unconditional — every build
needs Python 3 dev headers). Crate `globe-experiment`; `iced` is gone, do not
reintroduce it.

Terra, Luna, and the seven planets at true position/scale from JPL DE440 +
real EOP, each drawn as a single shader impostor (no meshes anywhere) with
data-driven per-body shading; Hillaire-2020 atmosphere; mutual eclipse
shadows. Satellite tracking via SGP4 or numerical propagation, with predicted
orbit paths and a manually-thrustable satellite. Inertial (star-fixed) camera
orbiting a selectable target; simulation clock (1x–100x). **Past scenes
only** (before build date) — what makes full EOP accuracy attainable.
Saturn's rings are not yet rendered (deferred).

Two binaries over one shared `src/engine/` (no lib crate): `globe-experiment`
(windowed app + `src/scenes/`) and `headless` (single-frame PNG). The `*_py`
scenes load their UI panels from a runtime Python script (the required
`--script` argument; reference scripts in repo-root `scenes/`) — edit +
relaunch, no rebuild. See `architecture.md`.

## Build & run

```sh
cargo run --release          # the windowed app (default-run = globe-experiment)
# headless: one frame to PNG. ONE --scene JSON (simulation + camera +
# optional ui); output target stays as CLI flags. Unknown JSON keys rejected.
cargo run --release --bin headless -- --output frame.png --scene \
    '{"simulation":{"datetime":"2024-01-15T12:30:00Z"},
      "camera":{"longitude":-75,"latitude":40,"distance":12742,"tilt":0}}'
# Optional "ui" section (Vec<ui::UiPanel>) overlays mock panels for layout
# debugging: corner anchor + "rows" (outer = top-to-bottom, inner =
# left-to-right instruments); taffy computes all sizes, no pixels in the JSON.
# Instruments: header/readout/dual_readout/button/toggle/lamp/slider, e.g.
#   "ui":[{"anchor":"top_left","rows":[
#     [{"header":{"title":"Time"}}],
#     [{"readout":{"label":"UTC","value":"12:30:00"}}],
#     [{"toggle":{"label":"Run","active":true}},
#      {"lamp":{"label":"Signal","status":"ok"}}]]}]
```

First build is slow (~1.5 min extra, needs network): `build.rs` downloads the
textures, ephemeris, EOP, IERS tables, and gravity coefficients into
`OUT_DIR` and bakes the atmosphere LUTs. Later builds reuse them; delete a
file in `OUT_DIR` to refresh it. VRAM is ~1.5 GB (see `constraints.md`).

**WGSL is compiled by naga at runtime, not during `cargo build`.** Validate
after every shader edit:

```sh
naga --compact --capabilities none src/engine/shaders/scene.wgsl
```

**Python scene scripts** (`scenes/*.py`): `ruff format` + `ty check` after
edits (`format-python` / `check-python-ty` skills). `ty` reporting the
embedded `globe` module as unresolved is expected (no stubs yet) — ignore
that, fix anything else.
