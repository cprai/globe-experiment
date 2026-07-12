---
paths:
  - "src/**/*.rs"
  - "build.rs"
---

# Code style & conventions

- Comments explain the WHY (hidden constraint, subtle invariant, workaround
  for a specific bug) — never what the code does, the caller, or the task.
- Small focused structs, descriptive names; match surrounding code.
- **TLE data**: inline `const`s in the scene files, never in `satellite.rs`;
  the `ISS_TLE` literal is deliberately duplicated across scenes — do not
  factor into a shared const.
- Look/feel/atmosphere constants have fixed homes: shader look knobs at the
  top of `scene.wgsl`, input feel at the top of `camera/ptz.rs`, body
  physical constants in `planet.rs`, projection consts in `renderer`,
  atmosphere medium constants in `build.rs` + `scene.wgsl` (synced — see
  `atmosphere.md`).
- Keep docs current in the same change (code comments, `.claude/` rules,
  `README.md`) — stale docs are bugs. The source is authoritative for any
  constant value.
