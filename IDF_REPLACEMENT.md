# IDF_REPLACEMENT.md

Replace the hand-rolled TF-IDF "semantic" path with dense static embeddings
backed by `model2vec-rs` and `sqlite-vec`, while keeping the public MCP
surface smaller than it is today.

This is not a 1:1 port of the old feature. It is a cleanup and replacement:

- delete TF-IDF
- delete provider/env-var plumbing
- delete hidden embedding tools and knobs
- keep one simple semantic switch in the public API
- make `query_mode=natural_language` actually useful

The implementation must respect the real storage layout in this repo:

- code index DB: `repos/<source>/index.sqlite3`
- docs index DB: `docs/<source>/doc_index.sqlite3`
- no fake "single DB per workspace" story

We ship code and docs semantic retrieval in the same overall feature, but the
work is split into separate workstreams so the implementation is reviewable and
the ownership boundaries stay clean.

---

## 1. Frozen decisions

These are now fixed.

### 1.1 Product behavior

- `query_mode=natural_language` implies semantic retrieval.
- `semantic=true` remains as the explicit expert override for non-natural
  queries.
- If semantic retrieval is unavailable at request time, the request falls back
  to lexical retrieval and returns a warning. It must not hard-fail.
- `semantic_only` is deleted.
- `semantic_weight` is deleted.
- Hidden `embed_repo` is deleted.
- Provider selection via `CODESIGHT_EMBED_MODEL`, `OPENAI_*`, and `GOOGLE_*`
  is deleted.

### 1.2 Architecture

- Code and docs semantic retrieval ship together, in the same branch, but as
  separate implementation workstreams.
- The root crate owns:
  - model loading
  - chunk-text assembly
  - batching
  - fallback/warning policy
  - MCP orchestration
- `codesight-storage` owns:
  - SQLite DDL
  - vec table helpers
  - vector insert/delete/query helpers
  - no model code

### 1.3 Storage strategy

- Vector rows are keyed by the existing integer row IDs in the code and docs
  stores, not by stable string IDs.
- Stable string IDs remain the product-level identity.
- Embedding payloads are computed in the root crate and passed into storage
  keyed by stable ID; storage resolves those stable IDs to row IDs inside the
  write transaction before inserting vec rows.
- We keep one vec table per domain store:
  - code DB: `vec_symbol_embeddings`
  - docs DB: `vec_section_embeddings`

### 1.4 Upgrade strategy

- No in-place backfill.
- No compatibility mode for old TF-IDF artifacts.
- No "migrate old index and keep serving" path.
- Existing incompatible indexes are invalidated and reingested.
- Incremental ingest remains the single unified update path: when a file or doc
  changes, lexical and vector indexes update together.

---

## 2. Why the current path must go

The current TF-IDF path is the wrong abstraction and the wrong maintenance
burden.

- It duplicates lexical ranking that FTS5/BM25 already provides.
- It is not actually semantic.
- It stores sidecar state in `embeddings.json`.
- It relies on provider/env detection that is mostly aspirational.
- It creates runtime/spec drift because hidden semantic behavior exists outside
  `src/tools.json`.

Concretely, the old path spans:

- `src/retrieval/semantic.rs`
- `src/service/agent_v1/catalog_search/semantic.rs`
- hidden `embed_repo` dispatch in `src/service/agent_v1/registry.rs`
- TF-IDF management logic in `src/service/agent_v1/index_admin.rs`
- undocumented semantic args in `src/service/agent_v1/args.rs`

The replacement should reduce code and reduce decision surface at the same
time.

---

## 3. Chosen stack

### 3.1 Runtime components

| Layer | Choice | Notes |
|---|---|---|
| Dense encoder | `model2vec-rs` | Pure Rust static embeddings |
| Vector index | `sqlite-vec` | Lives in the same SQLite DB as each domain store |
| Lexical retrieval | existing FTS5/BM25 | Unchanged |
| Fusion | RRF | No score-scale tuning |

### 3.2 Model policy

Start with `potion-base-8M` for the compatibility spike. Do not hard-freeze the
final bundled model until M0 proves:

