# IDF_REPLACEMENT.md

Replace the hand-rolled TF-IDF "semantic" engine with **Model2Vec static
embeddings + sqlite-vec** for real semantic retrieval, hybrid-ranked with the
existing FTS5/BM25 path. Net effect: `query_mode=natural_language` and
`semantic=true` stop being aspirational and start working — with a **smaller
codebase, a single binary, and sub-20 ms hybrid queries**.

> Scope: one contiguous refactor. No backwards compatibility, no dual code
> paths, no deprecation window. `tools.json` changes ship in the same patch.

---

## 1. Why we're doing this

The current TF-IDF path (`src/retrieval/semantic.rs`, 392 LoC) is:

- **Redundant with FTS5.** TF-IDF and BM25 both rank by lexical overlap; we
  already have BM25 inside SQLite FTS5. The "semantic" flag returns
  essentially the same signal as `query_mode=keyword` — validated by direct
  comparison in session 2026-04-13.
- **Not semantic.** No synonym bridging, no concept mapping. The exemplar
  failure: query *"what happens when a file changes on disk"* misses
  `file_watcher`, `debounce`, and delta reindex entirely.
- **Parallel storage.** JSON vector store at `embedding_path()` duplicates
  what should live in the same SQLite DB as everything else. Breaks the
  "one DB per workspace" invariant.
- **Dead provider detection.** `detect_embedding_provider()` plumbs
  `sentence_transformers`/`gemini`/`openai` env vars that nobody wires; pure
  aspirational code.

Per AGENTS.md "shrink the code, always" and "delete useless features
aggressively" — this is the textbook candidate.

---

## 2. Chosen stack

| Layer | Crate | Role | Size impact |
|---|---|---|---|
| Embedding | `model2vec-rs` | Static-embedding inference (pure Rust, no ONNX/Metal/CUDA) | ~200 KB code |
| Model | `potion-base-32M` (or `potion-base-8M`) | Pretrained distilled model, baked into binary | ~30 MB (or ~15 MB) |
| Vector store | `sqlite-vec` (statically linked) | `vec0` virtual table, KNN on cosine | ~400 KB compiled |
| BM25 | existing FTS5 | Unchanged lexical retrieval | 0 |
| Fusion | new ~40-line module | Reciprocal Rank Fusion (RRF) over FTS5 ∪ KNN | — |

Why these specifically:
- **Pure Rust + static linking** keeps the "single binary" property intact.
  No ONNX runtime (rules out fastembed-rs), no Metal (rules out candle/mlx),
  no Python shim.
- **Static embeddings** = token-lookup + mean-pool, sub-millisecond query
  encode on CPU. Satisfies the <50 ms cached / <200 ms cold budget with
  headroom.
- **sqlite-vec** lives in the same DB file as symbols and FTS5. One
  connection, one transaction, one backup. Matches the existing invariant
  enforced by `codesight_storage::db::open_rw/open_ro`.

---

## 3. Code-reduction target

**Delete (≈1,100 LoC):**

| File | LoC | Action |
|---|---|---|
| `src/retrieval/semantic.rs` | 392 | delete entirely |
| `src/service/agent_v1/catalog_search/semantic.rs` | 130 | rewrite (~60 LoC — net -70) |
| `tests/agent_v1_semantic.rs` | 574 | delete; replace with ~150 LoC targeting the new engine |
| `src/service/agent_v1/index_admin.rs` (TF-IDF block ~L220–300) | ~80 | delete; embedding is automatic during ingest |
| `detect_embedding_provider`, `EMBED_MODEL` env var, `OPENAI_*`/`GOOGLE_*` provider stubs | ~40 | delete |
| `src/config/app.rs::embedding_path` (JSON file location) | ~5 | delete — vectors live in SQLite now |

**Add (≈500 LoC):**

