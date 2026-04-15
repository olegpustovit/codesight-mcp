# EXCEL_PLAN.md

Performance & ergonomics improvements for `codesight-mcp`, derived from real
dogfooding friction (2026-04-13/14 sessions) and hardened with external
research on tokenizer crates (pure-Rust `tiktoken`), rust-analyzer's VFS
tombstoning pattern, and the known Claude Code file-state tracking
limitation.

This file is a **single-shot implementation plan**. Each item is independently
shippable, has exact file paths, API shapes, and test commands.

---

## Scope, non-goals, and coordination

- **In scope:** retrieval ergonomics (envelope fields), index freshness
  (delete/rename detection), richer quality signal, rename-aware context
  assembly, harness-friction mitigations, session-level observability.
- **Out of scope (explicit):** `src/retrieval/semantic.rs`, the embeddings
  pipeline, any vector-store work — a parallel agent is implementing proper
  semantic search. **Do not touch** files under `src/retrieval/` except
  through the stable interfaces already re-exported from
  `src/retrieval/mod.rs`.
- **Out of scope (deferred):** T1 (tiktoken), T5 (ToolEnvelope), T6
  (plan_turn), T7 (audit_config), T8 (ownership), T10 (security hardening)
  all stay in `REFACTOR_WORKLOG.md`. This plan is additive, not a replacement.
- **Workspace dependency invariants still hold.** Any new code stays in the
  crate it belongs to unless a new crate creates a cleaner domain boundary
  without introducing upward or horizontal coupling.

## Guiding principles (non-negotiable)

1. **No net LoC growth** on the product crate. If an item adds >30 LoC, it
   must either delete something equivalent or move behavior into a crate.
2. **Tests-first-or-same-patch.** Every item ships with a targeted
   `cargo nextest run -E 'test(...)'` invocation that proves the new
   behavior. The `tests/envelope_shape.rs` invariant (BASELINE_META_KEYS)
   must keep passing throughout.
3. **Stateless-per-request.** No item introduces per-conversation server
   state. Session-level aggregates (E10) live in the savings ledger file,
   not in RAM.
4. **Contract-first.** Every item that changes tool I/O updates
   `src/tools.json` in the same patch.
5. **Backwards compatibility: none.** Breaking the JSON response shape is
   fine — callers re-generate from `tools.json`. We do not ship shims.

## External research that hardens this plan

