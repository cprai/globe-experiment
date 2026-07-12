---
name: codebase-search
description: Search and explore code by symbol instead of grep/glob/whole-file reads — the preferred, token-cheap way to find definitions, callers, call chains, and structure. Prefer serena (LSP, live, covers Rust + WGSL + Python) for symbol lookup/navigation/references; fall back to codebase-memory-mcp for its unique strengths (call-graph tracing, Cypher/aggregation queries, complexity hotspots, and indexing dependency-crate source). Use whenever code needs to be searched or before planning major code changes.
---

# Codebase search (serena first, codebase-memory-mcp for graph work)

**Prefer symbol search over grep/glob/whole-file reads.** It returns exact
symbols and precise source ranges instead of raw file dumps — this exists to
**save token cost**. Two MCP tools cover it; route by what you need.

## Which tool (routing)

| Need | Tool |
|---|---|
| File/symbol structure overview | **serena** `get_symbols_overview` |
| Find a definition by name/path | **serena** `find_symbol` (`include_body=true` for source) |
| Who *references* a symbol | **serena** `find_referencing_symbols` |
| Implementations / declaration of a trait or item | **serena** `find_implementations` / `find_declaration` |
| LSP diagnostics for one file | **serena** `get_diagnostics_for_file` |
| **Call-graph tracing** (who calls X / what X calls) | **codebase-memory** `trace_path(mode="calls")` |
| **Data-flow** through a chain | **codebase-memory** `trace_path(mode="data_flow")` |
| **Cypher / aggregation / complexity hotspots / dead code** | **codebase-memory** `query_graph(...)` |
| **Dependency-crate source** (satkit, wgpu, egui, ...) | **codebase-memory** (index the crate first — see below) |
| Concept search when no symbol name is known | either: serena `find_symbol` (substring) or codebase-memory `search_graph`/`search_code` |

Rule of thumb: **serena for "where is this symbol / who uses it"** across any
supported language; **codebase-memory for graph questions** (call chains,
aggregations) and for reading dependency source.

---

## serena (primary) — LSP-backed, live, multi-language

Serena drives real language servers, so it is **live** (reads the working
tree — there is **no index to refresh**, unlike codebase-memory) and
LSP-accurate. It covers exactly the languages listed in
`.serena/project.yml` → `languages:`:

- **rust** → rust-analyzer (the whole `src/` engine + `build.rs`).
- **hlsl** → shader-language-server, which handles **`src/engine/shaders/scene.wgsl`**.
- **python** → pyright, for the runtime scene scripts **`scenes/*.py`**.

Core tools: `get_symbols_overview(relative_path)`,
`find_symbol(name_path_pattern, relative_path=..., include_body=...)`,
`find_referencing_symbols`, `find_implementations`, `find_declaration`,
`get_diagnostics_for_file`. Symbol edits (`replace_symbol_body`,
`insert_after_symbol`, ...) exist too but `Read`-then-`Edit` remains the
normal edit flow here.

Notes learned on this repo:

- **WGSL symbol output is coarse.** shader-language-server's `documentSymbol`
  for WGSL lists structs (labeled `TypeParameter`) and only a fraction of the
  functions, plus a flood of locals. Good enough to *locate* things; don't
  expect rust-analyzer-grade navigation on the shader. **naga stays the
  authoritative WGSL error/lint check** (see `validate-wgsl-naga`) — serena is
  for *querying* the shader, not validating it.
- **Adding a language is config, not code.** A language only becomes queryable
  once its key is in `.serena/project.yml` **and** its language-server binary
  is present; the change takes effect on a serena restart (`/mcp` reconnect).
  Only `.serena/project.yml` is versioned — the rest of `.serena/` (cache,
  memories, downloaded servers) is gitignored/machine-local.
- A benign `Pydantic V1 … Python 3.14` warning prints at startup; ignore it.

---

## codebase-memory-mcp (graph work + dependencies)

Kept for what an LSP can't do: call-graph tracing, Cypher/aggregation
queries, complexity hotspots, dead-code sweeps, and reading dependency-crate
source. It is a **snapshot graph of Rust code only** — so it needs indexing.

### Re-index rule (both directions)

- **Before graph searching**: re-run `index_repository` so the graph matches
  the working tree. The graph does NOT track edits.
- **After any Rust change**: re-index again so later `trace_path`/`query_graph`
  calls see the new code.
- Indexing this project is fast (~800 nodes, seconds); re-index liberally.
  (serena needs none of this — this rule is codebase-memory-only.)

```
index_repository(repo_path="/workspaces/globe-experiment")
```

The project's graph name is **`workspaces-globe-experiment`** (confirm with
`list_projects` if a call errors on it). `index_status(project=...)` reports
freshness. Default (`full`) mode is fine.

### Graph tools

| Need | Tool |
|---|---|
| Who calls X / what does X call | `trace_path(function_name, mode="calls")` |
| Data-flow through a chain | `trace_path(mode="data_flow")` |
| Structure overview | `get_architecture(aspects=[...])` |
| Find a symbol by name/concept | `search_graph(project, query=... or name_pattern=".*Rx.*", label="Class"/"Enum"/"Interface"/"Function"/"Method")` |
| Read one symbol's source | `get_code_snippet(qualified_name)` |
| Text search when a name is unknown | `search_code(pattern)` — graph-augmented grep |
| Multi-hop / aggregation / complexity hotspots | `query_graph(query="MATCH ... RETURN ...")` (Cypher) |

Notes on the graph:

- Rust structs are labeled `Class`, traits `Interface`, enums `Enum`.
- `query_graph` does not accept multi-label `OR` like `(n:Enum OR n:Interface)`
  — run one query per label, or match unlabeled and filter on properties.
- `search_graph` caps results (default 200) — check `has_more` and paginate
  with `offset`, or narrow with `label`/`file_pattern`.

### Dependencies: index the crate before referencing it

To reference a dependency's source (satkit, wgpu, egui, winit, ...), **index
that crate first** and search its graph instead of reading registry files raw.
Each crate indexes as its own separate project (graphs do not link):

```
index_repository(repo_path="$CARGO_HOME/registry/src/<index>/<crate>-<version>", mode="fast")
```

Find the path with `ls $CARGO_HOME/registry/src/*/ | grep <crate>`
(`CARGO_HOME` is `/usr/local/cargo` here). Registry sources are immutable per
version, so a dependency needs indexing **once** — the re-index rule does not
apply to deps; a version bump changes the path, so index the new version's dir.

---

## When plain tools are still right

- **Genuinely non-code files** — `Cargo.toml`, `.claude/` docs, `OUT_DIR`,
  `build.rs` asset URL lists — Grep/Read.
- **Always `Read` a file before editing it.** Both MCPs locate code; edits
  still go through the normal Read → Edit flow.