| File | LoC | Role |
|---|---|---|
| `src/retrieval/embedder.rs` | ~80 | Model2Vec loader, `Embedder::embed(&str) -> Vec<f32>` |
| `crates/codesight-storage/src/storage/vector_index.rs` | ~140 | vec0 schema init, insert/delete/knn helpers |
| `crates/codesight-storage/src/storage/migrations.rs` (new migration) | ~25 | CREATE VIRTUAL TABLE + backfill |
| `src/service/agent_v1/catalog_search/semantic.rs` (rewrite) | ~60 | apply_semantic using embedder + vec0 + RRF |
| `src/retrieval/fusion.rs` | ~40 | RRF over two ranked lists |
| `tests/semantic_hybrid.rs` | ~150 | end-to-end hybrid search correctness |
| indexing hook in ingest path | ~30 | embed on symbol insert/update, delete on prune |

**Net: approximately −600 LoC**, one fewer on-disk format, one fewer
aspirational provider layer, one real retrieval capability.

---

## 4. Architecture

### 4.1 Indexing (eager, during ingest)

Every symbol insert/update/delete inside
`crates/codesight-storage` gets a matching embedding write. No separate
`manage_indexes action=embed_symbols` step — that tool surface is deleted.
Embedding runs synchronously in the ingest transaction so freshness is
guaranteed (no "embeddings are stale" state).

```text
ingest_sources
    ├── tree-sitter extract → SymbolRecord
    ├── FTS5 insert          (unchanged)
    └── vec_symbols insert   ← new: embedder.embed(chunk_text)
```

**Chunk text** (what we embed per symbol):

```
{kind} {name}
{signature}
{leading_doc_comment}
{body_first_N_chars}   // N = 512 for functions/methods, 256 for types
```

No file-path padding, no synthetic metadata. Let the model see real code.
If the symbol has no body (e.g. a trait method signature), embed just
signature + doc.

**Doc sections** get the same treatment on the docs side — embed
`heading_path + summary + body_first_512_chars`. Unlocks
`domain=hybrid` as a real cross-corpus semantic search, not just a union
of two BM25 indexes.

### 4.2 Query path

```rust
// src/service/agent_v1/catalog_search/semantic.rs (new, ~60 LoC)
pub(super) fn apply_semantic(
    service: &CodeService,
    args: &SearchCatalogArgs,
    limit: usize,
    mode: &SemanticMode,
    items: &mut Vec<Value>,
) {
    let Some(embedder) = service.embedder() else { return };          // singleton
    let Ok(store) = service.ensure_repo_store(&args.source_id, ..) else { return };

    let qvec = embedder.embed(&args.query);
    let knn = store.vec_knn(&qvec, limit * 2)?;                       // sqlite-vec KNN
    match mode {
        SemanticMode::Only => {
            items.clear();
            items.extend(knn.into_iter().map(|h| to_item(&args.source_id, h)));
        }
        SemanticMode::Hybrid => {
            // RRF over (existing BM25 items) ∪ (knn)
            *items = fusion::rrf_merge(items.drain(..), knn, /*k=*/60, limit);
        }
        SemanticMode::Bm25 => {}                                      // no-op
    }
}
```

### 4.3 Fusion: RRF not weighted sum

The user-provided spec proposed `score = α·cosine + (1-α)·bm25`. **Reject
this** — BM25 scores and cosine similarities live on incommensurate scales;
α-tuning is per-corpus. Use **Reciprocal Rank Fusion** (Cormack et al.
2009) as the default:

```rust
// src/retrieval/fusion.rs
pub fn rrf_merge(a: impl Iterator<Item=Hit>, b: Vec<Hit>, k: usize, top: usize) -> Vec<Hit> {
    let mut scores: HashMap<String, f64> = HashMap::new();
    for (rank, hit) in a.enumerate()      { *scores.entry(hit.id.clone()).or_default() += 1.0 / (k + rank + 1) as f64; }
    for (rank, hit) in b.iter().enumerate() { *scores.entry(hit.id.clone()).or_default() += 1.0 / (k + rank + 1) as f64; }
    // resolve to items, sort by score desc, truncate to `top`
}
```

RRF is parameter-free (k=60 is the field default), score-scale-agnostic,
and consistently outperforms weighted sums on hybrid IR benchmarks. We
keep `semantic_weight` in `tools.json` **only** as an advanced override
that switches to weighted-sum mode; default stays RRF.

