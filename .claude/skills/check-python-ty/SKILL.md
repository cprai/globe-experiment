---
name: check-python-ty
description: Type-check the Python scene scripts (scenes/*.py) with ty (Astral's type checker) after edits. Use after every scenes/*.py change to catch type errors. The embedded `globe` pyo3 module resolves as unresolved (no stubs yet) — that specific failure is expected and ignored for now; fix anything else ty reports.
---

# Verify Python code (ty)

Type-check the runtime scene scripts under `scenes/` after **every** edit.
`ty` is Astral's fast Rust type checker; run it to catch type errors the way
`clippy` catches Rust footguns.

## Tools
- `ty` (`ty check`)

## Command
```sh
ty check scenes/
```

## Expected failure — ignore for now
The scene scripts `import` the embedded **`globe`** module, which only exists
at runtime inside the app (it is a pyo3 module registered from Rust in
`crates/engine/src/py.rs`). No type stubs exist for it, so ty reports it as an
**unresolved import** and exits non-zero. **This is expected and ignored for
now** (owner: types are not set up properly yet). Read past those `globe`
diagnostics and fix any *other* type error ty surfaces.

- The future proper fix is a `globe.pyi` stub (or a configured stub path) so ty
  can resolve the embedded API; until then, the unresolved-import noise stands.

## Notes
- **Format first** with the `format-python` skill (`ruff format`); ty checks
  types, not layout.
- serena's `get_diagnostics_for_file` gives the same in-session view via
  pyright (serena's configured Python server) if you'd rather see diagnostics
  without leaving the tool loop — but `ty check` is the canonical CLI pass.
- Like clippy for Rust, a clean type check is necessary but not sufficient:
  a scene script only truly exercises through the app (a Python-paneled
  scene's `--script` argument — see `scenes.md`).
