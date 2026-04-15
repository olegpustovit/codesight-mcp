# REFACTOR_WORKLOG.md

Single living state for `codesight-mcp`. Update this file in the same patch
as the change it describes. Move closed items out — git history is the
archive. See [`AGENTS.md`](AGENTS.md) for the policies that govern this work.

---

## 2026-04-14 — IDF replacement verification follow-up

Verified the `IDF_REPLACEMENT.md` rollout against the live code and fixed the
remaining drift in the dense lane:

- **Dense results now survive lexical misses.** `search_catalog` no longer
  drops dense-only symbol/section hits during RRF merge; dense materialization
  happens from stable ids, and file hits stay appended after the fused
  symbol/section lanes as specified.
- **Public contract restored.** `src/tools.json` again exposes
  `search_catalog.semantic`, and file-only semantic requests now return the
  required `SEMANTIC_FALLBACK` warning with `reason=files_only`.
- **Incremental embedding narrowed.** Post-ingest embedding now re-encodes
  only updated code/doc files instead of the whole store. Replaced/removed
  files also clear their vec rows before lexical deletion so vec tables do not
  accumulate stale rowids across incremental ingests.
- **Lexical + vec writes are now atomic.** Code/doc ingest builds embeddings
  before commit and writes lexical rows plus `vec_*_embeddings` rows inside the
  same SQLite transaction. The second-phase reopen/requery/write path is gone,
  along with the old storage-only embedding-input helpers.
- **Bundled model, zero config.** `potion-base-8M` now ships inside the
  binary via vendored assets under `assets/embedder/`; runtime extracts the
  three required files into a shared local cache dir for `model2vec-rs`.
  The `CODESIGHT_EMBEDDING_MODEL_PATH` / `CODESIGHT_EMBEDDING_MODEL_REPO`
  knobs are gone.
- **Dense test hotpath trimmed.** `tests/agent_v1_dense.rs` now shares one
  indexed fixture across the read-only cases, and the embedder unit tests
  reuse one loaded model. That removes repeated cold ingest/model-load work
  from the test code while preserving coverage.
- **Warm semantic perf guardrail added.** `tests/search_perf.rs` now covers
  semantic result presence plus warm semantic latency on the shared indexed
  fixture, so portability-friendly CPU inference still has an explicit budget.
- **Verification.** `cargo nextest run --test agent_v1_dense`,
  `cargo nextest run -p codesight-mcp --lib`,
  `cargo nextest run -p codesight-storage --lib`, and
  `cargo check -p codesight-storage --benches`.

## 2026-04-14 — TF-IDF → dense embeddings (IDF_REPLACEMENT.md)

Landed the full IDF replacement across WS0–WS6. Outcomes:

- **Deleted.** `src/retrieval/semantic.rs` (29 symbols),
  `tests/agent_v1_semantic.rs`, `embed_repo` registry entry + handler,
  runtime embedder model-path/repo config knobs,
  `SearchCatalogArgs::{semantic_only, semantic_weight}`. Net deletion.
- **Added.** `src/retrieval/embedder.rs` (model2vec-rs wrapper, lazy
  `EmbedderSlot`), `src/retrieval/fusion.rs` (RRF), storage-side
  `crates/codesight-storage/src/storage/vector_index.rs` with
  insert/delete/KNN for symbols and sections.
- **Storage.** sqlite-vec auto-registered via `sqlite3_auto_extension`
  in `db.rs::ensure_sqlite_vec_registered`. DDL migration: code
  `user_version` 5→6, docs 1→2, both adding `vec_*_embeddings` vec0
  tables keyed by `row_id`. `EMBEDDING_DIM=256` pinned for
  `potion-base-8M`.
- **Wiring.** `do_index_folder_with_warnings` embeds symbols after
  crawl; `do_index_local` does sections. `search_catalog` runs
  `run_dense_lane` when `semantic=true` or
  `query_mode=natural_language`, fuses via RRF, emits
  `SEMANTIC_FALLBACK` on model/vec failure — never hard-fails.