---

## 5. SQLite schema

New migration in
`crates/codesight-storage/src/storage/migrations.rs` — bumps
`CODE_INDEX_TARGET_USER_VERSION`:

```sql
-- Migration N: add vector index
-- Requires sqlite-vec loaded via sqlite3_auto_extension at open_rw/open_ro.
CREATE VIRTUAL TABLE vec_symbols USING vec0(
    symbol_id TEXT PRIMARY KEY,            -- same stable_id as symbols table
    embedding FLOAT[DIM]                   -- DIM = model output dim (e.g. 256)
);

CREATE VIRTUAL TABLE vec_sections USING vec0(
    section_id TEXT PRIMARY KEY,
    embedding FLOAT[DIM]
);
```

- **DIM is a compile-time constant** driven by the baked-in model. Changing
  models = new migration + reindex. No on-the-fly reshape.
- **No backfill of existing indexes.** Per AGENTS.md "no backwards
  compatibility": bump `user_version`, on open detect version skew, force a
  reindex. Users re-run `ingest_sources` once. Documented in worklog.
- `vec_sections` only gets populated when the docs store exists.

### Loading sqlite-vec statically

In `crates/codesight-storage/src/storage/db.rs` (modify `open_rw`/`open_ro`):

```rust
use rusqlite::ffi::sqlite3_auto_extension;
use sqlite_vec::sqlite3_vec_init;

fn ensure_vec_extension() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        sqlite3_auto_extension(Some(std::mem::transmute(sqlite3_vec_init as *const ())));
    });
}
```

Called exactly once before the first `Connection::open`. No runtime
`.so`/`.dylib` lookup, no `load_extension` permission bit.

---

## 6. Model packaging

```toml
# Cargo.toml (root)
[dependencies]
model2vec-rs = "0.1"
sqlite-vec   = "0.1"
rust-embed   = { version = "8", optional = true }

[features]
default       = ["bundled-model"]
bundled-model = ["rust-embed"]    # bakes potion-base-32M into the binary
```

```rust
// src/retrieval/embedder.rs
#[cfg(feature = "bundled-model")]
#[derive(rust_embed::RustEmbed)]
#[folder = "models/potion-base-32M/"]
struct BundledModel;

pub struct Embedder { inner: model2vec_rs::Model }

impl Embedder {
    pub fn load() -> anyhow::Result<Self> {
        #[cfg(feature = "bundled-model")] {
            let bytes = BundledModel::get("model.safetensors").unwrap();
            let tok   = BundledModel::get("tokenizer.json").unwrap();
            Self::from_bytes(&bytes.data, &tok.data)
        }
        #[cfg(not(feature = "bundled-model"))] {
            let path = std::env::var("CODESIGHT_MODEL_PATH")
                .unwrap_or_else(|_| "~/.codesight/models/potion-base-32M".into());
            Self::from_path(path)
        }
    }

    pub fn embed(&self, text: &str) -> Vec<f32> {
        self.inner.embed(text)              // ~0.3–0.8 ms for <2 KB text
    }

    pub fn dim(&self) -> usize { self.inner.dim() }
}
```

**Singleton.** `Embedder::load()` is called once at service construction
(`CodeService::new`) and stored behind an `Arc`. No per-request allocation.

### Binary size math

| Variant | Binary (LTO + strip + panic=abort) |
|---|---|
| Current codesight-mcp | ~18 MB |
| + model2vec-rs + sqlite-vec (no model) | ~19 MB |
| + potion-base-8M baked in | **~34 MB** |
| + potion-base-32M baked in | **~48 MB** |

Recommendation: **ship `potion-base-8M` as the bundled default** (34 MB
is still a very reasonable single binary). Document `--no-default-features`
+ `CODESIGHT_MODEL_PATH` for users who want potion-base-32M or a custom
code-distilled model.

---

## 7. `tools.json` contract change

Ship in the same patch — no shim, no alias.

