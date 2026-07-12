---
paths:
  - "src/**/*.rs"
  - "build.rs"
  - "shaders/scene.wgsl"
  - "scenes/**/*.py"
---

# Code style & conventions

## Comment rules

Keep comment volume low; a comment must earn its tokens.

- Comments explain the WHY only: hidden constraints, precision rationale,
  workarounds for specific bugs, "must match X" sync warnings, "deliberately
  X - do not Y" markers, and math/algorithm/reference-frame/astronomy
  derivations - all terse (1-4 lines).
- Never write comments that: restate what the code plainly does; describe
  relationships queryable by the code search tools (callers, "the twin of
  X", where something is used); recount project history; explain well-known
  crate APIs; or duplicate what a `.claude/rules/` file states (leave at
  most a one-line marker at the exact line where violating the rule would
  look like an inviting cleanup).
- State each fact once, in the place it belongs; elsewhere use nothing or a
  terse pointer.
- Doc comments on items: 1-3 lines stating contract/units/frame. Exception:
  clap `///` docs render as `--help` text - keep them user-facing.

## Conventions

- Small focused structs, descriptive names; match surrounding code.
- **TLE data**: inline `const`s in the scene files, never in `satellite.rs`;
  the `ISS_TLE` literal is deliberately duplicated across scenes - do not
  factor into a shared const.
- Look/feel/atmosphere constants have fixed homes: shader look knobs at the
  top of `scene.wgsl`, input feel at the top of `camera/ptz.rs`, body
  physical constants in `planet.rs`, projection consts in `renderer`,
  atmosphere medium constants in `build.rs` + `scene.wgsl` (synced - see
  `atmosphere.md`).
- Keep docs current in the same change (code comments, `.claude/` rules,
  `README.md`) - stale docs are bugs; see `documentation.md` for what
  belongs in docs. The source is authoritative for any constant value.
