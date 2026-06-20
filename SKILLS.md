# SKILLS.md

Index of the common, repeatable development tasks for **Globe**
(`globe-experiment`). Each task is documented as a self-contained skill in
`skills/`, with the exact command(s) and the tools it needs. The detail
(why each step matters, the traps) lives in `CLAUDE.md` and `MEMORY.md`;
this is the operational quick-reference.

These skills are derived from `CLAUDE.md` ("Build & run", "Code style",
"Testing & verification") and `MEMORY.md`. When a documented constant,
command, or path changes, update the matching skill in the same change
(the "keep docs current" rule applies here too).

## Tools used across the skills

| Tool | What it is | Used by |
|------|------------|---------|
| `cargo` (stable) | build/run/lint toolchain | [build-and-run](skills/build-and-run.md), [smoke-test](skills/smoke-test.md), [clippy-lint](skills/clippy-lint.md) |
| `cargo +nightly fmt` | Rust formatter (nightly for unstable `wrap_comments`) | [format-rust](skills/format-rust.md) |
| `wgslfmt` | WGSL formatter (whitespace/layout only, ASCII-safe) | [format-wgsl](skills/format-wgsl.md) |
| `naga` CLI (29.x) | authoritative static WGSL validator (same naga wgpu links) | [validate-wgsl-naga](skills/validate-wgsl-naga.md) |
| `wgsl-analyzer` | secondary spec-strict WGSL linter (LSP server only) | [lint-wgsl-analyzer](skills/lint-wgsl-analyzer.md) |

## Skills

| Skill | Task | When to run |
|-------|------|-------------|
| [build-and-run](skills/build-and-run.md) | Build & run the app | Any change you want to see live |
| [smoke-test](skills/smoke-test.md) | Headless run to validate pipelines/bindings | After renderer/pipeline/binding changes |
| [format-rust](skills/format-rust.md) | Format all `.rs` with nightly rustfmt | After every `.rs` edit |
| [format-wgsl](skills/format-wgsl.md) | Format `shaders/globe.wgsl` with `wgslfmt` | After every shader edit |
| [validate-wgsl-naga](skills/validate-wgsl-naga.md) | Statically validate the shader with naga | After every shader edit (authoritative) |
| [lint-wgsl-analyzer](skills/lint-wgsl-analyzer.md) | Spec-strict second-opinion WGSL lint | When you want an editor-grade recheck |
| [clippy-lint](skills/clippy-lint.md) | Lint Rust with clippy | After every `.rs` change |
| [add-scenario](skills/add-scenario.md) | Add a new past scenario + CLI wiring | Adding a tracked event |
| [edit-atmosphere-constants](skills/edit-atmosphere-constants.md) | Change atmosphere medium/LUT constants in sync | Touching atmosphere math |
| [refresh-embedded-assets](skills/refresh-embedded-assets.md) | Re-download/re-encode a baked asset | Refreshing a texture or EOP snapshot |

## Typical edit loops

- **Shader edit:** [format-wgsl](skills/format-wgsl.md) →
  [validate-wgsl-naga](skills/validate-wgsl-naga.md) →
  [build-and-run](skills/build-and-run.md) (look/binding correctness can
  only be seen at runtime).
- **Rust edit:** [format-rust](skills/format-rust.md) →
  [clippy-lint](skills/clippy-lint.md) →
  [smoke-test](skills/smoke-test.md) or [build-and-run](skills/build-and-run.md).
- **Atmosphere edit:** [edit-atmosphere-constants](skills/edit-atmosphere-constants.md)
  (both sides) → [format-wgsl](skills/format-wgsl.md) +
  [validate-wgsl-naga](skills/validate-wgsl-naga.md) →
  [build-and-run](skills/build-and-run.md).
