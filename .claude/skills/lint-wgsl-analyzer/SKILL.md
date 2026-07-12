---
name: lint-wgsl-analyzer
description: Get a secondary, spec-strict second opinion on engine/src/shaders/scene.wgsl via the wgsl-analyzer LSP server (pull diagnostics). Use when you want an editor-grade recheck; it is stricter than naga so expect false positives. The naga CLI remains authoritative.
---

# Lint WGSL with wgsl-analyzer (secondary)

A **secondary**, spec-strict second opinion on `engine/src/shaders/scene.wgsl`. The
naga CLI (the `validate-wgsl-naga` skill) is the authoritative check (it's
the real compiler); reach for wgsl-analyzer only when you want an
editor-grade recheck. It is **spec-stricter than naga**, so expect false
positives relative to what actually compiles — confirm any error against an
actual run before assuming the shader is broken.

## Tools
- `wgsl-analyzer` (driven via its **LSP server** only)

## How to run (verified 2026-06-20)
- **The CLI subcommands are stubs.** `wgsl-analyzer parse` / `diagnostics`
  / `unresolved-references` panic with "subcommand not implemented", and
  `--print-config-schema` prints nothing. Don't use them.
- **The only working path is the LSP server** (`wgsl-analyzer` with no
  subcommand, JSON-RPC over stdio), and it uses **pull** diagnostics
  (`textDocument/diagnostic`), *not* push (`publishDiagnostics` is never
  sent). Sequence:
  1. `initialize`
  2. `initialized`
  3. `textDocument/didOpen` (send the shader source)
  4. request `textDocument/diagnostic` and read the `items` from the
     response.
- A `didOpen`-and-wait-for-push client sees nothing (a dead end). The editor
  LSP tooling here is **not** wired for `.wgsl`, so this is a hand-driven
  JSON-RPC client task.

## Known false positive to expect
- It enforces WGSL's rule that operands of `&`/`|`/`^` be unary or
  parenthesized: the `hash3` bit-mix (`a*b ^ c*d ^ e*f`) is flagged until
  the multiplications are wrapped in `()` (naga accepts it unparenthesized).
  Treat its errors as worth investigating, but confirm against an actual run.
