# MEGAPLAN.md

**Goal.** Refactor the MCP server into composable crates where the
**code-intelligence backend is a pluggable trait**, so we can add an LSP
backend (rust-analyzer, ts-server, pyright, …) without touching the engine
or the tool surface.

Status: draft v3, 2026-04-19. Breaking changes only — no backcompat.

**Execution log (2026-04-19, session 1):** Phases A–D landed. Workspace now
has `codesight-engine`, `codesight-backend`, `codesight-backend-treesitter`,
and `mcp-rpc` as separate crates. The root `codesight-mcp` crate is the thin
binary wrapper that wires them together. `CodeService` implements
`mcp_rpc::Dispatcher`; the tree-sitter backend is the first `CodeBackend`
implementation and is registered by the binary at startup.

**Execution log (2026-04-19, session 2):** Phase C completion, Phase E
scaffold, Phase G cleanup landed.

- **Phase C gate met.** `codesight-engine` no longer has
  `codesight-extractors` or `tree-sitter` in its normal deps. The
  `CodeBackend` trait grew `detect_language` + `version_stamp`;
  `BackendRouter::indexer()` exposes the tree-sitter impl to engine
  indexing paths. `code_index`, `file_watcher`, `ensure_repo_store`,
  `evaluate_impact::changed_units`, `quality::repo_health`, and
  `util_graph::extract_symbol_names_from_content` all route through the
  trait. Engine tests keep working because `codesight-backend-treesitter`
  is a dev-dep.
- **Phase E scaffold.** `codesight-backend-lsp` crate ships with the
  `CodeBackend` impl, `LspConfig` loader (`.codesight.lsp.toml` shape),
  crash-loop `HealthTracker`, and `LspSession` spawn path. Live request
  pipeline + per-language queries pending the conformance corpus.
- **Phase G.** `tests/crate_graph.rs` extended to enforce the new
  layering (engine forbids `codesight-extractors` and any concrete
  backend). `AGENTS.md` layout table and dependency invariants rewritten.

---

## 0. Scope of this plan

**In scope (now):**
1. Decompose the MCP server so the "engine" is one crate, the "backend" is
   a trait, and there are ≥2 backend implementations (tree-sitter today,
   LSP next).
2. A clean `mcp-rpc` crate so the JSON-RPC boundary is generic.

**Deferred (next plan):**
- Non-Rust FFI exposure.
- Conformance corpus + live LSP request pipeline.
- Third-backend experiments (hybrid / cargo-check type enrichment).

The architectural rule driving the split: **backends are swappable
per-language**. Everything above the backend layer — handlers, tools.json,
dispatch, MCP framing — must not know whether the answer came from
tree-sitter, LSP, or a future source.

---

## 1. North star

```
┌───────────────────────────────────────────────────────────────────────┐
│ L2: protocol                                                          │
│     mcp-rpc: generic JSON-RPC framing (server + client), Dispatcher  │
├───────────────────────────────────────────────────────────────────────┤
│ L1: engine (orchestration, tool handlers, freshness, dispatch)       │
│     codesight-engine                                                 │
│         ↓ calls trait                                                 │
│ L0.5: backend trait                                                   │
│     codesight-backend (CodeBackend trait, fact envelope, capability) │
│         ▲                  ▲                    ▲                    │
│     backend-treesitter   backend-lsp     backend-hybrid (future)     │
├───────────────────────────────────────────────────────────────────────┤
│ L0: primitives + indexer plumbing                                     │
│     codesight-core · codesight-storage · codesight-analysis ·        │
│     codesight-retrieval · codesight-imports · codesight-framework    │
│     (tree-sitter extractors move *under* backend-treesitter)         │
└───────────────────────────────────────────────────────────────────────┘
```

### What changed vs v1

- New **L0.5 backend trait layer** — this is the whole point.
- `codesight-extractors` is no longer a peer of `codesight-storage`; it
  becomes an **implementation detail of `codesight-backend-treesitter`**.
- The engine never depends on tree-sitter or LSP directly. It depends on
  `codesight-backend` (trait) and loads one or more backend impls at
  runtime via a registry.

---

## 2. The backend trait — `codesight-backend`

### 2.1 Design principles

1. **Language-scoped.** A backend declares which languages it can handle;
   the engine's `BackendRouter` picks one per file/query.
