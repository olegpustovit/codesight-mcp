# ARCHITECTURE.md

A running map of how `codesight-mcp` is put together. Read this before touching
crate boundaries, data flow, or the MCP surface. For configuration knobs, see
[`CONFIGURATION.md`](CONFIGURATION.md). For active work and open issues see
[`REFACTOR_WORKLOG.md`](REFACTOR_WORKLOG.md).

## Crate layout

```
codesight-mcp/                          ← product crate (root Cargo.toml)
├── src/                                ← thin CLI entry + /stdio adapter (wires engine)
└── crates/
    ├── codesight-core/                 ← L0 primitives (symbols, fact quality, LanguageId)
    ├── codesight-storage/              ← L0 SQLite schema + migrations + reads/writes
    ├── codesight-retrieval/            ← L0 ranking, summarization, section parsing, embedder
    ├── codesight-language-support/     ← L0.75 one folder per language owns identity,
    │                                      SCM, extraction, imports, packages, liveness,
    │                                      complexity, frameworks, host profiles
    ├── codesight-analysis/             ← graph, quality, dead-code, readiness, evidence
    ├── codesight-backend/              ← L0.5 CodeBackend trait + tree-sitter impl
    ├── codesight-engine/               ← L1 MCP tool dispatch + indexing orchestration
    └── mcp-rpc/                        ← L2 JSON-RPC stdio framing
```

**Dependency direction is inward.** `codesight-core` is the base; every other
crate depends on it. `codesight-language-support` owns every per-language
heuristic (symbol extraction, imports, packages, entrypoints, complexity,
frameworks) behind the `Language` trait — a single default-bodied trait with
opt-in overrides. `codesight-backend` depends on the trait + core, exposing
tree-sitter via the `treesitter` feature. `codesight-analysis` and
`codesight-retrieval` depend on `core + storage`. `codesight-engine` depends
on everything below L1 and never on a concrete backend. `mcp-rpc` is a leaf
protocol crate — zero `codesight_*` deps.

### Workspace dependency graph (current snapshot)

| From → To                      | core | storage | language-support | retrieval | analysis | backend | engine | mcp-rpc |
|--------------------------------|:----:|:-------:|:----------------:|:---------:|:--------:|:-------:|:------:|:-------:|
| `codesight-core`               |  —   |    ✗    |        ✗         |     ✗     |    ✗     |    ✗    |   ✗    |    ✗    |
| `codesight-storage`            |  ✓   |    —    |        ✗         |     ✗     |    ✗     |    ✗    |   ✗    |    ✗    |
| `codesight-language-support`   |  ✓   |    ✗    |        —         |     ✗     |    ✗     |    ✗    |   ✗    |    ✗    |
| `codesight-retrieval`          |  ✓   |    ✓    |        ✗         |     —     |    ✗     |    ✗    |   ✗    |    ✗    |
| `codesight-analysis`           |  ✓   |    ✓    |        ✓         |     ✗     |    —     |    ✗    |   ✗    |    ✗    |
| `codesight-backend`            |  ✓   |    ✗    |        ✓         |     ✗     |    ✗     |    —    |   ✗    |    ✗    |
| `codesight-engine`             |  ✓   |    ✓    |        ✓         |     ✓     |    ✓     |    ✓    |   —    |    ✓    |
| `mcp-rpc`                      |  ✗   |    ✗    |        ✗         |     ✗     |    ✗     |    ✗    |   ✗    |    —    |
| `codesight-mcp` (root)         |  ✓   |    ✗    |        ✗         |     ✗     |    ✗     |    ✓    |   ✓    |    ✓    |

Rules enforced by `tests/crate_graph.rs`:

- `codesight-core` and `mcp-rpc` are leaves.
- `codesight-language-support` may depend only on `-core` — no upward reach
  into backend, engine, analysis, retrieval, or storage.
- `codesight-backend` depends on `-core` + `-language-support` only.
- `codesight-engine` pulls no concrete backend types through its lib.
- Root `codesight-mcp` depends on engine + backend + mcp-rpc only.