- **Nix darwin.** `.cargo/config.toml` now pins
  `CC=/usr/bin/clang`, `CXX=/usr/bin/clang++`, and adds the
  libiconv RUSTFLAGS shim required by `sqlite-vec`.
- **Tests green.** `storage::vector_index::sqlite_vec_available_and_roundtrips`,
  `retrieval::fusion::*`, and four new `tests/agent_v1_dense.rs` cases
  (fallback warning, natural_language-implies-semantic, deleted args
  silently ignored, lexical-only path clean).

## 2026-04-15 — Crate graph audit and policy unfreeze

Audited the workspace split against the current code shape before changing any
crate boundaries. Result:

- **No immediate forced split.** The current four-crate layout is still
  coherent, but the "exactly four crates" rule had become policy debt rather
  than architectural guidance.
- **Highest-feasibility next split:** `src/framework/` into its own crate.
  It is largely self-contained, heavily tested, and carries parser-heavy
  dependencies that currently bloat the root crate compile surface.
- **Medium-feasibility split:** move `src/indexing/imports/` behind either
  `codesight-extractors` or a dedicated import-analysis crate. This would
  further pull tree-sitter language dependencies out of the root crate, but it
  needs a cleaner seam because graph analysis and service handlers consume it.
- **Lower-feasibility split:** `src/analysis/` as a crate. The domain is real,
  but it still reaches into storage and indexing often enough that splitting it
  today would mostly move coupling around.
- **Policy change:** removed frozen-crate-graph language from
  `AGENTS.md`, `ARCHITECTURE.md`, `IDF_REPLACEMENT.md`, and `EXCEL_PLAN.md`.
  `tests/crate_graph.rs` now enforces dependency invariants instead of
  "exactly four workspace crates".
- **Architecture roadmap added.** `ARCHITECTURE.md` now lists the feasible
  split candidates in order: `codesight-framework`,
  `codesight-imports`, `codesight-retrieval`, then `codesight-analysis`,
  along with gains and prerequisites for each.
- **Verification:** `cargo nextest run --test crate_graph`.

---

## Current metrics (baseline: 2026-04-13, updated 2026-04-13 session 3)

| Metric | Current | Target | Trend |
|---|---|---|---|
| Product crate LoC (`src/`) | **~28.8k** | ↓ each session | ↓ -94 (workspace_digest) |
| Workspace crate LoC (`crates/`) | core 0.2k / extractors 9.4k / storage 3.1k | stable | stable |
| Governance score | **7780** (very good) | ↑ | ↑ +2 this session |
| Structural score | **8072** (very good) | ↑ | stable |
| Agent readiness score | **7498** (very good) | ↑ | ↑ +4 this session |
| Dependency cycles | **0** | 0 | ✅ |
| Cold dispatch handler (cached read) | **0.6 ms** (64 ms wall incl. startup) | < 50 ms | ✅ |
| Cold full ingest (this repo) | **1.8 s** (413 files, 4143 symbols) | < 5 s | ✅ |
| Incremental reindex (background watcher) | **37 ms** | < 100 ms | ✅ |
| Full test suite (1545 tests) | **~28 s** uncached | ≤ 30 s | ✅ |
| `_savings.json` total tokens saved | **267M+** | ↑ each session | ✅ +20M this session |
| Slow tests (> 1 s) | needs audit | 0 | — |

**Measurement script:** `scripts/bench-hotpaths`. Times cold ingest, keyword
search, and delta reindex on this repo.

---

## Top hotspots (CC > 20)

| Function | CC | File | LoC |
|---|---|---|---|
| `extract_dbt_macros` | 35 | `crates/.../sql/fallback.rs` | 237 |
| `DocStore::apply_index` | 35 | `src/storage/doc_index/store.rs` | 82 |
| `evaluate_impact_blast_radius` | 32 | `src/service/agent_v1/impact/blast_radius.rs` | 198 |
| `apply_index_with_deps` | 26 | `crates/.../index_store.rs` | 384 |
| `assemble_context` | 24 | `src/service/agent_v1/context.rs` | 301 |
| `dead_code_payload` | 22 | `src/analysis/dead_code/mod.rs` | 301 |
| `read_content` | 22 | `src/service/agent_v1/content.rs` | 211 |
| `ingest_sources` | 22 | `src/service/agent_v1/ingest.rs` | 289 |
| `insert_symbol` | 22 | `crates/.../index_store.rs` | 384 |
| `index_workspace` | 22 | `src/indexing/code_index.rs` | 351 |
| `search_columns` | 22 | `src/service/agent_v1/index_admin.rs` | 438 |
| `apply_code_index_migrations` | 21 | `crates/.../migrations.rs` | 257 |