2. **Capability-declared, not capability-discovered.** Each backend
   declares a `Capabilities` struct up front: which tool families it can
   serve (`symbols`, `references`, `rename`, `hover`, `call_hierarchy`,
   `impl_resolution`, `diagnostics`). Handlers inspect capabilities,
   never try-and-catch.
3. **Answers carry provenance.** Every fact is tagged with
   `FactQuality::{tree_sitter, lsp_authoritative, heuristic, …}` so
   downstream layers (analysis, quality signal, rename safety) can
   weight them. This is an extension of the existing `FactQuality`
   enum — no parallel taxonomy.
4. **No shared mutable state.** A backend is a `Box<dyn CodeBackend>`
   owning its connections. The engine passes read-only inputs; backends
   return values. Concurrency is the engine's problem.
5. **Index vs query.** Backends implement both a bulk `index()` path (for
   sources that need materialization, like tree-sitter → SQLite) and an
   on-demand `query()` path (for live sources like LSP). A given backend
   may be pure-query (LSP) or pure-index (tree-sitter today) or hybrid.

### 2.2 The trait (as shipped)

```rust
// crates/codesight-backend/src/lib.rs
pub trait CodeBackend: Send + Sync {
    fn name(&self) -> &'static str;                // "tree-sitter", "lsp/rust-analyzer"
    fn capabilities(&self) -> Capabilities;
    fn languages(&self) -> &[Language];            // which langs this backend claims

    fn detect_language(&self, project_root: &Path, path: &Path) -> Option<String>;
    fn version_stamp(&self) -> String;

    // ── Index (optional; LSP impl returns NotSupported) ─────────────
    fn index_file(&self, req: IndexFileRequest) -> Result<IndexedFile, BackendError>;
    fn index_project(&self, req: IndexProjectRequest, sink: &mut dyn IndexSink)
        -> Result<IndexStats, BackendError>;

    // ── Query ───────────────────────────────────────────────────────
    fn symbols_in_file(&self, path: &Path) -> Result<Vec<SymbolRef>, BackendError>;
    fn workspace_symbols(&self, q: &str, scope: Scope)
        -> Result<Vec<SymbolRef>, BackendError>;
    fn references(&self, sym: &SymbolRef) -> Result<Vec<Location>, BackendError>;
    fn definition(&self, loc: &Location) -> Result<Option<Location>, BackendError>;
    fn call_hierarchy(&self, sym: &SymbolRef)
        -> Result<CallHierarchy, BackendError>;
    fn rename_prepare(&self, loc: &Location, new_name: &str)
        -> Result<RenamePlan, BackendError>;
    fn hover(&self, loc: &Location) -> Result<Option<HoverInfo>, BackendError>;

    // ── Lifecycle ───────────────────────────────────────────────────
    fn warm(&self, project_root: &Path) -> Result<(), BackendError> { Ok(()) }
    fn shutdown(&self) -> Result<(), BackendError> { Ok(()) }
}

pub struct Capabilities {
    pub has_index: bool,
    pub has_symbols_in_file: bool,
    pub has_workspace_symbols: bool,
    pub has_references: bool,
    pub has_definition: bool,
    pub has_rename: bool,
    pub has_call_hierarchy: bool,
    pub has_hover: bool,
    pub has_diagnostics: bool,
    pub fact_quality: FactQualityTier,
}
```

### 2.3 Mapping our tools to backend methods

| `tools.json` surface                         | Backend method(s)                                   | Fallback if missing |
|----------------------------------------------|-----------------------------------------------------|----------------------|
| `inspect_structure view=file_outline`        | `symbols_in_file`                                    | tree-sitter scan     |
| `search_catalog unit_kinds=[symbol]`         | `workspace_symbols` + BM25 rerank                    | storage FTS only     |
| `search_catalog unit_kinds=[file]` / regex   | pure storage (not backend)                           | —                    |
| `inspect_graph graph=references`             | `references`                                         | stored refs          |
| `inspect_graph graph=calls`                  | `call_hierarchy`                                     | stored calls         |
| `inspect_graph graph=imports`                | `index_file` (imports are a backend artifact)       | `codesight-imports` fallback |
| `evaluate_impact rename_safety`              | `rename_prepare` + storage scan                      | scan-only            |
| `evaluate_impact blast_radius`               | `references` ⊕ storage                               | storage              |
| `read_content kind=symbol`                   | storage + `symbols_in_file`                          | storage              |
| `evaluate_quality hotspots/dead_code/etc.`   | pure analysis (not backend)                          | —                    |