### Why this split

- `codesight-language-support` is the single touch-point for adding or
  modifying a language. One folder (`src/languages/<lang>/`) owns the whole
  lifecycle — identity, grammar, SCM, extraction, imports, package
  discovery, entrypoints, complexity, frameworks. The `Language` trait is
  fully default-bodied so a minimal lang is a struct + `id()` + `manifest()`.
- `codesight-core` holds only primitives that are legitimately shared: the
  `Symbol` / `FactQuality` / `LanguageId` types and small serde DTOs. It
  stays small and stable.
- `codesight-engine` is the orchestration layer — indexing, MCP dispatch,
  config/security/server-support, agent-v1 handlers. Backend-agnostic via
  the `CodeBackend` trait.

### Shipped consolidation (2026-04-21)

- `codesight-language-support` — absorbed the former
  `codesight-extractors`, `codesight-framework`, and `codesight-imports`
  crates. One folder per language under `src/languages/<lang>/` owns the
  grammar handle, SCM (embedded via `include_str!`), symbol extraction,
  import extraction + resolution, package discovery, entrypoints, test
  detection, complexity, and framework providers. Host profiles (openapi,
  ansible) are sibling `Language` impls reusing the host grammar. Depends
  only on `codesight-core`.
- `codesight-backend` — holds `CodeBackend` trait + optional tree-sitter
  impl (Cargo `treesitter` feature). Dispatches via the `Language` trait.
- `codesight-retrieval` — ranking, section parsing, fusion/RRF, fuzzy
  matching, summarizer config, dense embedder. Depends on core + storage.
- `codesight-analysis` — graph (calls, coupling, cycles, layers, pagerank),
  quality, dead-code, agent readiness, evidence, impact, test gaps.
  Depends on core + storage + language-support.

### Planned split roadmap

The roadmap is empty. Future workspace crates are evaluated case-by-case
against the dependency invariants enforced by `tests/crate_graph.rs`. A
module becomes a crate only when it owns a coherent domain, exports a narrow
API, and removes real compile and dependency weight from the product.

- Per-language logic always lives under
  `crates/codesight-language-support/src/languages/<lang>/`. A new lang
  never spawns a new crate — it's a folder.
- `codesight-core` stays deliberately small. It is not the place to park
  every type that happens to cross a crate boundary.

## Data flow: `ingest_sources` → retrieval

```
MCP client
  │  JSON-RPC over stdio (mcp-rpc)
  ▼
src/ (thin adapter) → codesight-engine::service
  │  dispatch via agent-v1 + tools.json (coerce → validate)
  ▼
codesight-engine::indexing::code_index
  │  crawl, delta, symbol extraction
  ▼
codesight-backend (treesitter feature)
  │  tree-sitter parse → SCM extraction
  ▼
codesight-language-support
  │  Language::extract_symbols, extract_imports,
  │  package discovery, entrypoint/liveness metadata
  ▼
codesight-storage
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
incompatible tree-sitter release. Each language's `Language::extract_symbols`
override lives under `crates/codesight-language-support/src/languages/<lang>/`:

- `vue` — waiting on a `tree-sitter-vue` port on tree-sitter ≥ 0.26.
- `razor` — same situation.
- `blade` — same situation.

Everything else trends toward structural extraction via the default SCM
pipeline (`grammar_spec() → tags.scm`) or the generic AST fallback. A
bespoke extractor requires stating why the curated query pack is
insufficient.

## Observability

All logs go through `tracing`; library code contains no bare `eprintln!` or
`println!`. The subscriber is initialized in `src/main.rs` to write to
**stderr** only so JSON-RPC on stdout stays clean. Log format is selectable
via `--log-format json|pretty` or `CODESIGHT_LOG_FORMAT`. Filtering uses
`RUST_LOG` with the standard `EnvFilter` syntax; the default fallback is
`warn,codesight_mcp=info,codesight_engine=info,codesight_analysis=info,codesight_backend=info,codesight_language_support=info,codesight_retrieval=info,codesight_storage=info`.

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