**Bottleneck:** structural equality score (0.518) — driven by god-functions.

---

## Completed this session (2026-04-13 session 3)

| Task | Summary | LoC delta | Tests |
|---|---|---|---|
| T3 | Kill `workspace_digest` mode | -94 | 31 context/workspace pass |
| T4 | Rename `churn_rate`->`change_velocity`, `extraction_candidates`->`refactor_targets`, `coupling_metrics`->`module_coupling` | ~0 (renames) | 40 pass |
| T9 | Workspace dependency invariants gate (`tests/crate_graph.rs`) | +30 | 1 pass |

---

## Active workstreams — implementation-ready specs

Each item below is detailed enough to delegate to an agent in one go.

---

### T1 — Replace `BYTES_PER_TOKEN` with tiktoken

**Goal:** Accurate token counting everywhere. Replace the `/ 4` byte
approximation with real BPE tokenization.

**Crate:** Add `tiktoken = "3.1.2"` (pure Rust, no Python) to
`crates/codesight-storage/Cargo.toml`.

**Files to change:**

1. `crates/codesight-storage/src/config.rs:1` — delete
   `pub const BYTES_PER_TOKEN: u64 = 4;`. Add:
   ```rust
   use std::sync::OnceLock;
   use tiktoken::EncodingFactory;
   static ENCODER: OnceLock<tiktoken::Encoding> = OnceLock::new();
   pub fn count_tokens(text: &str) -> u64 {
       let enc = ENCODER.get_or_init(|| EncodingFactory::cl100k_base().unwrap());
       enc.encode(text, &[]).len() as u64
   }
   ```
   Keep `SCHEMA_VERSION` constant.