Tools that aren't in the table (quality signal, repo health, doc coverage,
broken links, change velocity) stay in `codesight-analysis` and don't go
through the backend. They operate on whatever the storage layer already
has.

### 2.4 Backend routing

`codesight-engine::BackendRouter`:
- Maintains a `Vec<Arc<dyn CodeBackend>>` from config.
- For a given `(Language, Capability)`, picks the **highest-`FactQuality`
  backend that declares the capability**. Deterministic tiebreak: registry
  order.
- On `BackendError::NotSupported`, falls through to the next backend.
- Caches per-language decisions for the session.

Example: a Rust workspace with both `tree-sitter` and `lsp/rust-analyzer`
loaded — rename goes through RA (`lsp_authoritative`), file-outline stays
on tree-sitter (faster, good enough, already indexed). That's the payoff.

---

## 3. Crate map (target)

### 3.1 Existing crates (stay; some rename / re-parent)

| Crate                       | Role                                       | Moves under? |
|-----------------------------|--------------------------------------------|--------------|
| `codesight-core`            | primitives, `Symbol`, `FactQuality`         | L0           |
| `codesight-storage`         | SQLite schema, WAL, reads/writes            | L0           |
| `codesight-analysis`        | quality, graph analysis, dead code          | L0           |
| `codesight-retrieval`       | BM25, embedder, sections, fuzzy             | L0           |
| `codesight-imports`         | multilingual import extraction              | L0           |
| `codesight-framework`       | dbt / laravel / nextjs / nuxt providers     | L0           |
| `codesight-extractors`      | **move under backend-treesitter**           | L0 → backend |

### 3.2 New crates

| Crate                                | Layer | Role | Status |
|--------------------------------------|:-----:|------|:------:|
| `codesight-backend`                  | L0.5  | `CodeBackend` trait, `Capabilities`, error types, `BackendRouter` | ✅ |
| `codesight-backend-treesitter`       | L0.5  | Tree-sitter impl of `CodeBackend`. Wraps `codesight-extractors` + writes into `codesight-storage`. | ✅ |
| `codesight-backend-lsp`              | L0.5  | Generic LSP client impl. Config-driven: `{ language, command, args, initialization_options }`. Scaffold only — live request pipeline pending conformance corpus. | 🟡 |
| `codesight-engine`                   | L1    | Extracted from root `src/service/` + `src/indexing/` + `src/server_support/` + `src/security/` + `src/config/`. Owns the `BackendRouter`. | ✅ |
| `mcp-rpc`                            | L2    | stdio JSON-RPC framing, generic `Dispatcher` trait, client *and* server. | ✅ |

### 3.3 Binary

`codesight-mcp` — the stdio MCP server at the repo root. Thin wiring:

```rust
let config = AppConfig::load();
let service = CodeService::with_backends(config, codesight_backend_treesitter::default_router())?;
service.enable_background_watch();
mcp_rpc::run_stdio(Arc::new(service), dispatch_config())
```

### 3.4 Dependency direction (enforced by `tests/crate_graph.rs`)

- L0 imports nothing from L0.5 / L1 / L2.
- `codesight-backend` imports `codesight-core` only. No storage, no
  extractors.
- `codesight-backend-treesitter` imports `codesight-backend`,
  `codesight-extractors`, `codesight-imports`.
- `codesight-backend-lsp` imports `codesight-backend` + `lsp-types` +
  tokio. It does **not** import any other `codesight-*` crate — it
  translates to the shared fact types at the boundary.
- `codesight-engine` imports `codesight-backend`, `codesight-storage`,
  `codesight-analysis`, `codesight-retrieval`, `codesight-framework`,
  `mcp-rpc`. It does **not** depend on any specific backend crate or
  on `codesight-extractors`.
- `mcp-rpc` imports nothing `codesight-*`.
- The binary depends on everything it wires; nothing at L0–L2 depends on
  the binary.

---

## 4. LSP backend (`codesight-backend-lsp`) — design notes

### 4.1 Shape

- Config-driven language table. Each entry: `{ language, command, args,
  initialization_options, file_patterns }`. Default profile ships
  rust-analyzer.