```diff
  "search_catalog": {
    "properties": {
-     "semantic":        { "type": "boolean", "description": "..." },
-     "semantic_only":   { "type": "boolean", "description": "..." },
-     "semantic_weight": { "type": "number",  "description": "α in [0,1]" },
+     "semantic":        { "type": "boolean", "description": "Hybrid: merge dense embedding KNN with BM25 via RRF." },
+     "semantic_only":   { "type": "boolean", "description": "Replace BM25 with embedding KNN entirely." },
+     "semantic_weight": { "type": "number",  "description": "Advanced: switch to weighted-sum fusion with α = this value. Default is RRF (parameter-free)." }
    }
  }
```

And — critically — **`query_mode=natural_language` now auto-enables
`semantic=true`.** Today it only flips `domain=hybrid` (useless alone).
After this change:

```rust
// catalog_search/mod.rs
let semantic_enabled =
    args.semantic.unwrap_or(false) ||
    query_mode_effective == "natural_language";
```

This is the single most impactful line in the whole refactor. It makes the
mode name honest.

Also delete these args that only existed for TF-IDF plumbing:
- any `rebuild_embeddings` action on `manage_indexes` (embedding is now
  automatic during `ingest_sources`)
- `CODESIGHT_EMBED_MODEL`, `OPENAI_API_KEY`/`OPENAI_EMBED_MODEL`,
  `GOOGLE_API_KEY`/`GOOGLE_EMBED_MODEL` references

---

## 8. Performance budget & verification

| Operation | Target | How measured |
|---|---|---|
| `Embedder::embed(query)` | < 1 ms | `cargo bench --bench embed` (microbench) |
| `vec_symbols` KNN (k=20, N=4k) | < 3 ms | Bench against this repo's indexed state |
| RRF merge (20 + 20 items) | < 0.2 ms | microbench |
| Full hybrid `search_catalog` | **< 20 ms p95** cold, **< 10 ms** warm | `timing_ms` meta field in integration test |
| Cold ingest (this repo, ~4100 syms) | **< 7 s** (was < 5 s) | wall-clock on `ingest_sources path=.` |
| Incremental reindex (single file) | **< 150 ms** (was < 100 ms) | benchmark in worklog |

Ingest regresses by ~2 s because we now embed every symbol eagerly. That's
an acceptable trade — the alternative (deferred/async embedding) creates a
"stale vector index" state that violates the stateless-per-request
invariant. Worklog gets an entry to revisit if users complain.

Every `search_catalog` response already returns `_meta.timing_ms`; integration
test asserts p95 under budget across 50 queries from
`tests/fixtures/semantic_queries.yaml` (new file).

---

## 9. Correctness harness

Replace `tests/agent_v1_semantic.rs` (574 LoC of TF-IDF internals) with
**behavioral goldens**:

```yaml
# tests/fixtures/semantic_queries.yaml
- query: "what is the main dispatch loop for MCP tool calls"
  must_include_symbol: ["run_dispatch_loop"]
  must_not_rank_above_top_5: ["run_for_loop_increases_hints"]
- query: "how does error handling work"
  must_include_symbol_prefix: ["ToolError", "IndexError"]
  must_not_top: ["run_missing_repo_error"]
- query: "what happens when a file changes on disk"
  must_include_file: ["src/service/file_watcher.rs"]
- query: "parse tree-sitter symbols from rust"
  must_include_file: ["crates/codesight-extractors/"]
```

Test harness runs against the live index, asserts recall invariants rather
than exact rank order (ranks drift with model versions). If any golden
regresses, CI fails — that's the semantic-quality tripwire.

Keep this fixture small (≤20 queries). It's a quality gate, not a
benchmark suite.

---

## 10. Rollout milestones

One branch, no feature flags. Each milestone is a reviewable commit.

1. **M1 — Dependencies + model.** Add `model2vec-rs`, `sqlite-vec`,
   `rust-embed` to `Cargo.toml`. Commit `models/potion-base-8M/` via
   Git LFS. Binary builds, no behavior change yet.
2. **M2 — `Embedder` singleton.** `src/retrieval/embedder.rs` + wired into
   `CodeService::new`. Unit tests for embed determinism + dim.