- the current Rust API is workable in this codebase
- binary size is acceptable
- query quality on this repo is good enough

If M0 fails on size or quality, we can still switch models without changing the
rest of the architecture.

### 3.3 Why row IDs are the right vec keys

Both stores already use integer row IDs as the FTS content row ID:

- code symbols: `symbols.row_id`
- doc sections: `sections.row_id`

That makes row IDs the best join key for dense retrieval too:

- aligns with the existing SQLite shape
- matches sqlite-vec examples and hot-path usage
- keeps vector joins integer-based
- avoids making vec tables carry text primary keys in the hot path

Stable IDs still matter, but only at the product layer. Storage can resolve
stable ID -> row ID inside the transaction and then write the vec rows.

---

## 4. Public MCP contract after the change

Keep the interface simple.

### 4.1 `search_catalog`

Keep:

- `semantic: boolean`
- `query_mode=keyword|natural_language|regex`

Delete:

- `semantic_only`
- `semantic_weight`

Behavior:

- `query_mode=natural_language` behaves as if `semantic=true`.
- `semantic=true` means "augment lexical search with dense retrieval where
  available."
- If dense retrieval is unavailable, the response still succeeds and includes a
  warning such as:

```json
{
  "code": "SEMANTIC_FALLBACK",
  "message": "Semantic retrieval unavailable; returning lexical results only.",
  "detail": {
    "reason": "model_load_failed | vectors_missing | index_rebuild_required"
  }
}
```

### 4.2 `manage_indexes`

Do not add new actions.

Keep:

- `invalidate`
- `delete`
- `rebuild`
- `status`
- `snapshot_quality`

Delete hidden embedding lifecycle entirely. `ingest_sources` is the only write
path that updates lexical and vector indexes.

### 4.3 Hidden/internal cleanup

Delete:

- `embed_repo` from the registry
- TF-IDF-only tests
- TF-IDF-only env vars
- TF-IDF-only config paths

The public surface should end smaller than what exists today, not larger.

---

## 5. Ownership split

This is the boundary that keeps the plan implementable.

### 5.1 Root crate responsibilities

New root-crate modules:

- `src/retrieval/embedder.rs`
- `src/retrieval/fusion.rs`

Root crate responsibilities:

- load the embedder once in `CodeService`
- assemble chunk text for code symbols and doc sections
- batch-encode updated entities
- decide whether to fall back and attach warnings
- orchestrate per-domain search and merge

### 5.2 Storage crate responsibilities

New or extended storage-side code:

- `crates/codesight-storage/src/db.rs`
- `crates/codesight-storage/src/storage/schema.rs`
- `crates/codesight-storage/src/storage/doc_schema.rs`
- `crates/codesight-storage/src/storage/vector_index.rs` or equivalent storage
  helpers

Storage responsibilities:

- ensure sqlite-vec is registered on open
- create vec tables for fresh stores
- resolve stable IDs to row IDs during writes
- insert/delete vec rows transactionally
- execute KNN queries and return row IDs

Storage does not know how to build embeddings.

---

## 6. Actual storage plan

### 6.1 Code store

Fresh code-store DDL grows one vec table:

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS vec_symbol_embeddings USING vec0(
    symbol_rowid INTEGER PRIMARY KEY,
    embedding FLOAT[DIM]
);
```

Join-back path:

- KNN query returns `symbol_rowid`
- storage joins `symbols.row_id = symbol_rowid`
- root crate turns those rows into MCP items

### 6.2 Docs store

Fresh docs-store DDL grows one vec table:

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS vec_section_embeddings USING vec0(
    section_rowid INTEGER PRIMARY KEY,
    embedding FLOAT[DIM]
);
```

Join-back path:

- KNN query returns `section_rowid`
- storage joins `sections.row_id = section_rowid`

### 6.3 Existing incompatible indexes

We do not mutate old indexes into the new shape.

Instead:

- detect schema/version mismatch
- mark the index invalid
- wipe code + docs DBs for that source on rebuild
- reingest from source using the unified lexical+vector path

This is simpler and safer than trying to backfill old stores.

---

## 7. Chunk text policy

The chunk-text policy belongs in the root crate, not storage.

