# ARCHITECTURE.md

A running map of how `codesight-mcp` is put together. Read this before touching
crate boundaries, data flow, or the MCP surface. For configuration knobs, see
[`CONFIGURATION.md`](CONFIGURATION.md). For active work and open issues see
[`REFACTOR_WORKLOG.md`](REFACTOR_WORKLOG.md).

## Crate layout

```
codesight-mcp/                          ← product crate (root Cargo.toml)
├── src/
│   ├── cli/                            ← operator subcommands (`check`)
│   ├── config/                         ← layered AppConfig + env + trust
│   ├── service/                        ← MCP boundary + agent-v1 dispatch
│   ├── indexing/                       ← crawl, delta, imports, packages, dep_graph
│   ├── framework/                      ← dbt / laravel / nextjs / nuxt detection
│   ├── retrieval/                      ← ranking, summarization, sections
│   ├── server_support/                 ← repo-id, path map, reindex state
│   ├── security/                       ← binary policy, secret policy, paths
│   └── tools.json                      ← frozen MCP contract
└── crates/
    ├── codesight-analysis/             ← graph, quality, dead-code, readiness, evidence
    ├── codesight-core/                 ← shared primitives (symbols, fact quality)
    ├── codesight-extractors/           ← SCM engine + curated query packs
    └── codesight-storage/              ← SQLite schema + migrations + reads/writes
```

**Dependency direction is inward.** `codesight-core` is the base; the
`extractors` and `storage` crates depend on it; the product crate depends on
all three. Nothing in the crates depends on the product layer.

### Workspace dependency graph (current snapshot)

Extracted from `cargo metadata --format-version 1 --no-deps` on 2026-04-10;
regenerate whenever a workspace crate is added or an edge changes.

| From → To             | `codesight-core` | `codesight-extractors` | `codesight-storage` | `codesight-analysis` | `codesight-mcp` |
|-----------------------|:----------------:|:----------------------:|:-------------------:|:--------------------:|:---------------:|
| `codesight-core`      |        —         |           ✗            |          ✗          |          ✗           |        ✗        |
| `codesight-extractors`|        ✓         |           —            |          ✗          |          ✗           |        ✗        |
| `codesight-storage`   |        ✓         |           ✗            |          —          |          ✗           |        ✗        |
| `codesight-analysis`  |        ✓         |           ✗            |          ✓          |          —           |        ✗        |
| `codesight-mcp`       |        ✓         |           ✓            |          ✓          |          ✓           |        —        |

Rules enforced by this table:

- `codesight-core` is a leaf — it depends on no other workspace crate.
- `codesight-extractors` and `codesight-storage` do not depend on each other
  (no horizontal edges between the two middle crates).
- `codesight-mcp` is the only consumer that pulls in all three inner crates.
- Additional workspace crates are allowed when they preserve these dependency
  invariants and own a coherent domain boundary.
- Any new crate member or new edge requires updating this table **and** the
  relevant worklog entry in [`REFACTOR_WORKLOG.md`](REFACTOR_WORKLOG.md) in the
  same patch.


### Why this split

- `codesight-storage` and `codesight-extractors` are the two pieces most likely
  to get heavier over time (new languages, new query packs, schema evolution).
  Isolating them shortens the compile loop for the rest of the product.
- `codesight-core` holds only primitives that are legitimately shared: the
  `Symbol` / `FactQuality` taxonomy, small serde DTOs, the error marker traits.
  It is deliberately small so it stays stable.
- Everything else — ingest orchestration, MCP dispatch, framework detection,
  retrieval, analysis — lives in the product crate today. Future splits should
  prefer self-contained domains that reduce root-crate dependency load before
  carving out tightly-coupled orchestration code.

### Planned split roadmap

The root crate is not being split indiscriminately. A module becomes a crate
only when it owns a coherent domain, exports a narrow API, and removes real
compile and dependency weight from `codesight-mcp`.

Shipped: `codesight-analysis` (2026-04-15) — carved `src/analysis/**` out of the
root crate except for `import_graph` (moved to `src/indexing/dep_graph.rs`
alongside its `indexing::imports` inputs) and `reference_check` (moved under
`src/service/agent_v1/reference_check/` since it reaches into both
`indexing::imports` and `indexing::packages`). `doc_coverage_payload` stays in
the root crate (`src/service/agent_v1/quality/doc_coverage.rs`) because
`DocStore` still lives in `src/storage/doc_index/`; promote that next.

| Candidate crate | Current scope | Feasibility | What we gain | Main blockers / prerequisites |
|-----------------|---------------|-------------|--------------|-------------------------------|
| `codesight-framework` | `src/framework/**` | High | Removes a self-contained framework domain from the root crate; trims framework-specific parser dependencies from the product crate; keeps dbt / Laravel / Next.js / Nuxt work behind one library boundary | Keep `FrameworkProvider`, `FileContext`, `ExtraImport`, registry assembly, and profile rendering owned by the crate; expose a small facade instead of leaking root-only helpers |
| `codesight-imports` | `src/indexing/imports/**` plus `src/indexing/packages/**` plus `src/indexing/dep_graph.rs` | Medium-high | Pulls multilingual import extraction, package registry logic, and file-level dependency adjacency out of the root crate; gives service code one dependency-facts library; isolates more tree-sitter language dependencies from MCP/service code | Decide whether package-registry code stays attached to import extraction; once shipped, `src/service/agent_v1/reference_check/` can either move into `codesight-analysis` or stay where it is as a thin service facade |
| `codesight-retrieval` | `src/retrieval/**` | Medium | Isolates ranking, section parsing, embedding/model loading, and retrieval helpers from MCP dispatch; shrinks the root crate's hot edit surface for search work | Move config-driven behavior behind small request/config structs; keep storage and service integration at the API boundary rather than through root-module imports |

