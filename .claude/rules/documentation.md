---
paths:
  - ".claude/**/*.md"
  - "README.md"
---

# Documentation guidelines (`.claude/` rules + README)

Docs exist to cut agent token cost, not to mirror the code. The 2026-07
reduction pass brought `.claude/` from ~3,900 to ~1,000 lines; keep it there.

- **Only document what an agent cannot recover from the code search tools.**
  Structure, signatures, callers, per-file contents, and constant values are
  queryable - never enumerate them. The source is authoritative when they
  disagree.
- **High-level and stable.** `architecture.md` describes modules and
  invariants at an altitude that only changes on major refactors. Prefer
  wording that survives code churn (name concepts and rules, not line-level
  details).
- **One canonical home per fact.** Cross-cutting stories live in exactly one
  file (floating origin/render frame: `camera.md`; reversed-Z: `renderer.md`;
  EOP/satkit behavior: `simulation.md`; Python-scene rules: `scenes.md`);
  everywhere else at most a one-line pointer. Before adding a fact, check it
  is not already stated elsewhere.
- **What earns its place**: the non-obvious WHY (owner decisions, rejected
  designs, platform quirks, accuracy constraints), math/algorithms/reference
  frames/astronomy (terse), sync-across-files warnings, and verification
  procedures. No constant-value snapshots, no history narration (git log has
  it), no restating code.
- **Minimize cross-module docs**: a rule that spans modules should still be
  written once, in the file whose path scope best matches where it bites.
- **Path-scope rules files** (`paths:` frontmatter) unless they are genuinely
  load-bearing for every session; launch-loaded files (`CLAUDE.md`,
  `architecture.md`, `constraints.md`, `platform.md`, `rejected.md`,
  `testing.md`, `backlog.md`) cost tokens in every conversation - keep them
  smallest.
- There is no `HISTORY.md` and no changelog doc - do not create one.