### 7.1 Code symbols

Embed this shape:

```text
{kind} {name}
{signature}
{summary}
{source_prefix}
```

Rules:

- `summary` comes from stored summary/doc-comment text when present
- `source_prefix` is a bounded prefix of the symbol source
- do not pad with file paths or synthetic metadata
- keep the text short enough for fast incremental embedding

Initial limits:

- functions/methods: first 512 chars of source
- types/traits/structs/enums: first 256 chars of source

### 7.2 Doc sections

Embed this shape:

```text
{heading}
{heading_path}
{summary}
{content_prefix}
```

Rules:

- `content_prefix` is a bounded prefix of section text
- do not dump whole documents into embeddings

Initial limit:

- docs sections: first 512 chars of content

These limits are deliberately small to protect incremental performance.

---

## 8. Query behavior

### 8.1 Domain behavior

Dense retrieval applies only to units that have embeddings:

- code symbols
- doc sections

File-text hits remain lexical-only.

That means:

- `unit_kinds=["file"]` stays pure lexical
- `unit_kinds=["symbol"]` can use lexical + dense
- `unit_kinds=["section"]` can use lexical + dense
- `domain=hybrid` runs both domain lanes, then merges results

### 8.2 Fusion behavior

Use Reciprocal Rank Fusion.

Do not keep weighted-sum mode.

RRF lanes:

- code symbols: `RRF(symbol_bm25, symbol_knn)`
- doc sections: `RRF(section_bm25, section_knn)`

Then merge the already-fused code and docs lanes using the existing domain cap
behavior.

File-text hits are appended from the lexical lane only and are never fed into
vector fusion.

### 8.3 Fallback behavior

Semantic requests fall back to lexical search with warnings when:

- the model failed to load
- vec tables are missing
- the index is invalid and rebuild is required
- the current request only targets file hits

Fallback is response-success, not response-error.

### 8.4 Natural-language honesty

The current code makes `natural_language` mean "default to hybrid domain."
That is not enough.

After this refactor:

- `query_mode=natural_language` implies semantic search
- if semantic is available, use it
- if semantic is unavailable, warn and fall back

That makes the mode name honest without making the system brittle.

---

## 9. Unified ingest and invalidation

This is the most important implementation rule.

There is one write path:

- `ingest_sources`

Every update must keep lexical and vector state in sync.

### 9.1 Full ingest

For a fresh or invalidated source:

1. index code into the code DB
2. index docs into the docs DB
3. batch-embed code symbols and write code vec rows
4. batch-embed doc sections and write docs vec rows
5. mark both stores fresh together

### 9.2 Incremental ingest

For a touched file or doc:

1. re-extract the updated units
2. replace lexical rows
3. delete vec rows for removed units
4. embed only the updated units
5. write new vec rows in the same operation

There is no detached embedding phase.

### 9.3 Invalid index handling

If a source is opened and one of the domain stores is incompatible:

- treat the source as needing rebuild
- do not partially serve stale semantic state as if it were current
- `search_catalog` falls back with warning until reingest happens
- `ingest_sources` rebuilds the invalidated stores

Code and docs semantic support ship together, so invalidation logic must think
in terms of the source, not just a single DB file.

---

## 10. Workstreams

Both code and docs ship together, but the work is decomposed so the branch can
land as a sequence of clean commits.

### WS0 - Compatibility spike

Goal: prove the dependencies are workable before restructuring the code.

Deliverables:

- add `model2vec-rs`
- add `sqlite-vec`
- register sqlite-vec from `crates/codesight-storage/src/db.rs`
- prove one local KNN query against a temp SQLite DB
- prove one local embedding call from the current Rust API
- record binary size and a first query latency number

Gate:

- if this fails, stop and revise stack choice before touching the search path

### WS1 - Root embedder and fusion

Goal: establish the root-crate orchestration layer.

Deliverables:

- `src/retrieval/embedder.rs`
- `src/retrieval/fusion.rs`
- `CodeService` owns a singleton embedder
- helper APIs for chunk assembly and batched encode

Gate:

- no storage writes yet

### WS2 - Code-store vector path