- One `LspSession` per language per project root. Long-lived; warmed on
  first use; closed on `shutdown`.
- Request model: the LSP is strictly **request/response for queries**.
  No document sync — we don't edit files from the server. The agent edits;
  we re-send `didOpen` at query time with the on-disk content.
  (Rationale: the MCP server is read-mostly; keeping doc sync stateful
  is a lot of complexity for "maybe 5% faster queries".)
- Cancellation: every LSP request has a 3s budget; on timeout we return
  `BackendError::Timeout`, the router falls back to tree-sitter.

### 4.2 What LSP gives us that tree-sitter doesn't

- Precise **rename safety** (we know all conflicting shadows).
- Precise **references** across generics / trait impls / macros.
- **Hover** (type info, docs) — new capability for us.
- **Call hierarchy** — currently we approximate.

### 4.3 What tree-sitter keeps owning

- Bulk ingest (LSP is too slow and too heavy for 10k files).
- File outline (LSP's `documentSymbol` is fine but tree-sitter is 10×
  faster once indexed).
- File-text regex search / BM25 (no LSP equivalent).
- The `FactQuality::trusted_structural` tier — LSP only augments.

### 4.4 Why a generic LSP backend (not rust-analyzer-specific)

- One impl covers Rust, TypeScript, Python (pyright), Go (gopls), C++
  (clangd) — with config changes, not code.
- Each language-specific quirk (RA's proc-macro gate, pyright's memory
  budget) lives in the config, not the code.

### 4.5 LSP lifecycle concerns we have to handle

- **Proc macros and `build.rs` execution** — RA runs them. The backend
  must respect a trust-check before starting an LSP session against an
  untrusted path. Reuse the engine's `config/trust.rs`.
- **Slow start.** Warm lazily; tree-sitter answers while LSP is booting.
- **Crash loops.** After 3 consecutive failures in <60s, mark the LSP
  backend "unhealthy" for that project root; fall through permanently
  until next process restart. Implemented in `health::HealthTracker`.
- **Cross-session sharing.** One LSP process per (language, root). Share
  across all codesight sources that resolve to the same root.

---

## 5. Files moved (mechanical map)

### 5.1 Root `src/` → `codesight-engine/src/`

```
src/service/**           → codesight-engine/src/service/**
src/indexing/**          → codesight-engine/src/indexing/**
src/server_support/**    → codesight-engine/src/server_support/**
src/security/**          → codesight-engine/src/security/**
src/config/**            → codesight-engine/src/config/**
src/tools.json           → codesight-engine/src/tools.json  (compile-embedded)
src/mcp.rs               → mcp-rpc/src/server.rs  (generic-ified)
src/cli/**               → binary crate (stays at repo root)
src/main.rs              → binary crate (stays at repo root)
```

### 5.2 `codesight-extractors/` → `codesight-backend-treesitter/`

The crate itself stays at `crates/codesight-extractors/`, but its only
consumer is `codesight-backend-treesitter`. The engine no longer depends
on it; that relationship is enforced by `tests/crate_graph.rs`.

---

## 6. Phases

### Phase A — Preflight ✅ DONE

- `tests/crate_graph.rs` with target invariants.
- Baseline benches on cold-ingest time, incremental reindex, dispatch
  P50/P95, `cargo check` wall time.

### Phase B — Extract `codesight-engine` ✅ DONE

Moved files, no behavior change. `CodeService::new` ingests via the
tree-sitter extractors directly (temporary `#[cfg]` shim — removed in
Phase C).

### Phase C — `codesight-backend` trait + `backend-treesitter` impl ✅ DONE

- Trait sized to what the tool handlers needed (`detect_language`,
  `version_stamp`, `index_file` + query methods).
- `codesight-backend-treesitter` wraps `codesight-extractors`.
- `codesight-engine::indexing` + all stale-check / changed-units
  handlers go through `BackendRouter::indexer()` instead of reaching
  into extractors.
- Gate: `cargo check -p codesight-engine` compiles without depending on
  `codesight-extractors` or `tree-sitter` (transitively). Enforced by
  `tests/crate_graph.rs`.

### Phase D — Extract `mcp-rpc` ✅ DONE

