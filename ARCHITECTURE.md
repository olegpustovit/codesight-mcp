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
│   ├── indexing/                       ← crawl, delta orchestration (code_index, dep_graph)
│   ├── server_support/                 ← repo-id, path map, reindex state
│   ├── security/                       ← binary policy, secret policy, paths
│   └── tools.json                      ← frozen MCP contract
└── crates/
    ├── codesight-analysis/             ← graph, quality, dead-code, readiness, evidence
    ├── codesight-core/                 ← shared primitives (symbols, fact quality)
    ├── codesight-extractors/           ← SCM engine + curated query packs
    ├── codesight-framework/            ← dbt / laravel / nextjs / nuxt detection
    ├── codesight-imports/              ← multilingual import extraction + package manifest registry
    ├── codesight-retrieval/            ← ranking, summarization, section parsing, embedder
    └── codesight-storage/              ← SQLite schema + migrations + reads/writes
```

**Dependency direction is inward.** `codesight-core` is the base; `extractors`
and `storage` depend on it; `framework` depends on `extractors` (for shared
`parse_tree`); `imports` depends on both core and extractors; `analysis` and
`retrieval` both depend on core + storage. The product crate depends on every
inner crate. Nothing in the crates depends on the product layer.

### Workspace dependency graph (current snapshot)

Extracted from `cargo metadata --format-version 1 --no-deps` on 2026-04-15;
regenerate whenever a workspace crate is added or an edge changes.

| From → To              | `codesight-core` | `codesight-analysis` | `codesight-extractors` | `codesight-framework` | `codesight-imports` | `codesight-retrieval` | `codesight-storage` | `codesight-mcp` |
|------------------------|:----------------:|:--------------------:|:----------------------:|:---------------------:|:-------------------:|:---------------------:|:-------------------:|:---------------:|
| `codesight-core`       |        —         |          ✗           |           ✗            |           ✗           |          ✗          |           ✗           |          ✗          |        ✗        |
| `codesight-analysis`   |        ✓         |          —           |           ✗            |           ✗           |          ✗          |           ✗           |          ✓          |        ✗        |
| `codesight-extractors` |        ✓         |          ✗           |           —            |           ✗           |          ✗          |           ✗           |          ✗          |        ✗        |
| `codesight-framework`  |        ✗         |          ✗           |           ✓            |           —           |          ✗          |           ✗           |          ✗          |        ✗        |
| `codesight-imports`    |        ✓         |          ✗           |           ✓            |           ✗           |          —          |           ✗           |          ✗          |        ✗        |
| `codesight-retrieval`  |        ✓         |          ✗           |           ✗            |           ✗           |          ✗          |           —           |          ✓          |        ✗        |
| `codesight-storage`    |        ✓         |          ✗           |           ✗            |           ✗           |          ✗          |           ✗           |          —          |        ✗        |
| `codesight-mcp`        |        ✓         |          ✓           |           ✓            |           ✓           |          ✓          |           ✓           |          ✓          |        —        |

Rules enforced by this table:

- `codesight-core` is a leaf — it depends on no other workspace crate.
- `codesight-extractors` and `codesight-storage` do not depend on each other.
- `codesight-framework` depends only on `codesight-extractors` (for the
  shared `parse_tree` helper) — no storage, no core.
- `codesight-mcp` is the only consumer that pulls in every inner crate.
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
- What still lives in the product crate is the orchestration layer — ingest
  (`src/indexing/code_index.rs`), MCP dispatch (`src/service/`), CLI
  (`src/cli/`), config/security/server-support, and the file-level dependency
  graph (`src/indexing/dep_graph.rs`) that bridges `codesight_imports` into
  ingest. Future splits should prefer self-contained domains that reduce
  root-crate dependency load before carving out tightly-coupled orchestration
  code.

### Shipped splits (2026-04-15)

- `codesight-framework` — dbt / Laravel / Next.js / Nuxt detection,
  `FrameworkProvider` trait, `FrameworkRegistry`, `impl_framework_provider!`
  macro. Depends on `codesight-extractors` only.
- `codesight-imports` — multilingual import extraction + package manifest
  registry. Depends on `codesight-core` and `codesight-extractors`.
- `codesight-retrieval` — ranking, section parsing, fusion/RRF, fuzzy
  matching, summarizer config, dense embedder (bundled model assets now live
  inside the crate). Depends on `codesight-core` and `codesight-storage`.
- `codesight-analysis` — graph (calls, coupling, cycles, layers, pagerank,
  PSR-4), quality, dead-code, agent readiness, evidence, impact, test gaps.
  Depends on `codesight-core` and `codesight-storage`.

### Carve-outs from the analysis split

Three modules stayed in the root crate to keep `codesight-analysis` from
needing a forbidden edge to `codesight-imports`:

- `import_graph` lives as `src/indexing/dep_graph.rs`, next to its
  `codesight_imports::imports` inputs.
- `reference_check` lives at `src/service/agent_v1/reference_check/` — it
  consumes both `codesight_imports::imports` and `codesight_imports::packages`.
- `doc_coverage_payload` is parked at
  `src/service/agent_v1/quality/doc_coverage.rs` until `DocStore` is promoted
  from `src/storage/doc_index/` into `codesight-storage`; once that lands the
  helper can rejoin `codesight_analysis::quality`.

### Planned split roadmap

The roadmap is empty. Future workspace crates are evaluated case-by-case
against the dependency invariants enforced by `tests/crate_graph.rs`. A
module becomes a crate only when it owns a coherent domain, exports a narrow
API, and removes real compile and dependency weight from `codesight-mcp`.

Modules that should stay in the root crate for now:

- `src/service/**`, `src/mcp.rs`, and `src/cli/**` are orchestration and public-boundary code. They should depend on inner crates, not become them.
- `src/config/**`, `src/security/**`, and `src/server_support/**` are too small and too boundary-oriented to justify workspace overhead.
- `src/indexing/code_index.rs` and watcher/freshness coordination stay in the product crate — it composes storage + analysis + imports through their public facades.
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
When a read tool detects a stale code store, `ensure_repo_store` kicks off
`do_index_folder` on a detached background thread via the
[`CodeService::install_arc_handle`] back-pointer and returns the current
(stale) store immediately — handlers never block on the reindex. Responses
that overlap an in-flight reindex are annotated with `STALE_DATA_WARNING`
(see `src/service/agent_v1/freshness.rs`).

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
`crates/codesight-analysis/src/evidence.rs`.

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
`warn,codesight_mcp=info,codesight_analysis=info,codesight_extractors=info,codesight_framework=info,codesight_imports=info,codesight_retrieval=info,codesight_storage=info`.

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