Goal: make code semantic retrieval real.

Deliverables:

- extend code-store DDL with `vec_symbol_embeddings`
- add storage helpers:
  - delete symbol vec rows for removed symbols
  - resolve stable IDs to `symbols.row_id`
  - insert symbol vec rows
  - query symbol KNN
- extend code ingest path so updated symbols write lexical + vector state

Gate:

- code symbols work end to end before docs land

### WS3 - Docs-store vector path

Goal: ship docs semantic retrieval in the same feature without entangling it
with the code store.

Deliverables:

- extend docs-store DDL with `vec_section_embeddings`
- add storage helpers mirroring the code path
- extend docs ingest so updated sections write lexical + vector state

Gate:

- docs sections work end to end before hybrid query integration

### WS4 - Unified invalidation and rebuild behavior

Goal: remove compatibility ambiguity.

Deliverables:

- detect incompatible code/docs stores
- mark the source as rebuild-required
- ensure `search_catalog` falls back with warning during that window
- ensure `ingest_sources` rebuilds lexical + vector state together

Gate:

- no partial semantic serving on incompatible indexes

### WS5 - Query integration and public API cleanup

Goal: simplify the MCP surface while wiring in the new retrieval path.

Deliverables:

- integrate symbol and section KNN into `search_catalog`
- `query_mode=natural_language` implies semantic
- semantic fallback warnings
- delete `semantic_only`
- delete `semantic_weight`
- delete hidden `embed_repo`
- update `src/tools.json`
- update Rust arg structs to match `tools.json`

Gate:

- runtime and schema must match exactly

### WS6 - Test and performance hardening

Goal: keep the feature honest.

Deliverables:

- targeted integration tests for code semantic retrieval
- targeted integration tests for docs semantic retrieval
- fallback warning tests
- invalid-index/rebuild tests
- incremental update tests for vector delete+insert
- behavioral golden queries
- measured ingest and query timings

Gate:

- no merge without passing the repo performance policy

---

## 11. Tests we actually need

Do not replace everything with only goldens.

We need all of:

### 11.1 Compatibility tests

- sqlite-vec registration works in this build
- embedder loads and returns a stable dimension

### 11.2 Storage tests

- code vec rows insert/delete/update correctly
- docs vec rows insert/delete/update correctly
- KNN returns joinable row IDs

### 11.3 Ingest tests

- full ingest populates lexical and vector indexes
- incremental code update updates both
- incremental docs update updates both
- removed symbols/sections delete vec rows

### 11.4 Query behavior tests

- `natural_language` uses semantic retrieval when available
- semantic requests fall back with warning when unavailable
- file-only searches remain lexical-only
- hybrid code+docs retrieval returns both domains correctly

### 11.5 Contract tests

- `tools.json` matches Rust args
- deleted knobs are rejected/absent
- hidden `embed_repo` is gone

### 11.6 Behavioral goldens

Keep a small golden set, but use it as one layer, not the only layer.

---

## 12. Performance policy

Do not relax AGENTS.md to make the plan easier.

Targets remain:

- cached reads: `< 50 ms`
- cold reads on medium repo: `< 200 ms`
- incremental reindex after single-file edit: `< 100 ms`
- cold full ingest of this repo: `< 5 s`

Implications:

- embed only changed symbols/sections on incremental ingest
- batch encode updates
- keep chunk text small
- do not add a global re-embed step

If eager embedding threatens these numbers, fix the implementation. Do not
change the target in the plan.

---

## 13. Code deletion target

This refactor should still end net-negative in code.

Expected deletions:

- `src/retrieval/semantic.rs`
- TF-IDF-only logic in `src/service/agent_v1/catalog_search/semantic.rs`
- hidden `embed_repo` path in `src/service/agent_v1/index_admin.rs`
- semantic provider detection and env-var plumbing
- `AppConfig::embedding_path`
- TF-IDF-only tests

Expected additions:

- root embedder + fusion modules
- storage vec helpers
- targeted semantic integration tests

Net goal:

- fewer moving parts
- fewer runtime modes
- no sidecar embedding JSON

---

## 14. Done criteria

This replacement is done only when all of these are true:

1. TF-IDF code and provider plumbing are deleted.
2. `semantic_only`, `semantic_weight`, and hidden `embed_repo` are gone.
3. `query_mode=natural_language` performs semantic retrieval when available.
4. Semantic requests fall back to lexical results with warnings when
   unavailable.
5. Code symbol embeddings and doc section embeddings both ship in the same
   feature.
6. Incremental ingest updates lexical and vector indexes together.
7. Incompatible old indexes are invalidated and rebuilt rather than migrated
   in place.
8. `src/tools.json` and Rust args match exactly.
9. Targeted tests cover storage, ingest, query behavior, contract cleanup, and
   goldens.
10. The AGENTS performance targets are met without rewriting the policy.

If any of these are false, the branch is not ready.

---

## 15. Implementation appendix — dogfooded via codesight-mcp

Every target below was verified by querying the live codesight index of this
repo (`OlegHQ/codesight-mcp`, 413 files, 4143 symbols). Line numbers are
current as of commit `e9f8ea9`.

### 15.1 Exact delete list (WS5)

Full module deletions:

- `src/retrieval/semantic.rs` — entire file (29 symbols: `EmbeddingProvider`,
  `EmbeddingStore`, `StoredData`, `TfidfModel`, `detect_embedding_provider`,
  `compute_tfidf_embeddings`, `compute_query_vector`, `tokenize`, plus tests).
- `src/service/agent_v1/catalog_search/semantic.rs` — entire file
  (`SemanticMode`, `apply_semantic`, `build_semantic_item`). Replaced by new
  `src/retrieval/embedder.rs` + `src/retrieval/fusion.rs` called from
  `catalog_search/mod.rs`.
- `tests/agent_v1_semantic.rs` — entire file. Replace with targeted
  integration tests (§11) against the new stack.

Targeted surgical edits:

- `src/service/agent_v1/args.rs::SearchCatalogArgs` lines 80–86: delete
  `semantic_weight` (L82) and `semantic_only` (L85). Keep `semantic`.
- `src/service/agent_v1/registry.rs::AGENT_V1_HANDLERS` L11–26: delete
  `("embed_repo", CodeService::embed_repo)` entry. Evaluate deleting
  `search_columns`, `check_references`, `get_cross_repo_map` in the same
  pass — they are registry-only, not in `src/tools.json`, matching the
  "public surface should end smaller than today" goal (§4.3). Confirm with
  maintainer before scope creep.
- `src/service/agent_v1/index_admin.rs::CodeService.embed_repo` L211–304:
  delete entire method and its `Args` struct.
- `src/service/agent_v1/catalog_search/mod.rs::search_catalog` L464–466:
  delete the `if (semantic_enabled || semantic_only) && flags.want_symbol`
  block. Replace with dense-lane dispatch (§15.4).
- `src/service/agent_v1/catalog_search/mod.rs` L395–397: delete the
  `semantic_only` read and the `SemanticMode::resolve` call.
- `src/service/agent_v1/catalog_search/mod.rs` L84–90: delete the
  `"No embedding provider configured..."` error path; replace with fallback
  warning (§15.4).
- `src/config/app.rs::AppConfig.embedding_path` L98–100: delete. Grep all
  callers — currently only `semantic.rs` and `index_admin.rs::embed_repo`.
- `src/config/env.rs` L17: delete `pub const EMBED_MODEL`. No other uses
  outside the deleted TF-IDF path.

Env/config that disappears (no new callers): `CODESIGHT_EMBED_MODEL`,
`OPENAI_API_KEY`, `OPENAI_EMBED_MODEL`, `GOOGLE_API_KEY`,
`GOOGLE_EMBED_MODEL`. Removed by deleting `detect_embedding_provider` and
its error messages (the only readers).

### 15.2 Storage hook points (WS0, WS2, WS3)

SQLite open path — sqlite-vec registration:

- `crates/codesight-storage/src/db.rs::apply_rw_pragmas` L82–92: add
  `sqlite_vec::load(&conn)?` call *after* pragmas. Must also register on
  the RO path: extend `open_ro` L96–104 with the same loader (sqlite-vec is
  a query-time dependency, readers need it too).