Planned order:

1. `codesight-framework`
2. `codesight-imports`
3. `codesight-retrieval`

Modules that should stay in the root crate for now:

- `src/service/**`, `src/mcp.rs`, and `src/cli/**` are orchestration and public-boundary code. They should depend on inner crates, not become them.
- `src/config/**`, `src/security/**`, and `src/server_support/**` are too small and too boundary-oriented to justify workspace overhead.
- `src/indexing/code_index.rs` and watcher/freshness coordination stay in the product crate until the import/dependency subdomain is extracted cleanly.
- `codesight-core` stays deliberately small. It is not the place to park every type that happens to cross a crate boundary.

## Data flow: `ingest_sources` → retrieval

```
MCP client
  │  JSON-RPC over stdio
  ▼
src/mcp.rs
  │  parse request, route to dispatch
  ▼
src/service/agent_v1/mod.rs::dispatch
  │  1. coerce-then-validate against src/tools.json  (P4)
  │  2. routes to registry::dispatch_tool
  ▼
src/service/agent_v1/<tool>.rs (e.g. ingest.rs, catalog_search.rs)
  │  validate args, shape response envelope
  ▼
src/indexing/code_index.rs  (ingest)
  │  crawl → extractors → storage
  ▼
crates/codesight-extractors
  │  SCM engine + curated query packs (tree-sitter)
  ▼
crates/codesight-storage
     WAL-configured SQLite, forward-only migrations
```

### Reindex lifecycle and freshness

`ingest_sources` wraps its actual indexing work in a RAII
`ReindexGuard` that calls `ReindexStateManager::mark_reindex_start` at entry
and either `mark_reindex_done` on success or `mark_reindex_failed` on drop.
The manager exposes strict / relaxed freshness mode. When
`CODESIGHT_STRICT_FRESHNESS=true` (or the equivalent config), the manager
is initialized in strict mode; read handlers that opt into
`await_freshness_if_strict` block until a mid-flight reindex completes.

## MCP contract

`src/tools.json` is the single frozen contract. Two rules:

1. `agent-v1` is the only public surface. No parallel dispatch trees, no
   hidden tool aliases.
2. Schema changes to `tools.json` land in the same patch as the handler
   changes and the test updates. See `AGENTS.md` "Working rules".

Validation is done in `src/service/agent_v1/validate.rs`: `tools.json` is
embedded at compile time, each `inputSchema` is compiled once into a
`jsonschema::Validator`, and dispatch runs coerce→validate before any
handler sees the arguments. Coercion promotes stringified scalars
(`"true"`, `"42"`) into their typed form when the schema calls for it.

## FactQuality taxonomy

Extraction and analysis both tag facts with a coarse `FactQuality` value:

| Tier                   | Source                                            |
|------------------------|---------------------------------------------------|
| `trusted_structural`   | Curated tree-sitter query packs, grammar-backed.  |
| `baseline_structural`  | Generic AST fallback, language-agnostic walker.   |
| `heuristic`            | Residual Rust overrides, regex fallbacks.         |

Downstream graph and quality analysis default to the structural tiers;
heuristic evidence is labeled and treated as secondary. See
`src/analysis/evidence.rs`.

## Known heuristic islands

These exist because upstream grammar crates are still pinned to an
incompatible tree-sitter release. Each has a named exit criterion in its
residual facade:

- `vue`  — waiting on a `tree-sitter-vue` port on tree-sitter ≥ 0.26.
- `razor` — same situation.
- `blade` — same situation.

Everything else should trend toward structural extraction via query packs
or the generic AST fallback. New Rust extractors require stating why Tier
1–3 (query pack / generic AST / container + injection) are insufficient.

## Observability

All logs go through `tracing`; library code contains no bare `eprintln!` or
`println!`. The subscriber is initialized in `src/main.rs` to write to
**stderr** only so JSON-RPC on stdout stays clean. Log format is selectable
via `--log-format json|pretty` or `CODESIGHT_LOG_FORMAT`. Filtering uses
`RUST_LOG` with the standard `EnvFilter` syntax; the default fallback is
`warn,codesight_mcp=info,codesight_storage=info,codesight_extractors=info`.

Dispatch, ingest, and extraction boundaries are instrumented with spans;
the per-file extract span is `DEBUG` so hot loops stay quiet by default.

## SQLite configuration

Every connection flows through `codesight_storage::db::open_rw` /
`open_ro`, which apply the WAL-mode pragmas on open:

- `journal_mode = WAL`
- `synchronous = NORMAL`
- `temp_store = MEMORY`
- `mmap_size = 256 MiB`
- `busy_timeout = 5000`
- `cache_size = 64 MiB`
- `foreign_keys = ON`

The code-index schema has a forward-only migration chain in
`crates/codesight-storage/src/storage/migrations.rs`; the current target is
`CODE_INDEX_TARGET_USER_VERSION`, and `PRAGMA user_version` is bumped
inside the migration transaction.
