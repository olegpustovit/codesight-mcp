# ARCHITECTURE.md

A running map of how `codesight-mcp` is put together. Read this before touching
crate boundaries, data flow, or the MCP surface. For configuration knobs, see
[`CONFIGURATION.md`](CONFIGURATION.md). For the active refactor program see
[`REFACTOR_PLAN.md`](REFACTOR_PLAN.md).

## Crate layout

```
codesight-mcp/                          ← product crate (root Cargo.toml)
├── src/
│   ├── cli/                            ← operator subcommands (`check`)
│   ├── config/                         ← layered AppConfig + env + trust
│   ├── service/                        ← MCP boundary + agent-v1 dispatch
│   ├── indexing/                       ← crawl, delta, imports, packages
│   ├── analysis/                       ← graph, quality, cross-repo, evidence
│   ├── framework/                      ← dbt / laravel / nextjs / nuxt detection
│   ├── retrieval/                      ← ranking, summarization, sections
│   ├── server_support/                 ← repo-id, path map, reindex state
│   ├── security/                       ← binary policy, secret policy, paths
│   └── tools.json                      ← frozen MCP contract
└── crates/
    ├── codesight-core/                 ← shared primitives (symbols, fact quality)
    ├── codesight-extractors/           ← SCM engine + curated query packs
    └── codesight-storage/              ← SQLite schema + migrations + reads/writes
```

**Dependency direction is inward.** `codesight-core` is the base; the
`extractors` and `storage` crates depend on it; the product crate depends on
all three. Nothing in the crates depends on the product layer.

### Workspace dependency graph (authoritative edge list)

Extracted from `cargo metadata --format-version 1 --no-deps` on 2026-04-10;
regenerate whenever a workspace crate is added or an edge changes.

| From → To             | `codesight-core` | `codesight-extractors` | `codesight-storage` | `codesight-mcp` |
|-----------------------|:----------------:|:----------------------:|:-------------------:|:---------------:|
| `codesight-core`      |        —         |           ✗            |          ✗          |        ✗        |
| `codesight-extractors`|        ✓         |           —            |          ✗          |        ✗        |
| `codesight-storage`   |        ✓         |           ✗            |          —          |        ✗        |
| `codesight-mcp`       |        ✓         |           ✓            |          ✓          |        —        |

Rules enforced by this table:

- `codesight-core` is a leaf — it depends on no other workspace crate.
- `codesight-extractors` and `codesight-storage` do not depend on each other
  (no horizontal edges between the two middle crates).
- `codesight-mcp` is the only consumer that pulls in all three inner crates.
- Any new crate member or new edge requires updating this table **and** the
  S2 workstream entry in [`REFACTOR_PLAN.md`](REFACTOR_PLAN.md) in the same
  patch (freeze guard, not just documentation).


### Why this split

- `codesight-storage` and `codesight-extractors` are the two pieces most likely
  to get heavier over time (new languages, new query packs, schema evolution).
  Isolating them shortens the compile loop for the rest of the product.
- `codesight-core` holds only primitives that are legitimately shared: the
  `Symbol` / `FactQuality` taxonomy, small serde DTOs, the error marker traits.
  It is deliberately small so it stays stable.
- Everything else — ingest orchestration, MCP dispatch, framework detection,
  retrieval, analysis — lives in the product crate. A Phase-2 split (`indexing`
  / `analysis` crates) is deferred until shared fact types settle; see
  `REFACTOR_PLAN.md` "Intentionally out of scope".

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
`src/analysis/evidence.rs` and `REFACTOR_PLAN.md` WS12.

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