3. **M3 — Schema migration + vec extension loader.** New migration,
   `sqlite3_auto_extension` hook, `store.vec_knn(vec, k)` helper.
4. **M4 — Indexing hook.** Embed-on-insert in the ingest path.
   `ingest_sources` now populates `vec_symbols`. Delete the separate
   TF-IDF embedding block in `index_admin.rs`.
5. **M5 — Query integration.** New `apply_semantic` + RRF fusion.
   Rewire `query_mode=natural_language` to auto-enable semantic.
6. **M6 — `tools.json` update + doc sections.** Ship contract change.
   Add `vec_sections` population on the docs side.
7. **M7 — Delete TF-IDF.** Remove `src/retrieval/semantic.rs`,
   `detect_embedding_provider`, `embedding_path`, old JSON store, old test
   file. `cargo check` is green, full suite passes.
8. **M8 — Golden harness.** Add `tests/fixtures/semantic_queries.yaml` and
   `tests/semantic_hybrid.rs`. Verify the four demonstrated failure
   cases now pass.
9. **M9 — Worklog + performance numbers.** Update
   `REFACTOR_WORKLOG.md`: baseline LoC, new LoC, new timing numbers,
   confirm "tokens saved" regression-free on the live `_savings.json`.

Each milestone lands on `dev`. Main merge only after M9.

---

## 11. Risks & mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| `model2vec-rs` crate instability (v0.1.x) | medium | Pin exact version; vendor if needed — the model format is a frozen safetensors + tokenizer.json pair, so we can always load manually with `safetensors` + `tokenizers` crates as a ~120 LoC fallback |
| `sqlite-vec` extension-loading friction with rusqlite `bundled` feature | medium | Prototype in M1; if it fights us, fall back to a parallel `_vectors` table with a hand-rolled brute-force cosine KNN (≤30 ms for 4k vectors — still fine) |
| Binary size balloon | low | Default to potion-base-8M, feature-gate the bundle, document external-model path |
| Static embedding quality below user expectation | low/medium | Hybrid via RRF means BM25 catches what embeddings miss. Escape hatch: distill a code-specialized model via the official Python model2vec tool, swap the baked bytes, bump migration version |
| Indexing latency regression | medium | Measured in the performance budget table; if > 10 s on this repo, batch embeddings in groups of 128 with SIMD |
| Someone on the team reaches for the old `CODESIGHT_EMBED_MODEL` env var | low | Deleted outright — loud failure beats silent stub. The worklog lists this as a breaking change |

---

## 12. What this unlocks next

Not in scope for this patch, but now cheap:

- **`evaluate_quality analysis=plan_turn`** — the ranked read-plan already
  mooted in `REFACTOR_WORKLOG.md` becomes a trivial wrapper: embed the
  user's task description, KNN over symbols + sections, return the top-N
  with token estimates.
- **Duplicate-code detection** — cosine similarity between symbol
  embeddings identifies semantic clones that AST-based detectors miss
  (same logic, different names/structure).
- **`search_catalog query_mode=natural_language domain=hybrid`** finally
  matches the description in `search_catalog`'s tool schema: "natural
  language hits both code and docs semantically."

None of these require new infra. They're follow-up PRs of ~100 LoC each
that consume the embedder and vec0 table this plan installs.

---

## 13. Success criteria

This patch is done when all of the following are true:

1. `git diff --stat` shows a **net reduction of ≥ 500 LoC** across `src/`
   and `tests/`.
2. The four failing queries from the 2026-04-13 session (`dispatch loop`,
   `error handling`, `file changes on disk`, `parse tree-sitter symbols`)
   all return correct top-5 hits.
3. `search_catalog` p95 latency ≤ 20 ms on this repo's index.
4. Binary size ≤ 35 MB with default features.
5. `cargo nextest run` passes with zero `#[ignore]` additions.
6. `_savings.json` shows non-negative token savings for the golden
   semantic queries (the new `natural_language` behavior must never
   surface *more* tokens than naive BM25).
7. `REFACTOR_WORKLOG.md` reflects the new baseline numbers and the
   closed work items.

If any of these miss, the branch doesn't merge.