| Assumption | Source | Implication |
|---|---|---|
| Pure-Rust `tiktoken` crate exists (updated March 2026) with `cl100k_base`. | [crates.io/crates/tiktoken](https://crates.io/crates/tiktoken), [anysphere/tiktoken-rs](https://github.com/anysphere/tiktoken-rs) | T1 viable when we get to it; **out of scope for this plan** — byte/4 approximation is fine for E1–E10. |
| rust-analyzer's VFS treats file deletion as "push a `Removed` change through the change pipeline, Salsa re-derives from a smaller fact set." | [rust-analyzer vfs docs](https://rust-lang.github.io/rust-analyzer/vfs/index.html) | E2's design: diff disk-set vs index-set at ingest time; delete stale rows in one SQL `DELETE WHERE path NOT IN (?...)`. No per-symbol tombstone needed — referential integrity via `ON DELETE CASCADE` already covers it. |
| `notify-debouncer-full` emits `Rename { from, to }` as a single debounced event. Our watcher already matches `EventKind::Remove(_)`; the `files_removed` counter in `reindex_files` stats confirms the store already accepts deletions. | [notify-rs docs](https://docs.rs/notify), `src/service/file_watcher.rs:105` | E2 only needs to wire the **ingest** path; the watcher path is already correct. |
| "Edit/Write tools fail with 'File has not been read yet'" is an upstream Claude Code limitation — tracked in [claude-code#4230](https://github.com/anthropics/claude-code/issues/4230), [#10123](https://github.com/anthropics/claude-code/issues/10123), [#10437](https://github.com/anthropics/claude-code/issues/10437). | Anthropic issue tracker | E7 must **mitigate** (not "fix") — we surface a `next_step_hint` in responses and document the two-tool workflow. |

---

## Item index (ordered by leverage/effort ratio)

| ID | Title | LoC | Effort | User-facing win |
|---|---|---|---|---|
| **E1** | `total_matches` in every `search_catalog` response | +20 / −0 | XS | Plan renames without a second count call |
| **E5** | `tokens_saved_per_ms` efficiency metric in `_meta` | +15 / −0 | XS | Pick the right tool when two apply |
| **E4** | `quality_signal` actionable recommendations[] | +120 / −0 | S | Quality dashboard becomes a task list |
| **E3** | `refactor_targets` whole-repo mode | +60 / −0 | S | Ask "where to refactor?" without knowing the file |
| **E7** | Harness-friction mitigations (next_step_hint + AGENTS.md) | +40 / −5 | S | Cut 30 % of redundant re-reads |
| **E2** | Incremental ingest detects deletes & renames | +90 / −0 | M | Stale ghosts vanish without full re-ingest |
| **E10** | `manage_indexes action=session_stats` | +80 / −0 | S | See per-tool savings over the session |
| **E6** | `assemble_context mode=rename_plan` | +160 / −0 | M | Executable, ranked rename work-list |
| **E9** | Bench harness: incremental deletion + rename | +40 / −0 | S | Regression gate for E2/E6 |
| **E8** | Watcher: explicit rename event propagation (folded into E2) | +15 | XS | Consistency with manual-ingest path |

Total: **≈ +640 product-crate LoC** across 10 items. Absorbed (at least)
by deletions from the worklog (T1/T5 when they land) and by the -94 from T3
already in git. Net budget stays neutral or better.

---

## E1 — `total_matches` in every `search_catalog` response

**Goal.** Every `search_catalog` response (any `output_mode`, any
`query_mode`) includes `data.total_matches: u64` — the count **before**
limit/budget truncation. Currently `items` mode returns only the clipped
slice, so an agent planning a rename has to make a second `output_mode=count`
call to gauge scale.

### Design

Add to every `items` / `files_with_matches` / `count` response body:
```json
"data": {
  "items": [...],          // as today
  "total_matches": 247,    // NEW: pre-truncation cardinality
  ...
}
```

In `_meta`, keep existing `truncated: bool`, but add `truncated_reason`:
`"limit" | "token_budget" | null`. Callers can then decide whether to raise
`limit` or `max_tokens`.

### Files to change

| File | Change |
|---|---|
| `src/service/agent_v1/catalog_search/mod.rs` (around line 148–192) | Compute total **before** the `truncate` step; thread into response via the `extra` vec as `("total_matches", json!(n))`. Replace the ad-hoc `total_count` in count mode with the same field so both modes agree. |
| `src/service/agent_v1/catalog_search/filters.rs` (or wherever the post-filter truncation lives) | Return the pre-truncation length as a second tuple element from the ranker. |
| `src/tools.json` | Add to `search_catalog` output shape description: `"total_matches": "Pre-truncation match count."` under the response doc block. |
| `tests/search_catalog_basics.rs` (new if absent) | Assert `total_matches >= items.len()` for all modes; assert `total_matches == items.len()` when `limit > matches`. |

### Code sketch

```rust
// catalog_search/mod.rs — around line 148
let pre_truncation_len = items.len();
let (items, limit_truncated) = truncate_to_limit(items, lim);
let (items, budget_truncated) = truncate_to_budget(items, max_tokens);
let truncated = limit_truncated || budget_truncated;
let truncated_reason = match (limit_truncated, budget_truncated) {
    (true, _) => Some("limit"),
    (_, true) => Some("token_budget"),
    _ => None,
};
extra.push(("total_matches".into(), json!(pre_truncation_len)));
if let Some(r) = truncated_reason {
    extra.push(("truncated_reason".into(), json!(r)));
}
```

### Test

```bash
cargo nextest run -E 'test(search_catalog) + test(catalog)'
```

New assertions in `tests/search_catalog_basics.rs`:
- `limit=3, corpus=10` → `items.len()==3 && total_matches==10 && truncated_reason=="limit"`
- `limit=100, corpus=10` → `items.len()==10 && total_matches==10 && truncated==false`

### Risks / rollback

- Risk: pre-truncation materializes a large Vec before clipping. Current
  ranker already does this (we sort before limit), so no extra allocations.
- Rollback: one-line revert of the `extra.push` lines.

### LoC estimate

+20 product, +35 test.

---

## E5 — `tokens_saved_per_ms` efficiency metric

**Goal.** Expose `_meta.tokens_saved_per_ms` so agents can compare tool paths
objectively (e.g., `search_catalog natural_language` vs.
`inspect_graph centrality` for "what are the central files?").

### Design

Computed in `MetaBuilder::finish` — one line:

```rust
let timing_ms = self.started_at.elapsed().as_secs_f64() * 1000.0;
let tokens_saved_per_ms = if timing_ms > 0.0 {
    (tokens_saved as f64) / timing_ms
} else {
    0.0
};
```

Added to `Meta` struct with `#[serde(skip_serializing_if = "f64::is_nan_or_zero")]`
wait — no, always serialize, zero is a valid datapoint.

### Files to change

| File | Change |
|---|---|
| `src/service/savings.rs` | Add field `tokens_saved_per_ms: f64` to `Meta`; compute in `finish`. |
| `tests/envelope_shape.rs` | Add `"tokens_saved_per_ms"` to `BASELINE_META_KEYS`. |

### Test

```bash
cargo nextest run --test envelope_shape
```

Assert the new key is present and is finite, non-negative.

### Risks / rollback

- Noise for fast-and-tiny responses (dividing by 0.1 ms gives huge ratios).
  Mitigation: floor `timing_ms` at `0.1` to stabilize.

### LoC estimate

+15 product, +5 test.

---

## E4 — `quality_signal` actionable `recommendations[]`

**Goal.** Turn `evaluate_quality analysis=quality_signal` from a dashboard
into a task list. Each low sub-metric yields a structured recommendation
with a concrete first step.

### Design

Extend the existing `quality_signal` payload (in `src/analysis/quality_signal.rs`
per the worklog hotspots table — actually check the path first; see Files
below) with:

```json
{
  "agent_dimensions": { ... },           // as today
  "agent_submetrics": { ... },           // as today
  "recommendations": [
    {
      "dimension": "change_safety",
      "sub_metric": "change_locality",
      "sub_metric_value": 0.38,
      "severity": "high",                  // low|medium|high
      "suggestion": "Split god-functions: blast_radius.rs::evaluate_impact_blast_radius (CC 32) and context.rs::assemble_context (CC 24).",
      "first_step": {
        "tool": "evaluate_quality",
        "args": { "analysis": "refactor_targets", "file_path": "src/service/agent_v1/impact/blast_radius.rs" }
      },
      "worklog_hint": "T5 (ToolEnvelope) or extract-helpers in blast_radius.rs"
    }
  ]
}
```

A **rule table** drives the mapping. One row per `(sub_metric, threshold)`
pair:

```rust
// src/analysis/quality_signal/recommendations.rs  (new file)
struct Rule {
    sub_metric: &'static str,
    dimension: &'static str,
    low_threshold: f64,
    severity_of: fn(f64) -> Severity,
    build_suggestion: fn(&Snapshot, &IndexStore) -> Recommendation,
}

const RULES: &[Rule] = &[
    Rule { sub_metric: "change_locality", dimension: "change_safety",
           low_threshold: 0.6, severity_of: threshold_bands,
           build_suggestion: suggest_split_god_functions },
    Rule { sub_metric: "symbol_uniqueness", dimension: "navigability",
           low_threshold: 0.7, severity_of: threshold_bands,
           build_suggestion: suggest_rename_ambiguous },
    Rule { sub_metric: "test_oracle_quality", dimension: "verifiability",
           low_threshold: 0.4, severity_of: threshold_bands,
           build_suggestion: suggest_strengthen_test_oracles },
    // ...one row per sub_metric in agent_submetrics.
];
```

Each `build_suggestion` re-uses the existing `hotspots` / `dead_code` /
`refactor_targets` analyses internally (without shelling back out through
MCP) to produce a concrete ref.

### Files to change

| File | Change |
|---|---|
| `src/analysis/quality_signal.rs` (or `quality_signal/mod.rs` — see the grep below) | Add `recommendations` to the payload. |
| `src/analysis/quality_signal/recommendations.rs` (new) | Rule table + suggestion builders. |
| `src/service/agent_v1/quality/mod.rs` | Forward the field — probably no change if `quality_signal_payload` returns a `Value` that's passed through. |
| `tests/quality_signal.rs` | Assert that on this repo (CC > 20 functions exist), at least one `high` severity recommendation is present. |

Verify path with:
```bash
```
(use codesight, not a blind edit — `search_catalog query=quality_signal_payload query_mode=keyword`).

### Code sketch

```rust
pub(crate) fn build_recommendations(
    dims: &Dimensions,
    subs: &SubMetrics,
    store: &IndexStore,
) -> Vec<Value> {
    let mut recs = Vec::new();
    for rule in RULES {
        let value = subs.get(rule.sub_metric).copied().unwrap_or(1.0);
        if value < rule.low_threshold {
            let sev = (rule.severity_of)(value);
            recs.push(serde_json::to_value(
                (rule.build_suggestion)(&Snapshot { value, dims, subs }, store)
                    .with_severity(sev)
            ).unwrap());
        }
    }
    recs.sort_by_key(|r| std::cmp::Reverse(severity_rank(r)));
    recs
}
```

### Test

```bash
cargo nextest run --test quality_signal
```

- Given this repo (known weak `change_locality ≈ 0.38`), assert
  `recommendations[0].severity == "high"` and the first_step's `tool` is
  `"evaluate_quality"`.
- Property test: for each rule, inject a fake snapshot with
  `sub_metric = threshold - ε` and assert the rule fires.

### Risks / rollback

- Rule-table drift vs. reality. Mitigation: rules live next to the metric
  they judge; adding a new sub-metric is a checked const array insertion
  (compiler errors if the match arm in `build_suggestion` is missing).

### LoC estimate

+120 product (rule table + 6 suggestion builders), +60 test.

---

## E3 — `refactor_targets` whole-repo mode

**Goal.** Make `file_path` optional on `evaluate_quality analysis=refactor_targets`.
When omitted, scan the top-N most complex files (via existing `hotspots_payload`)
and aggregate candidates across them. Currently agents have to run
`hotspots` → pick a file → `refactor_targets` — three round-trips for one
question.

### Design

Input (backwards-incompatible shape — no compat shim):

```json
{
  "source_id": "...",
  "analysis": "refactor_targets",
  "file_path": "optional",                // NEW: optional
  "top_files": 10,                          // NEW: defaults to 10; ignored if file_path set
  "min_complexity": 0,
  "min_callers": 0,
  "top_n": 25                               // per-file cap; also caps aggregate when repo-wide
}
```

Output when `file_path` omitted:

```json
{
  "analysis": "refactor_targets",
  "scope": "repository",
  "files_scanned": 10,
  "candidates": [
    { "id": "...", "file": "src/service/agent_v1/impact/blast_radius.rs",
      "name": "evaluate_impact_blast_radius", "cyclomatic": 32,
      "caller_count": 4, "score": 160.0 }
  ]
}
```

### Files to change

| File | Change |
|---|---|
| `src/service/agent_v1/quality_more.rs::quality_refactor_targets` | Branch on `args.file_path.is_some()`; in repo mode, call `hotspots_payload` with `top_n = top_files`, extract file paths, loop per-file reusing the existing candidate logic, merge. |
| `src/service/agent_v1/args.rs::EvaluateQualityArgs` | Add `top_files: Option<usize>`. |
| `src/tools.json` | Make `file_path` non-required; add `top_files` to the property list; update `refactor_targets` description. |
| `tests/refactor_targets.rs` | New tests `refactor_targets_repo_mode_*`: assert `scope == "repository"`, candidates span multiple files, result is score-sorted globally. |

### Code sketch

```rust
fn quality_refactor_targets(
    &self, args: &EvaluateQualityArgs, timer: &MetaBuilder,
) -> anyhow::Result<Value> {
    let store = require_store!(...);
    let raw_bytes = store.repo_total_bytes()?;
    let timer = MetaBuilder::new(raw_bytes);

    let files: Vec<String> = if let Some(fp) = args.file_path.as_deref() {
        vec![resolve_file(&store, fp)?]
    } else {
        let top_files = args.top_files.unwrap_or(10);
        hottest_files(&store, top_files)?   // ≈ hotspots_payload rebound
    };

    let mut candidates = Vec::new();
    for file in &files {
        candidates.extend(candidates_for_file(&store, file,
            args.min_complexity.unwrap_or(0),
            args.min_callers.unwrap_or(0))?);
    }
    candidates.sort_by(|a, b| { /* desc by score */ });
    candidates.truncate(args.top_n.unwrap_or(25));

    self.success_response(&timer, json!({
        "analysis": "refactor_targets",
        "scope": if args.file_path.is_some() { "file" } else { "repository" },
        "files_scanned": files.len(),
        "candidates": candidates,
    }))
}
```

### Test

```bash
cargo nextest run --test refactor_targets
```

Add:
- `refactor_targets_repo_mode_returns_cross_file_candidates` — index a
  fixture with 3 complex files, assert candidates span all 3.
- `refactor_targets_repo_mode_respects_top_n` — set `top_n=5`, assert
  `candidates.len() == 5` when >5 exist.

### Risks / rollback

- Without `file_path` the response can be large (10 files × avg 5 candidates
  = 50 rows × ~200 bytes). `top_n=25` cap + existing `max_tokens` budget
  keep it bounded.

### LoC estimate

+60 product, +40 test.

---

## E7 — Harness-friction mitigations

**Goal.** Reduce the double-read tax. We cannot fix the Claude Code harness
(confirmed upstream limitation — issues #4230, #10123, #10437), but we can:

1. **Signal edit intent.** When a caller pairs `read_content` with a
   `verify=true` flag or `include=["body","hash"]`, add
   `_meta.next_step_hint` recommending an explicit `Read` before `Edit`.
2. **Guarantee byte-for-byte equivalence.** `read_content` currently returns
   `content` with leading/trailing whitespace preserved and no BOM stripping
   — verify once with a test so agents can trust `read_content` output is
   identical to `Read` output for edit planning.
3. **Document.** AGENTS.md gets a "Two-tool edit pattern" subsection under
   "Effective use guidelines".

### Design

#### 1. `_meta.next_step_hint` field

```rust
// in read_content handler
if args.verify || args.include.iter().any(|s| s == "hash") {
    extra.push(("next_step_hint".into(), json!({
        "tool": "Read",
        "reason": "Claude Code's Edit/Write tools require a prior Read-tool call on the same file. \
                   Call Read(path) before editing; read_content alone is not registered with the harness.",
        "docs": "https://github.com/anthropics/claude-code/issues/4230"
    })));
}
```

#### 2. Byte-equivalence test

`tests/read_content_byte_parity.rs` (new) — index a fixture file, call
`read_content`, compare against `std::fs::read_to_string` of the same file,
assert exact equality (no BOM strip, no newline normalization).

#### 3. AGENTS.md section

Insert under "Effective use guidelines" after the bullet about batch
`read_content`:

```markdown
- **Two-tool edit pattern.** `read_content` is for discovery and planning.
  **Before you `Edit` or `Write` a file, call the harness's `Read` tool on
  it.** The Claude Code harness tracks read-state for Edit/Write separately
  from codesight and will reject edits on files it hasn't seen via `Read`.
  See upstream issue [claude-code#4230](https://github.com/anthropics/claude-code/issues/4230).
  This is a harness limitation, not a codesight bug.
```

### Files to change

| File | Change |
|---|---|
| `src/service/agent_v1/content.rs` | Conditionally emit `next_step_hint` on verify / hash include. |
| `AGENTS.md` | Add subsection. |
| `tests/read_content_byte_parity.rs` (new) | Byte-for-byte assertion. |

### Test

```bash
cargo nextest run --test read_content_byte_parity -E 'test(read_content)'
```

### Risks / rollback

- Adds ~60 bytes to every `verify`-enabled response. Benign, agent-facing
  guidance. If it adds noise, gate behind an env var.

### LoC estimate

+40 product + doc, +30 test.

---

## E2 — Incremental ingest detects deletes and renames

**Goal.** `ingest_sources incremental=true` diffs the disk file-set against
the index file-set and removes stale rows. Solves the "I renamed
`churn_rate.rs` → `change_velocity.rs` and the old file stayed in the
index" problem I hit in session 3.

### Design (follows rust-analyzer VFS pattern)

Pipeline at `src/indexing/code_index.rs::ingest_workspace` (or the
incremental-ingest entrypoint — resolve with codesight):

```
1. Walk disk → disk_set: HashSet<String>  (repo-relative paths)
2. Query index → index_set = store.all_file_paths()?.into_iter().collect()
3. Additions   = disk_set - index_set   → parse + insert as today
4. Modifications (fingerprint mismatch) → as today
5. Deletions   = index_set - disk_set   → store.delete_files(&deletions)?
   - must be a single SQL DELETE in a transaction
   - relies on foreign-key ON DELETE CASCADE for symbols/references/docs
```

Return shape gains:

```json
{
  "files_added": 5,
  "files_modified": 12,
  "files_removed": 3,       // NEW (was 0 on ingest path)
  "symbols_added": 142,
  "symbols_removed": 41,    // NEW
  ...
}
```

### Files to change

| File | Change |
|---|---|
| `src/indexing/code_index.rs::ingest_workspace` | After the walk, compute set-diff and call `store.delete_files(deletions)`. |
| `crates/codesight-storage/src/storage/index_writes.rs` (or wherever the existing `reindex_files` removal is — grep for `files_removed` counter) | Export `delete_files(&[&str]) -> Result<Removed>` if not public; verify FK cascade covers `symbols`, `refs`, `imports`, `doc_sections`. |
| `src/service/agent_v1/ingest.rs` | Forward the new counters into the response. |
| `crates/codesight-storage/src/storage/migrations.rs` | Sanity check: all FK relationships on `files.path` use `ON DELETE CASCADE`. If any lack it, add a forward-only migration (this crate has no backwards compat). |
| `tests/ingest_deletion.rs` (new) | Index a 3-file fixture, delete one file on disk, re-ingest incremental, assert `files_removed == 1` and the deleted file is no longer in `all_file_paths()`. |
| `tests/ingest_rename.rs` (new) | Rename `a.rs → b.rs` on disk, re-ingest, assert `all_file_paths()` contains `b.rs` but not `a.rs`, and symbols from `a.rs` are gone. |

### Code sketch

```rust
// src/indexing/code_index.rs
let disk_set: HashSet<String> = walk_files(root)?.into_iter()
    .map(rel_path_string).collect();
let index_set: HashSet<String> = store.all_file_paths()?.into_iter().collect();

let to_delete: Vec<&str> = index_set.difference(&disk_set).map(String::as_str).collect();
let Removed { files, symbols } = if !to_delete.is_empty() {
    store.delete_files(&to_delete)?
} else {
    Removed::default()
};

stats.files_removed = files;
stats.symbols_removed = symbols;
```

### Rename handling

Notify-rs distinguishes `Rename { from, to }` in debounced events ([notify-rs docs](https://docs.rs/notify)).
In the **watcher** path, `collect_changed_paths` currently folds every
`EventKind::Remove(_)` and `EventKind::Modify(_)` into a single changed-paths
Vec. Because `reindex_files` already tracks `files_removed`, the watcher
path is **already correct for renames** — the `from` path is seen as Remove
and the `to` path as Create, both queued in the same debounced batch.

The **only** gap is the ingest path (no watcher, just a user-triggered
`ingest_sources` call). E2 closes that gap. No change needed to the watcher
— but E8 (below) adds a test that asserts the watcher behavior explicitly.

### Test

```bash
cargo nextest run --test ingest_deletion --test ingest_rename
```

### Risks / rollback

- **Data loss** if a bug makes `disk_set` empty (e.g., walker returns 0
  files under a bad glob) and we then delete the entire index. Mitigation:
  **hard guard**:
  ```rust
  if disk_set.is_empty() && !index_set.is_empty() {
      bail!("refusing to delete all indexed files — disk walk returned empty");
  }
  ```
  And a test that triggers this guard.
- FK cascade assumption: verify with `PRAGMA foreign_keys` turned on in the
  migration, and add an assertion in the test that symbols really disappear.

### LoC estimate

+90 product, +70 test.

---

## E10 — `manage_indexes action=session_stats`

**Goal.** Return per-tool aggregated tokens saved, calls made, and
efficiency (tokens / ms) since binary start. Lets you grade a session
without grepping `_savings.json`.

### Design

New handler arm on `manage_indexes`:

```json
{
  "action": "session_stats"
}
```

Output:
```json
{
  "action": "session_stats",
  "process_started_at": "2026-04-14T10:12:33Z",
  "uptime_ms": 438293,
  "total_tokens_saved_this_session": 3721000,
  "per_tool": [
    { "tool": "search_catalog", "calls": 42, "tokens_saved": 2_800_000,
      "avg_timing_ms": 1.4, "avg_tokens_saved_per_ms": 2_000_000 },
    { "tool": "read_content", "calls": 11, "tokens_saved": 340_000, ... }
  ]
}
```

### Storage

Process-local only. Add:

```rust
// src/service/session_stats.rs (new)
pub struct SessionStats {
    started_at: Instant,
    per_tool: DashMap<&'static str, ToolStats>,
}
#[derive(Default)]
pub struct ToolStats { pub calls: u64, pub tokens_saved: u64, pub timing_ms_sum: f64 }
```

Record from the dispatch wrapper in `src/service/agent_v1/registry.rs` —
one line per tool exit. **No** `_savings.json` write on each call (that's
already handled by the ledger); this is an in-memory aggregate only.

### Files to change

| File | Change |
|---|---|
| `src/service/session_stats.rs` (new) | Struct + thread-safe accumulate. Use `parking_lot::Mutex<HashMap>` or `dashmap = "6"` (already a dep? verify). |
| `src/service/mod.rs::CodeService` | Hold an `Arc<SessionStats>`. |
| `src/service/agent_v1/registry.rs::dispatch_tool` | After handler returns, update stats from `_meta`. |
| `src/service/agent_v1/index_admin.rs` | Add `"session_stats"` match arm; return serialized stats. |
| `src/tools.json` | Add to `manage_indexes` action enum. |
| `tests/session_stats.rs` (new) | Dispatch a known mix of tools, assert per-tool counters match. |

### Test

```bash
cargo nextest run --test session_stats
```

### Risks / rollback

- Concurrency: a `DashMap` keyed by `&'static str` tool-name is cheap.
  Alternatively, `parking_lot::Mutex<HashMap<&'static str, ToolStats>>` —
  under contention the dispatch path is already single-digit-ms, so even
  a naive mutex is fine.
- No persistence across restarts — call this out in the response so agents
  don't confuse it with the lifetime ledger.

### LoC estimate

+80 product, +40 test.

---

## E6 — `assemble_context mode=rename_plan`

**Goal.** Given a symbol and a proposed `new_name`, return an executable,
ranked plan for the rename — grouped by file, with refs and token estimates
— so an agent can edit in one pass. Replaces the current workflow of
`search_catalog regex` → manual sort → per-file `read_content`.

### Design

```json
// input
{
  "source_id": "...",
  "mode": "rename_plan",
  "refs": [{ "kind": "symbol", "stable_id": "...::OldName#function",
             "new_name": "NewName" }],
  "max_tokens": 8000
}

// output
{
  "mode": "rename_plan",
  "symbol": { "id": "...", "old_name": "OldName", "new_name": "NewName" },
  "files": [
    {
      "path": "src/service/.../foo.rs",
      "edits": [
        { "line": 12, "kind": "definition", "context": "pub fn OldName(..." },
        { "line": 45, "kind": "call_site", "context": "OldName(x)" }
      ],
      "estimated_tokens_to_read": 420,
      "risk": "low"        // low|medium|high — medium if macro-expanded, high if dyn dispatch
    }
  ],
  "total_files": 4,
  "total_edits": 11,
  "strategy": "reference_graph + text_scan",
  "warnings": []           // e.g., "Name collision with NewName in bar.rs"
}
```

### Implementation

Reuses three existing pieces:
1. `impact::rename_safety_payload` for collision warnings.
2. `inspect_graph graph=references` for the reference graph.
3. A `string_scan` fallback (regex on file text) for matches the graph
   doesn't catch (macros, string literals in tests, doc comments).

Files come back **ordered**: definition file first, then callers ranked by
call count desc, then other occurrences.

### Files to change

| File | Change |
|---|---|
| `src/service/agent_v1/context_tail/rename_plan.rs` (new) | ~120 LoC handler. |
| `src/service/agent_v1/context_tail/mod.rs` | Add match arm; add `mod rename_plan;`. |
| `src/tools.json` | Add `"rename_plan"` to `assemble_context` mode enum; document `refs[0].new_name`. |
| `tests/rename_plan.rs` (new) | Index a fixture, rename `foo`→`bar`, assert every call site in the fixture is represented and definition file is `files[0]`. |

### Test

```bash
cargo nextest run --test rename_plan -E 'test(rename)'
```

### Risks / rollback

- False negatives from the string scan (e.g., `OldName` appearing as a
  substring of `OldNameHelper`). Mitigation: boundary-aware regex
  `\bOldName\b`; a test asserts the Helper variant is **not** returned.
- Output size for wide-blast renames. Existing `max_tokens` applies to
  `files[*].edits`; truncate edits per-file and surface `truncated_in_file`
  counter.

### LoC estimate

+160 product, +80 test.

---

## E9 — Bench harness: deletion + rename cases

**Goal.** `scripts/bench-hotpaths` gets two new cases so E2/E6 can't
regress without being caught.

### Cases

1. `incremental_delete` — on a cold-ingested fixture (413 files), delete
   50 files on disk, time `ingest_sources incremental=true`. Target:
   **< 150 ms** for 50 deletions.
2. `incremental_rename` — rename 20 files, time delta. Target: **< 250 ms**.
3. `rename_plan_small` — run E6 rename_plan over a 5-file fixture. Target:
   **< 30 ms**.

### Files to change

| File | Change |
|---|---|
| `scripts/bench-hotpaths` | Add the three cases; shell out to `cargo run --release --bin codesight-mcp -- bench --case <name>` or extend in-process. |
| `src/cli/bench.rs` (if the binary exists; else create) | New cases. |
| `REFACTOR_WORKLOG.md` current-metrics table | Add three new rows. |

### Test

Benchmarks aren't run on every test invocation. They're part of the
`check --bench` CLI command and run manually after perf-sensitive patches.

### LoC estimate

+40 script/CLI, +0 test.

---

## E8 — Watcher rename-event parity (folded into E2)

**Goal.** Prove the watcher path emits the same delta as the ingest path
for rename/delete. Prevents the two paths from drifting.

### Test

`tests/watcher_rename_parity.rs` (new, integration-style):
- Start watcher on a tempdir, write 3 files, wait for initial index.
- Rename one file.
- Assert the watcher's delta (observed via watch status
  `manage_indexes action=status`) matches what `ingest_sources incremental=true`
  would produce on the same tempdir post-rename.

### LoC estimate

+15 test.

---

## Rollout / sequencing

Recommended merge order (lowest risk → highest):

1. **E5** (efficiency metric) — trivial, touches only `Meta`.
2. **E1** (total_matches) — envelope-level, no semantics.
3. **E7** (harness hint) — additive `_meta` field only.
4. **E3** (refactor_targets repo mode) — additive arg.
5. **E4** (recommendations) — additive response field.
6. **E10** (session_stats) — new handler arm, zero risk to existing tools.
7. **E2 + E8 + E9** (deletion detection) — ship together; highest code-path
   risk. Land behind a 1-session dogfood gate before closing E2.
8. **E6** (rename_plan) — builds on E2 being reliable.

---

## Acceptance criteria (session-end gate)

A session that ships any subset of E1–E10 must prove:

- [ ] `cargo nextest run` passes full suite in ≤ 30 s.
- [ ] `cargo clippy --all-targets -- -D warnings` clean.
- [ ] `tests/crate_graph.rs` still passes (T9 gate from session 3).
- [ ] `tests/envelope_shape.rs` passes — any new `_meta` keys were added
      to `BASELINE_META_KEYS` in the same patch.
- [ ] `_savings.json` cumulative `total_tokens_saved` strictly increases
      after dogfooding the session with `codesight-mcp check --savings`.
- [ ] `REFACTOR_WORKLOG.md` Current metrics table is updated.
- [ ] `REFACTOR_WORKLOG.md` "Completed this session" section is updated.
- [ ] For E2: a manual smoke — `rm` a file from the repo, run
      `ingest_sources incremental=true`, confirm `files_removed: 1`.

---

## Risks at a glance

| Risk | Item | Mitigation |
|---|---|---|
| Deleting the whole index from an empty walk | E2 | Hard guard: refuse to delete all. Test for it. |
| Rule-table drift in quality_signal | E4 | Rules colocated with the metric; compiler-checked match arm. |
| Double-truncation bug when both limit and budget fire | E1 | `truncated_reason` orders them; test both together. |
| Session stats RAM grows | E10 | Tool count is bounded by `tools.json` (≤ 15 keys). HashMap tiny. |
| `next_step_hint` noise | E7 | Gate behind `verify`/hash include flags — opt-in path. |
| FK cascade missing on some tables | E2 | Audit `migrations.rs`; add forward-only migration if missing. |

---

## Non-goals — explicit exclusions

- **Semantic embeddings.** A parallel agent owns `src/retrieval/semantic.rs`
  and the embeddings indexing path. Do not touch.
- **Tokenizer swap.** Stays in worklog as T1. Byte/4 is good enough for
  E1–E10 budgets.
- **Tool envelope refactor.** Stays in worklog as T5. When T5 lands, E1's
  `total_matches` and E5's `tokens_saved_per_ms` move onto the `Meta`
  struct automatically — no rework needed.
- **Plan_turn, audit_config, ownership.** Already specced (T6/T7/T8),
  implement separately.
- **Security hardening.** T10 stays separate.

---

## Sources (external)

- [crates.io: tiktoken](https://crates.io/crates/tiktoken) — pure-Rust BPE, March 2026 update
- [crates.io: tiktoken-rs](https://crates.io/crates/tiktoken-rs) — alternate implementation
- [anysphere/tiktoken-rs GitHub](https://github.com/anysphere/tiktoken-rs) — maintained fork
- [rust-analyzer VFS documentation](https://rust-lang.github.io/rust-analyzer/vfs/index.html) — file-change / Salsa pattern
- [rust-analyzer Guide](https://rust-analyzer.github.io/book/contributing/guide.html) — incremental computation architecture
- [notify-rs Watcher docs](https://docs.rs/notify/latest/notify/trait.Watcher.html) — rename/delete event semantics
- [notify-rs GitHub](https://github.com/notify-rs/notify) — debouncer integration
- [claude-code#4230 — Write tool should auto-read](https://github.com/anthropics/claude-code/issues/4230) — harness limitation
- [claude-code#10123 — File-watch mechanism invalidates read state](https://github.com/anthropics/claude-code/issues/10123)
- [claude-code#10437 — "File has been unexpectedly modified" after Read](https://github.com/anthropics/claude-code/issues/10437)
