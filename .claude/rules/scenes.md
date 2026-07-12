---
paths:
  - "src/scenes/**/*.rs"
  - "src/headless.rs"
---

# Scenes

## Adding a scene

- One module per past scene, defining `<Name>Scene` (implements the four
  engine traits; `CameraControl` normally via `ScenePtzCamera`), its own
  `#[derive(clap::Args)] Args` struct, and `run(args)`; add a `SceneCommand`
  variant in `src/main.rs`. Copy an existing scene as the template — the
  Time-panel builder and `set_camera_target` helper are deliberately
  duplicated per scene (see `architecture.md`).
- `run()` must call `scene::init()` (= `init_satkit`) before any satkit use;
  Python-paneled scenes then call `engine::py::init()` (inittab strictly
  before interpreter init) before constructing the scene.
- Scene structs hold `clock`, `camera`, `camera_target` as direct fields; no
  scene stores a `CelestialSphere` (evaluate `CelestialSphere::at` on the
  spot). Clock access only through the `SceneClock` trait API; the clock tick
  is the application's job (`tick_scene`), never the scene's.
- Body scenes hold per-kind `Vec` fields (`orbital_bodies: Vec<OrbitalBody>`
  and/or `kinematic_bodies: Vec<KinematicBody>` — only the kinds the scene
  tracks) and convert them per frame: `state_at`/`trail` + `body_occluded`
  -> `TrackedBody` in `frame_state`, `state_at` -> `BodyTelemetry` in
  `get_drawables`. Empty (no tracked bodies) scenes set the clock start
  directly from the event datetime. A scene may seed a launch framing with
  `PtzCamera::looking_toward`
  against a throwaway sphere at the epoch — seed `camera_target` with the
  same body so the first frame does not reframe.
- Panel callbacks receive the scene as `&mut Self` at fire time and must be
  idempotent (see `architecture.md`); camera-target keys guard with
  `same_kind` and reframe before writing the target.
- `manual_control`'s burn keys set per-key request flags instead of acting
  directly — opposing held keys must cancel into one direction before the
  single `apply_thrust` call in `advance()` (the body dt-scales it, so a
  paused clock burns nothing).

## EOP valid time range (load-bearing accuracy constraint)

Every scene's epoch window **must** fall inside `[1962-01-01, last
EOP-All.csv entry]` (~build date); outside it satkit silently degrades (see
`simulation.md`). Out-of-range = does not meet the accuracy bar — flag it
rather than shipping. Exception: the `headless` binary deliberately does NOT
enforce this (the caller owns the time; do not add a check).

## Python-paneled scenes (none shipped right now)

The `*_py` scene pair and its `scenes/*.py` scripts were removed 2026-07-12
(owner: scripting returns later; recover them from git history). The engine's
Python interfaces (`engine::py`, `ui::py`) stay alive for that return, and
these rules bind any future Python-paneled scene: a clone of a Rust sibling
whose `get_drawables` delegates to the `--script` Python file, kept side by
side so the two panel APIs can be compared (keep the pair's panels in sync).

- **Split struct**: scene state lives in an Inner `#[pyclass]` (reaches
  Python only as the instance handed to the script — not registered in the
  `globe` module); the thin wrapper implements the four engine traits.
- **Clock/camera/target are plain wrapper fields, outside the pyclass** — a
  script has no direct clock/camera surface, and only a plain field can hand
  out the `&mut` that `SceneClock`/`ScenePtzCamera` require (a pyclass cell's
  borrow guard cannot).
- The Inner exposes a **snapshot/request mirror** of the wrapper-owned state:
  getters return snapshots the wrapper refreshes each frame (so both
  discard-pass fires of a callback read the same value — idempotent); setters
  record `requested_*` values the wrapper folds in before the next tick —
  the same one-frame timing as the Rust siblings' direct callbacks. The
  wrapper overrides `tick_scene` to fold clock requests; camera-target
  requests fold in `frame_state`.
- **The one borrow rule: never hold a pyclass borrow across a call into
  Python.** Trait bodies call no Python; `get_drawables` holds no Rust borrow
  at all — the script's property accesses each take their own transient
  borrow, and callbacks fire later during the egui pass when no borrow is
  live.
- **Error policy**: script load failure and a per-frame `get_drawables`
  exception print the traceback and **panic** (they would recur every frame —
  fail fast); a **callback** exception prints and continues (panicking
  mid-egui-pass would unwind through the presenter).
- **Script contract**: module-level `get_drawables(scene) -> list[Panel]`,
  importing from the embedded `globe` module; the clock is driven through the
  scene's own properties (nothing clock-shaped crosses into Python — each
  script owns its speed-slider min/max constants, like the Rust scenes own
  theirs). A camera-target key loop needs the `lambda i=i:` capture
  (Python's late binding).