2. `crates/codesight-storage/src/storage/symbol_reads.rs` — line 9: change
   `use crate::config::BYTES_PER_TOKEN;` → `use crate::config::count_tokens;`.
   Lines 171–174: replace `file_size_bytes / BYTES_PER_TOKEN` with
   `count_tokens(&source_text)` (the source text is already available in
   `symbol_record`'s query result).

3. `crates/codesight-storage/src/storage/index_store.rs:200` — replace
   `total_bytes / crate::config::BYTES_PER_TOKEN` with
   `crate::config::count_tokens(...)` or keep byte-approx for aggregate
   stats (where we don't have the text handy).

4. `src/config/defaults.rs:3` — delete `pub const BYTES_PER_TOKEN: u64 = 4;`.

5. `src/service/agent_v1/util.rs:5` — change import from
   `use crate::config::BYTES_PER_TOKEN;` → either import from storage crate
   or add a re-export. Token budget enforcement in handlers should call
   `count_tokens` on the serialized JSON response.

6. `src/storage/doc_index/query.rs:11` — same: replace BYTES_PER_TOKEN
   import and usage.

7. `scripts/generate_agent_v1_modules.py:45,88,105` — update the generated
   import strings.

**Test:** `cargo nextest run -E 'test(token_budget)' -E 'test(envelope)'`
to verify budgets still truncate correctly. Token counts will differ from
byte/4 — update any hardcoded expected values.

**Risk:** `cl100k_base` initialization is ~5ms on first call; OnceLock
ensures it's paid once. No perf regression on hot paths.

---

### T2 — `env::var` cleanup

**Goal:** All environment variables read through `src/config/env.rs` consts.

**Files with raw `std::env::var` outside `env.rs`:**

1. `src/retrieval/summarizer.rs:93` — has a local `env_opt(key)` helper.
   Replace with `crate::config::env::opt_bool` or similar. The keys used
   inside should already be in `env.rs` (`SUMMARIZER_PROVIDER`,
   `SUMMARIZER_MODEL`). If the summarizer reads other keys, add them.

2. `src/server_support/path_map.rs:71` — uses `std::env::var(ENV_VAR)` where
   `ENV_VAR` is a local `const ENV_VAR: &str = "CODESIGHT_PATH_MAP"`. Delete
   the local const; use `crate::config::env::PATH_MAP` which already exists.

**Test:** `cargo nextest run -E 'test(semantic)' -E 'test(path_map)'`

---

### T5 — `ToolEnvelope<T>` (P11)

**Goal:** Unified `_meta` envelope. Every handler currently builds `_meta`
by hand via `MetaBuilder`. Replace with a typed wrapper.

**Design:**

```rust
// src/service/envelope.rs (new file, ~40 lines)
pub struct ToolEnvelope<T: Serialize> {
    pub data: T,
    pub meta: Meta,       // from src/service/savings.rs
    pub ok: bool,
    pub warnings: Vec<String>,
}

impl<T: Serialize> ToolEnvelope<T> {
    pub fn success(timer: &MetaBuilder, data: T) -> Value {
        // build _meta from timer, serialize data, combine
    }
    pub fn error(timer: &MetaBuilder, err: ToolError) -> Value { ... }
}
```

**Migration pattern:** Each handler currently ends with
`self.success_response(&timer, data)` or
`self.success_response_with_meta_extra(...)`. Replace `success_response` impl
to use `ToolEnvelope::success`. The `_meta_extra` fields (like
`methodology`, `confidence_level`, `domain_effective`) become typed optional
fields on `Meta` or a `MetaExtras` map.

**Key invariant:** `tests/envelope_shape.rs` already pins the required
`_meta` keys (`BASELINE_META_KEYS`). The `ToolEnvelope` must produce all of
them. Run that test throughout.

**Files:** `src/service/savings.rs` (Meta struct), new `src/service/envelope.rs`,
every handler in `src/service/agent_v1/` that calls `success_response`.

**Test:** `cargo nextest run --test envelope_shape`

---

### T6 — `evaluate_quality analysis=plan_turn` (D1)

**Goal:** Opening-move advisor. Agent calls this at session start to get a
ranked list of what to read first, with concrete `read_content` refs.

**Design:** New match arm in `src/service/agent_v1/quality/mod.rs` dispatch.
New file `src/service/agent_v1/quality/plan_turn.rs`.

**Input:** `{ source_id, analysis: "plan_turn", query?: string, max_tokens?: int }`

- `query` is optional — if absent, return the generic "onboarding" plan.
- If present, return a plan tailored to the query.

**Output:**
```json
{
  "analysis": "plan_turn",
  "plan": [
    {
      "step": 1,
      "action": "read_content",
      "reason": "Entry point — main dispatch loop",
      "ref": { "kind": "symbol", "stable_id": "...", "path": "...", "start_line": N },
      "estimated_tokens": 450,
      "confidence": 0.92
    },
    ...
  ],
  "total_estimated_tokens": 2800,
  "strategy": "centrality_guided"
}
```

**Implementation:**
1. Use `inspect_graph graph=centrality algorithm=pagerank top_n=10` internally
   to find the most central files.
2. If `query` is provided, run `search_catalog` internally to find relevant
   symbols, then rank by centrality + relevance.
3. For each entry, compute `estimated_tokens` from the symbol's
   `symbol_size_bytes` (or tiktoken after T1).
4. Return concrete `ref` objects that the agent can pass directly to
   `read_content`.

**Difference from prior-art `plan_turn`:** prior-art returns hints and
confidence. We return executable refs — zero extra calls needed.

**Difference from existing `query_suggestions`:** `query_suggestions`
(`src/service/agent_v1/context_tail/query_suggestions.rs`, 86 lines) returns
suggested *queries* (strings). `plan_turn` returns a *read plan* (refs with
token estimates). They're complementary — `query_suggestions` helps you
formulate searches; `plan_turn` tells you what to read.

**Files:** Add `"plan_turn"` to `evaluate_quality` analysis enum in
`src/tools.json`. New `src/service/agent_v1/quality/plan_turn.rs`. Update
dispatch in `src/service/agent_v1/quality/mod.rs`.

**Test:** New `tests/plan_turn.rs` — index a small fixture repo, call
`plan_turn`, assert response has `plan` array with `ref` objects that
include `stable_id`, `estimated_tokens > 0`, `confidence > 0`.

---

### T7 — `manage_indexes action=audit_config` (D2)

**Goal:** Scan agent instruction files for stale symbol references, dead
file paths, and token waste. codesight is a universal tool — support the
open standard first, then vendor conventions as fallbacks.

**Design:** New match arm in `src/service/agent_v1/index_admin.rs` dispatch
(`manage_indexes`). New file `src/analysis/audit_config.rs`.

**Input:** `{ source_id, action: "audit_config" }`

**Config file discovery** (scan all that exist, grouped by agent):

| Agent | Files (repo root) | Rule dirs |
|---|---|---|
| **Open standard** | `AGENTS.md` | — |
| **Claude Code** | `CLAUDE.md` | — |
| **Codex / OpenCode** | `AGENTS.md` (same) | — |
| **Cursor** | `.cursorrules` | `.cursor/rules/*.md` |
| **GitHub Copilot** | `.github/copilot-instructions.md` | `.github/copilot/*.md` |
| **Windsurf** | `.windsurfrules` | `.windsurf/rules/*.md` |
| **codesight** | `.codesight.toml` | — |

Also scan `CONTRIBUTING.md`, `ARCHITECTURE.md` — common project docs that
agents consume and that can contain stale refs.

Discovery list lives in a const array so adding a new agent convention is
one line. Rule directories are globbed for `*.md`/`*.mdc` files. An
optional `files` input param lets the caller override auto-discovery to
audit a specific file.

**Output:**
```json
{
  "action": "audit_config",
  "files_scanned": ["AGENTS.md", "CLAUDE.md"],
  "findings": [
    {
      "file": "AGENTS.md",
      "line": 42,
      "kind": "stale_symbol_ref",
      "text": "src/old_module.rs::OldFunction",
      "detail": "Symbol not found in index"
    },
    {
      "file": "AGENTS.md",
      "line": 87,
      "kind": "dead_file_path",
      "text": "src/removed_file.rs",
      "detail": "File does not exist on disk"
    },
    {
      "file": "CLAUDE.md",
      "kind": "token_waste",
      "estimated_tokens": 4200,
      "detail": "File is 4200 tokens — consider trimming"
    }
  ],
  "total_issues": 3
}
```

**Implementation:**
1. Discover config files using the priority list above. Check repo root
   for each. Collect all that exist.
2. Parse each file for patterns that look like file paths (regex:
   `[\w/.-]+\.(rs|py|ts|js|go|java|rb|php|c|cpp|h)` and
   `src/...`, `crates/...`, `lib/...`, `app/...`) or symbol-like
   references (`File::Symbol`, `module.function`, `path.rs::Name`).
3. Cross-reference each against the index: `store.file_exists(path)` for
   paths, `store.symbol_record(id)` for symbol-shaped strings.
4. Count tokens per config file for the waste metric (using tiktoken
   after T1, or byte-approx until then).

**Files:** `src/tools.json` — add `"audit_config"` to `manage_indexes`
action enum. Add optional `files` param (array of strings). New
`src/analysis/audit_config.rs`. Update dispatch in
`src/service/agent_v1/index_admin.rs`.

**Test:** New `tests/audit_config.rs` — create a tempdir with an
`AGENTS.md` containing a stale path and a valid path, index it, call
`audit_config`, assert findings include the stale one but not the valid one.

---

### T8 — `evaluate_quality analysis=ownership` (D3)

**Goal:** Code ownership analysis via git blame + CODEOWNERS.

**Design:** New match arm in `src/service/agent_v1/quality/mod.rs`. New file
`src/analysis/ownership.rs`.

**Input:** `{ source_id, analysis: "ownership", refs?: [...], top_n?: int }`

- Without `refs`: return repo-wide ownership summary.
- With `refs`: return ownership for specific files/symbols.

**Output:**
```json
{
  "analysis": "ownership",
  "owners": [
    {
      "file": "src/service/agent_v1/content.rs",
      "primary_author": "alice",
      "author_share": 0.72,
      "last_modified_days_ago": 3,
      "codeowners_match": "@team-core"
    }
  ],
  "top_contributors": [
    { "author": "alice", "files_owned": 45, "share": 0.38 }
  ]
}
```

**Implementation:**
1. Shell out to `git log --format='%an' --follow -- <file>` for each file
   in refs (or top_n most central files if no refs).
2. Parse `.github/CODEOWNERS` or `CODEOWNERS` if present — match file
   patterns to entries.
3. Combine blame-based ownership with CODEOWNERS matches.

**Files:** `src/tools.json` — add `"ownership"` to `evaluate_quality`
analysis enum. New `src/analysis/ownership.rs`. Update dispatch in
`src/service/agent_v1/quality/mod.rs`.

**Test:** New `tests/ownership.rs` — create a git repo fixture with multiple
commits by different authors, call `ownership`, assert `primary_author` and
`author_share` are correct.

---

### T10 — Security hardening (P20)

**Goal:** Prevent symlink escape and enforce max_files.

**Files:**
1. `src/indexing/code_index.rs` — in `index_workspace` (line 51, CC 22),
   add a symlink check before processing each file: `if path.is_symlink()
   && !path.canonicalize()?.starts_with(&root_canonical) { skip }`.
2. `src/config/defaults.rs` — `DEFAULT_MAX_FOLDER_FILES` is already 0
   (unlimited). Change to a sane default like `50_000` and enforce it in
   `index_workspace` with an early return + warning.
3. `src/service/agent_v1/ingest.rs` — validate that `path` doesn't escape
   the workspace root via `..` traversal.

**Test:** `cargo nextest run -E 'test(symlink)' -E 'test(max_files)'` — add
new test cases in `tests/index_file.rs` for symlink escape and file limit.

---

## Dogfooding fallbacks

When the agent reaches for `Grep`/`Read`/`Glob` instead of codesight, log
it here with the reason.

**2026-04-13.** Zero fallbacks across two sessions. All retrieval via codesight.
Batch `read_content` (5 symbols, one call) was highest leverage. Stale-index
false positive on `agent_readiness.rs` fixed by re-ingest — lesson: always
ingest at session start with uncommitted changes. `quality_signal` is our
strongest differentiator vs prior-art. ~3.7M tokens saved this session (254M
cumulative). **Bug found:** proposed building 4 features that already exist
(watch mode, single-file reindex, class hierarchy, semantic embeddings) because
I didn't `search_catalog` before writing the roadmap. Codesight found them
instantly when queried — the failure was mine, not the tool's.

**2026-04-13 session 3.** Implemented T3 (kill workspace_digest, -94 LoC),
T4 (rename churn_rate/extraction_candidates/coupling_metrics), T9 (crate
graph CI gate). Codesight used for all discovery: `search_catalog regex` to
map every reference before renaming, `natural_language` to verify dispatch
loop, `fuzzy` to find MetaBuilder from typo. Zero fallbacks.
**Friction:** `read_content` tip says "use Read tool if you plan to edit" —
this is correct but forces a double-read (codesight for discovery, Read for
harness tracking). ~20M tokens saved this session (267M cumulative).

**2026-04-15.** Retrieval fallback for the crate-graph audit because the
codesight MCP tool surface was not available in this Codex harness. Used shell
search (`rg`, `cargo metadata`) to audit workspace edges and documentation
drift, then updated the policy docs and invariants test in the same patch.
Product gap: add a first-class workspace-graph / crate-coupling view so this
audit can be dogfooded instead of reconstructed from Cargo metadata.

---

## Slow-test watch list

| Test | Binary | Time | Plan |
|---|---|---|---|
| *(audit pending)* | | | |