- stdio JSON-RPC framing in its own crate.
- `run_stdio` takes `Arc<dyn Dispatcher>` instead of `Arc<CodeService>`.
- `CodeService` implements `Dispatcher`.
- Ships a minimal `McpClient` too, so any future embedder doesn't have
  to re-invent framing.

### Phase E — Add `codesight-backend-lsp` 🟡 SCAFFOLD LANDED

Shipped:
- `CodeBackend` impl with capability set claiming refs / rename /
  hover / call hierarchy / definition / workspace_symbols.
- `LspConfig` loader (`.codesight.lsp.toml`) with a bundled
  rust-analyzer default profile.
- `HealthTracker` (crash-loop gate per §4.5).
- `LspSession` spawn path (subprocess with stdin/stdout piped).

Still to land:
- Actual LSP request/response pipeline (JSON-RPC over the pipes,
  outstanding-request map, cancellation + timeout).
- Translate LSP types to our fact envelope.
- Router integration test `tests/backend_lsp_rust.rs` — rust-file
  references go through LSP, non-rust files stay on tree-sitter, LSP
  absent falls back.
- Trust gate + conformance corpus (curated Rust files both backends
  must agree on).

### Phase G — Cleanup ✅ DONE

- Dead shims removed. `src/lib.rs` skeletal.
- `AGENTS.md` layout + layering invariants rewritten.
- `tests/crate_graph.rs` enforces Phase C gate.

---

## 7. Risks + mitigations

| Risk                                                                 | Lik. | Mitigation |
|----------------------------------------------------------------------|:----:|------------|
| `CodeBackend` trait design churns during LSP implementation          |  H   | Trait is narrow today. Add methods only when a tool needs one. |
| LSP startup latency kills dispatch P50                               |  M   | Backend router falls through to tree-sitter on LSP cold/timeout; warm in a background task. |
| Proc-macro execution via RA against an untrusted repo                |  H   | Hard trust gate before any LSP session starts. Default deny. |
| Handlers leak tree-sitter types into engine (hidden L0.5 breach)     |  M   | `tests/crate_graph.rs` + CI fail. |
| Two backends diverge on subtle semantics (e.g. what "reference" means)|  M   | Shared conformance tests in `tests/backend_conformance/` — both backends must pass. |
| LSP config file sprawl                                               |  M   | Ship one profile (rust-analyzer). Others are user-config only until there's demand. |

---

## 8. Success criteria

The reorg is "done" when **all** of the following hold:

1. `cargo nextest run` green on the new layout.
2. `tests/crate_graph.rs` enforces L0 → L2 invariants. ✅
3. **Two backends compile and ship**: `codesight-backend-treesitter`
   always; `codesight-backend-lsp` with a rust-analyzer profile, opt-in
   via config.
4. On a Rust file, `evaluate_impact rename_safety` answers via LSP with
   `_meta.source = "lsp/rust-analyzer"`. On the same file with LSP
   disabled, it answers via tree-sitter with
   `_meta.source = "tree_sitter"`.
5. A new backend can be added by implementing `CodeBackend` in a new
   crate and adding one entry in the engine's `BackendRegistry` config.
   No changes to `tools.json`, no changes to handlers.
6. MCP server P50 dispatch latency within 10% of the pre-reorg baseline.
7. `AGENTS.md` reflects the new shape. ✅

If we can't hit (3)+(4), the backend trait is wrong — stop, redesign.

---

## 9. Open questions (deliberately unresolved)

- **Where does LSP config live?** A top-level `codesight.lsp.toml`? Part
  of `.codesight.toml`? Per-language merged from `$XDG_CONFIG_HOME`?
  Likely: inline in `.codesight.toml` with a user-level override file.
- **Is `codesight-imports` redundant once LSP is available?** LSP gives
  imports per-file via `documentSymbol` + language-specific quirks. We
  probably keep `codesight-imports` as the tree-sitter-path fallback;
  unify later if a pattern emerges.
- **Conformance test corpus size.** A dozen curated Rust files? A
  whole external repo submoduled? Start with curated; add a real repo
  once cases outgrow synthetic.
- **Hybrid backend** (tree-sitter index + LSP augment, single crate) —
  versus the current router-of-two design. Router wins on composability;
  hybrid wins on latency. Router first; hybrid only if measurements
  show routing overhead matters.

---

*This revision narrows the focus to the engine/backend workstream: a
pluggable backend trait at the heart, and a first LSP backend behind it.*
