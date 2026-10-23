# Architecture

How `symslice-mcp` is put together. Read this before changing crate
boundaries, the data flow, or the MCP surface.

## Crates

```
symslice-mcp/                     root crate: CLI (clap) + rmcp stdio server
└── crates/
    ├── symslice-core/            shared primitives: Symbol, FactQuality, Confidence, LanguageId
    ├── symslice-storage/         SQLite schema, forward-only migrations, reads/writes
    ├── symslice-language-support/ one folder per language: grammar, tags.scm, extraction,
    │                               imports, packages, entrypoints, complexity, frameworks
    ├── symslice-retrieval/       ranking, fusion, fuzzy matching, doc sections, embedder
    ├── symslice-analysis/        graph, impact, quality, dead code, agent readiness, test gaps
    └── symslice-engine/          indexing orchestration, tool dispatch, tools.json, config
```

Workspace dependencies (from the crates' `Cargo.toml`):

| Crate | Depends on |
|---|---|
| `symslice-core` | — |
| `symslice-storage` | core |
| `symslice-language-support` | core |
| `symslice-retrieval` | core, storage |
| `symslice-analysis` | core, storage, language-support |
| `symslice-engine` | core, storage, language-support, retrieval, analysis |
| `symslice-mcp` (root) | all of the above |

`tests/crate_graph.rs` enforces these invariants:

- `symslice-language-support` must not depend on analysis, engine,
  retrieval, or storage.
- The engine must depend on storage, analysis, retrieval, and
  language-support.
- No crate depends on the root `symslice-mcp` crate.
- Analysis and engine code must not branch on language-name literals or file
  extensions. Per-language behaviour belongs behind the `Language` trait.

Adding a language means adding a folder under
`crates/symslice-language-support/src/languages/<lang>/`, not a new crate.

## Data flow

```
MCP client ── JSON-RPC over stdio (rmcp) ──▶ src/main.rs
                                               │
                         symslice-engine::service::CodeService::dispatch
                         (coerce + validate args against tools.json)
                                               │
              ┌────────────────────────────────┼─────────────────────────────┐
     indexing::code_index              agent_v1 adapters (cutplan.rs)    analysis / retrieval
     crawl → delta → extract           search outline read references   graph, impact, quality,
              │                        impact health                     ranking, embeddings
     symslice-language-support                   │
     tree-sitter parse → tags.scm                 │
              │                                   │
              └──────────────▶ symslice-storage (SQLite, WAL) ◀────────┘
```

- The six public tools are thin adapters in
  `crates/symslice-engine/src/service/agent_v1/cutplan.rs`. They resolve the
  source from the process working directory, translate arguments, call the
  internal handlers (`search_catalog`, `read_content`, `inspect_graph`, …),
  and freeze the response envelope. The internal handlers are not exposed to
  MCP clients.
- `tools.json` is embedded at compile time. Each `inputSchema` is compiled
  once into a JSON-schema validator, and dispatch coerces stringified scalars
  before validating (`service/agent_v1/validate.rs`).
- Each tool has a wall-clock budget (`service/agent_v1/registry.rs`): `read`
  3 s, `search`/`outline`/`references` 5 s, `impact`/`health` 15 s, ingest
  60 s. Exceeding it returns a `TIMEOUT` error.
- When a read detects a stale store, `ensure_repo_store` starts a background
  reindex and returns the current store immediately. Responses that overlap
  an in-flight reindex carry `STALE_DATA_WARNING`.

## Trust labels

There are two levels of labelling:

- **Facts.** `FactQuality` (`symslice-core`) tags extracted facts as
  `trusted_structural` (curated query pack), `baseline_structural` (generic
  AST walker or profiled alias), or `heuristic` (manual, regex, or text
  fallback).
- **Responses.** `_meta.confidence` is one of `safe`, `partial`, `heuristic`
  (the `Confidence` enum in `symslice-core`), or `unreliable` (set by the
  `impact` adapter when only text mentions exist). The README describes
  what each value promises to callers.

## Storage

Every connection goes through `symslice_storage::db::open_rw` / `open_ro`:
WAL journal, `synchronous=NORMAL`, `temp_store=MEMORY`, 256 MiB mmap, 64 MiB
cache, `foreign_keys=ON`, 5 s busy timeout. The code-index schema migrates
forward only (`crates/symslice-storage/src/storage/migrations.rs`, target
`CODE_INDEX_TARGET_USER_VERSION`).

## Observability

Logging uses `tracing` and writes to stderr only, so stdout carries nothing
but JSON-RPC. `--log-format json|pretty` or `SYMSLICE_LOG_FORMAT` selects
the format, and `RUST_LOG` filters it.