Code-store DDL:

- `crates/codesight-storage/src/storage/schema.rs::INDEX_STORE_INIT_SQL_TAIL`
  L7–79: append `CREATE VIRTUAL TABLE IF NOT EXISTS vec_symbol_embeddings
  USING vec0(symbol_rowid INTEGER PRIMARY KEY, embedding FLOAT[DIM])`. The
  existing `symbols` table already has `row_id INTEGER PRIMARY KEY
  AUTOINCREMENT` (L16–34) and `symbol_fts` already uses
  `content_rowid='row_id'` (L37–45) — the key join is already the right
  shape.

Docs-store DDL:

- `crates/codesight-storage/src/storage/doc_schema.rs::DOC_STORE_INIT_SQL_TAIL`
  L15–67: append `CREATE VIRTUAL TABLE IF NOT EXISTS vec_section_embeddings
  USING vec0(section_rowid INTEGER PRIMARY KEY, embedding FLOAT[DIM])`.
  `sections` table L24–40 already has `row_id` and `section_fts` uses
  `content_rowid='row_id'` (L41–50).
- Bump `DOC_INDEX_TARGET_USER_VERSION` (currently `1`, L75) to `2` and add
  a forward-only migration branch in `apply_doc_index_migrations` (L79–85)
  that creates the vec table on existing stores. *Exception to §1.4 in
  letter only*: sqlite-vec table creation is safe against existing data
  because it carries no rows until ingest populates it. If the maintainer
  prefers strict invalidation semantics, skip the migration and let the
  store-version mismatch (§15.5) force a rebuild — cleaner.

New storage helpers (WS2/WS3 deliverables, host crate =
`codesight-storage`):

- `crates/codesight-storage/src/storage/vector_index.rs` (new): functions
  `insert_symbol_vectors(tx, stable_id → embedding)`,
  `delete_symbol_vectors(tx, stable_ids)`,
  `query_symbol_knn(conn, query_vec, k) -> Vec<(row_id, distance)>`, plus
  section-side mirrors. Each write helper resolves stable_id → row_id via
  a `SELECT row_id FROM symbols WHERE id = ?` inside the same transaction
  (no stable ID leaks into vec tables). Pattern already used in
  `crates/codesight-storage/src/storage/symbol_reads.rs::symbol_record`
  (L63).

### 15.3 Ingest wiring (WS2, WS3)

Code ingest entry:

- `src/service/agent_v1/ingest.rs::ingest_sources` L17–233. L155 dispatches
  to `do_index_folder_with_warnings`.
- `src/service/code_tools.rs::do_index_folder_with_warnings` L26+ owns the
  crawl → extract → write pipeline. Add a post-write embedding phase here:
  batch-encode all newly-inserted/updated symbols using the singleton
  embedder on `CodeService`, then call the new
  `vector_index::insert_symbol_vectors` helper inside the existing write
  transaction. Do NOT detach into a background task — §9 mandates unified
  writes.

Docs ingest entry:

- `do_index_local` (same module) is the docs counterpart. Mirror the code
  path: after sections land via `DocStore`, embed and call
  `insert_section_vectors`.

Incremental delete coverage:

- Symbol removal already cascades through `symbols_ad` trigger (schema.rs
  L53–56). Add an equivalent trigger or explicit `delete_symbol_vectors`
  call to wipe vec rows when symbols disappear. Triggers are cleaner
  because they stay in sync with the FTS cleanup; vec0 supports DELETE
  statements keyed on `symbol_rowid`.

Full reindex invalidation:

- `wipe_indexes_for_repo` (referenced at `ingest.rs:152`) already drops
  both DB files. No change needed — new vec tables get created on
  `open_rw` via `INDEX_STORE_INIT_SQL`.

### 15.4 Query integration (WS5)

`src/service/agent_v1/catalog_search/mod.rs::search_catalog` L384–491 is
the integration site. New flow:

1. Resolve `semantic_enabled = args.semantic.unwrap_or(false) ||
   query_mode_effective == "natural_language"`. This replaces L395–397.
