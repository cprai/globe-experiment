---
name: format-python
description: Format the Python scene scripts (scenes/*.py) with ruff format, the formatting authority for Python. Use after every edit to a scenes/*.py file. Ruff is ASCII-safe and touches only layout; do not hand-format.
---

# Format Python code (ruff)

Format any Python in the repo — the runtime scene scripts under `scenes/`
(none checked in right now; scripting returns later). **`ruff format` is the
sole formatting authority for Python** here — don't hand-format, and keep
diffs limited to real changes.

## Tools
- `ruff` (`ruff format`)

## Command
```sh
ruff format scenes/
```
Check-only (no writes — reports which files *would* change):
```sh
ruff format --check scenes/
```

## Scope
- Runs on the `scenes/*.py` scene scripts, the only hand-written Python this
  repo ever holds (everything else Python-side is embedded CPython driven
  from Rust in `crates/engine/src/py.rs`).
- Formatting only. It does **not** type-check — verify types separately with
  the `check-python-ty` skill (`ty check`).
- No `pyproject.toml`/`ruff.toml` is checked in, so ruff uses its built-in
  defaults (Black-compatible). If project-specific rules are ever wanted, add a
  config file rather than hand-tuning.

## Notes
- ruff is a single fast Rust binary (installed via `uv`/`cargo`); like
  `wgslfmt` it only rewrites whitespace/layout, never tokens, and is
  ASCII-safe.
- Run it **after every `scenes/*.py` edit**, the same discipline as
  `cargo +nightly fmt` for Rust and `wgslfmt` for the shader.
