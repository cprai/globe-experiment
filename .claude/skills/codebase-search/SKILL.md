---
name: codebase-search
description: Search and explore code through the codebase-memory-mcp knowledge graph instead of grep/glob/whole-file reads — the preferred, token-cheap way to find definitions, callers, call chains, and structure. Use whenever code needs to be searched or before planning major code changes. ALWAYS re-index first (indexing is fast on this project), and re-index again after any code change. Also covers referencing dependency source (satkit, wgpu, egui, ...): index the crate's registry dir first, then search its graph.
---

# Codebase graph search (codebase-memory-mcp)

Use the `codebase-memory-mcp` knowledge graph as the **preferred way to search
code** in this repo. Graph queries return exact symbols, precise source ranges,
and resolved call edges instead of raw file dumps — this exists to **save
token cost**: prefer `get_code_snippet` over reading a whole file, and
`search_graph`/`trace_path` over grep sweeps.

The project's graph name is **`workspaces-globe-experiment`** (derived from
the repo path; confirm with `list_projects` if a call errors on it).

## Re-index rule (both directions)

- **Before any searching**: re-run `index_repository` so the graph matches the
  working tree. The graph is a snapshot — it does NOT track edits.
- **After any code change**: re-index again so later queries (yours or another
  agent's) see the new code.
- Indexing this project is fast (~800 nodes, seconds), so re-indexing
  liberally is fine. Do not skip it to "save time".

```
index_repository(repo_path="/workspaces/globe-experiment")
```

Default (`full`) mode is fine here. `index_status(project=...)` reports
freshness if unsure whether a re-index already happened this session.

## Tool cheat sheet

| Need | Tool |
|---|---|
| Find a function/struct/trait by name or concept | `search_graph(project, query="..." or name_pattern=".*Regex.*", label="Class"/"Enum"/"Interface"/"Function"/"Method")` |
| Read one symbol's source (exact range) | `get_code_snippet(qualified_name)` — instead of `Read` on the whole file |
| Who calls X / what does X call | `trace_path(function_name, mode="calls")` |
| Data-flow through a chain | `trace_path(mode="data_flow")` |
| Structure overview | `get_architecture(aspects=[...])` |
| Text search when a symbol name is unknown | `search_code(pattern)` — graph-augmented grep |
| Complex/multi-hop questions, aggregations, complexity hotspots | `query_graph(query="MATCH ... RETURN ...")` (Cypher) |

Notes learned on this repo:

- Rust structs are labeled `Class`, traits `Interface`, enums `Enum`.
- `query_graph` does not accept multi-label `OR` patterns like
  `(n:Enum OR n:Interface)` — run one query per label, or match unlabeled and
  filter on properties.
- `search_graph` caps results (default 200) — check `has_more` and paginate
  with `offset`, or narrow with `label`/`file_pattern` first.

## Dependencies: index the crate before referencing it

Any time you need to reference a dependency's source (satkit, wgpu, egui,
winit, ...), **index that crate first** and search its graph instead of
reading registry files raw. Each crate indexes as its own separate project
(the graphs do not link — no cross-project call edges), named after its path:

```
index_repository(repo_path="$CARGO_HOME/registry/src/<index>/<crate>-<version>", mode="fast")
```

Find the exact path with `ls $CARGO_HOME/registry/src/*/ | grep <crate>`
(CARGO_HOME is `/usr/local/cargo` here). Registry sources are immutable per
version, so a dependency needs indexing **once** — the re-index rule above
does not apply to deps; `list_projects` shows if it is already indexed. A
version bump changes the path, so just index the new version's dir.

## When plain tools are still right

The graph indexes Rust code only. Keep using Grep/Glob/Read for:

- `shaders/scene.wgsl`, `build.rs` asset lists, `Cargo.toml`, `.claude/` docs,
  and other non-code or config files.
- **Always `Read` a file before editing it** — the graph locates code; edits
  still go through the normal Read → Edit flow.