2. After `search_code_side`/`search_doc_side` populate lexical items,
   branch on `semantic_enabled && flags.want_symbol`:
   - Encode query via `CodeService.embedder` (new field, WS1).
   - Call `vector_index::query_symbol_knn(store, qvec, k)` and, if docs
     in play, `query_section_knn`.
   - Run `retrieval::fusion::rrf` per domain over `(bm25_rank,
     knn_rank)`. Replace the current items list.
3. On model-load-fail / vec-missing / embedder-unavailable, append a
   `SEMANTIC_FALLBACK` warning (§4.1 shape) and keep lexical items.
   Mechanism already in place: `success_response_with_warnings`
   (`src/service/mod.rs:180`).

File-text hits path (L432+): already lexical-only — no change needed.

### 15.5 Invalid-index detection (WS4)

- `check_staleness` (`crates/codesight-storage/src/db.rs:138`) is the
  current stale-detection seam. Extend it (or add a sibling) to also
  check `PRAGMA user_version` against `INDEX_STORE_TARGET_USER_VERSION`
  (needs to be introduced, mirroring `DOC_INDEX_TARGET_USER_VERSION`
  at `doc_schema.rs:75`).
- On mismatch: surface through `manage_indexes status`
  (`index_admin.rs::manage_indexes`) and have `search_catalog` emit
  `SEMANTIC_FALLBACK { reason: "index_rebuild_required" }` until
  `ingest_sources` with `incremental: false` runs.

### 15.6 Embedder singleton placement (WS1)

- `src/service/mod.rs::CodeService` struct L59–65: add
  `embedder: Arc<OnceCell<Result<Embedder, EmbedderError>>>`.
- `CodeService::new` L68–88: construct the cell lazily — do not block
  startup on model load. First `search_catalog` with semantic enabled
  triggers initialization; failures become `SEMANTIC_FALLBACK` warnings.
- `src/retrieval/embedder.rs` (new): wraps `model2vec_rs::Model` with a
  `encode(&[&str]) -> Vec<Vec<f32>>` batching API. Dim must be exposed
  so the DDL constant `DIM` (§6.1, §6.2) can be compile-time-known or
  checked at open.

### 15.7 Surface-shrink scoreboard

Approximate deltas (verified via outline counts):

| Area | Symbols deleted | Symbols added |
|---|---|---|
| `src/retrieval/semantic.rs` | 29 | 0 |
| `src/retrieval/embedder.rs` | 0 | ~4 |
| `src/retrieval/fusion.rs` | 0 | ~2 |
| `catalog_search/semantic.rs` | 5 | 0 |
| `index_admin.rs::embed_repo` | 2 | 0 |
| `registry.rs` entry | 1 | 0 |
| `args.rs` (two fields) | 2 | 0 |
| `config/app.rs::embedding_path` | 1 | 0 |
| `config/env.rs::EMBED_MODEL` | 1 | 0 |
| `storage/vector_index.rs` | 0 | ~6 |
| `tests/agent_v1_semantic.rs` | ~25 | 0 |

Net: strongly negative symbol count, matches §13.

### 15.8 Suggested commit sequence

1. `ws0-spike`: add `model2vec-rs` + `sqlite-vec` deps; register loader;
   one smoke test in `crates/codesight-storage/src/db.rs` tests module.
2. `ws1-embedder`: land `src/retrieval/embedder.rs` + `fusion.rs`;
   `CodeService` owns singleton; still unused.
3. `ws2-code-vec`: DDL, `vector_index.rs` helpers, ingest write path for
   symbols. Ends with storage-level integration test.
4. `ws3-docs-vec`: mirror for sections.
5. `ws4-invalidation`: version bump + fallback plumbing.
6. `ws5-delete-and-wire`: delete the TF-IDF modules and stale args/env/
   tools; wire new path into `search_catalog`; update `src/tools.json`;
   update `tests/agent_v1_semantic.rs` → new `tests/agent_v1_dense.rs`
   targeted tests.
7. `ws6-harden`: goldens + perf measurements recorded in
   `REFACTOR_WORKLOG.md`.

Each commit must leave `cargo nextest run --lib` green on the root crate
and both middle crates.

