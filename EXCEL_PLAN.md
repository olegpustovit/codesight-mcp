# EXCEL_PLAN.md

Implementation plan refresh for `codesight-mcp`, rewritten after a live repo
audit on 2026-04-15.

This replaces the earlier draft. The previous version had the right themes, but
it drifted behind the codebase after the 2026-04-15 crate split wave. This
version is grounded in the current workspace layout, the current handler/module
ownership, and the behavior verified by targeted tests run during the audit.

Verified during this review:

- `cargo nextest run --test crate_graph` — pass
- `cargo nextest run --test quality_signal` — pass
- `cargo nextest run --test refactor_targets` — pass
- `cargo nextest run --test incremental --test envelope_shape` — pass

---

## Architecture refresh

The plan must target the code that exists today, not the pre-split layout.

Current workspace members:

- `codesight-mcp` — root product crate; owns MCP boundary, ingest orchestration,
  watcher/freshness, CLI, config/security, and the frozen `src/tools.json`
  contract.
- `codesight-core` — shared primitives only.
- `codesight-extractors` — tree-sitter and query-pack extraction.
- `codesight-storage` — SQLite schema, migrations, reads/writes, vec tables.
- `codesight-analysis` — quality, impact, graph, dead-code, test-gap,
  agent-readiness logic.
- `codesight-framework` — framework detection.
- `codesight-imports` — import extraction and package registry.
- `codesight-retrieval` — ranking, fuzzy, sections, summarizers, embedder.

Dependency direction remains:

- `codesight-core` is the leaf.
- `codesight-analysis` depends on `codesight-core` + `codesight-storage`.
- `codesight-retrieval` depends on `codesight-core` + `codesight-storage`.
- `codesight-framework` depends on `codesight-extractors`.
- `codesight-imports` depends on `codesight-core` + `codesight-extractors`.
- The root crate depends on all inner crates and owns orchestration only.

Implications for this plan:

- Anything that computes or formats quality-domain recommendations belongs in
  `crates/codesight-analysis`, not `src/analysis/**` in the root crate.
- Anything that touches ranking, section parsing, or embedder behavior must
  target `crates/codesight-retrieval/**`; the old `src/retrieval/**` paths are
  gone.
- Anything that changes tool I/O, dispatch, freshness, watcher behavior, or the
  savings envelope belongs in the root crate under `src/service/**`,
  `src/indexing/**`, or `src/tools.json`.

Preflight doc drift discovered during the audit:

- `README.md` still says "four-crate workspace".
- `REFACTOR_WORKLOG.md` currently contains a raw `=======` merge marker.

Those are not the primary purpose of this file, but they should be cleaned up
before or alongside the next docs pass so the plan, README, architecture doc,
and worklog stop disagreeing about the same repo.

---

## Gaps found in the previous draft

### 1. E1 was under-scoped

The old draft assumed `total_matches` could be derived by "count before
truncate" inside `format_output`. That is not true in the current pipeline:

- `search_catalog` caps each lexical lane before formatting.
- hybrid search uses `per_kind_cap`, so symbol/section counts are already
  clipped before `format_output`.
- the dense path currently truncates to `lim` before `format_output`.

Real `total_matches` needs dedicated count paths, not a formatter-local fix.

### 2. E2 is mostly shipped already

Delete/rename handling is no longer hypothetical:

- `src/indexing/code_index.rs::index_workspace_with_symbol_vectors` computes
  `removed_files = existing - seen_files`.
- `src/indexing/code_index.rs::reindex_files` also computes `removed_files`.
- `crates/codesight-storage/src/storage/index_store.rs::apply_index_with_deps`
  calls `delete_removed_files` before replacing touched/updated files.

The remaining work is hardening, response-shape surfacing, and explicit tests,
not inventing deletion support from scratch.

### 3. E4 and parts of E6 targeted the wrong owners

The old plan pointed at `src/analysis/quality_signal.rs` and root-owned
retrieval paths that no longer exist. After the split:

- quality/governance logic lives in `crates/codesight-analysis/src/quality_signal.rs`
- fuzzy/ranking/section code lives in `crates/codesight-retrieval/**`

### 4. E7 is partially implemented already

`read_content` already emits an unconditional `_meta.tip`:

- `src/service/agent_v1/content.rs`

So the real work is to replace the generic always-on hint with a structured,
opt-in hint that appears only when it helps.

### 5. E10 violated repo policy

The earlier draft proposed in-memory process-local `DashMap` session stats.
That conflicts with `AGENTS.md`:

- session-level aggregates must live in the savings ledger file, not in RAM

`session_stats` should therefore be backed by `src/service/savings.rs`, not a
new ad hoc runtime store.

### 6. E6 picked the wrong reference primitive

The previous draft wanted to reuse `inspect_graph graph=references`. That path
is explicitly documented as heuristic-only and searches symbol bodies via
substring logic.

For rename planning, the better base is the `find_references` machinery in:

- `src/service/agent_v1/graph_tail_b/find_references.rs`

That path scans indexed symbol bodies plus full file text and is a better fit
for whole-word rename planning.

### 7. Acceptance criteria were too broad

The prior version required full-suite `cargo nextest run` as a default gate.
That conflicts with the repo rule that tests should be targeted by default.

This plan now uses targeted nextest commands as the default proof path, and
reserves full-suite runs for merge gates or explicit requests.

## Specification / implementation planning gaps

The main planning failures were not missing ideas; they were mismatches between
the intended contract and the code that actually owns it.

- **Contract drift.** Some documentation and examples still described
  pre-split paths or outdated tool args (`local_path`, `kinds=...`) even
  though `src/tools.json` already requires `path` and `graph`.
- **Ownership drift.** The old draft assigned work to root-crate modules that
  no longer exist after the split, which would have sent implementation into
  the wrong crate.
- **Response-shape drift.** Several items named desired fields
  (`total_matches`, session stats, next-step hints) without defining where in
  the envelope they live or which handler owns the serialization.
- **Rollout drift.** The old draft defaulted to full-suite verification and a
  new bench CLI even though the repo policy is targeted `nextest` plus the
  existing `scripts/bench-hotpaths` harness.
- **Timeline drift.** Historical notes about the crate graph were being read as
  current architecture. That must be marked explicitly in docs and the worklog.

---

## Revised item map

| ID | Status after audit | Owner | What changes now |
|---|---|---|---|
| `P0` | New | docs | Sync plan assumptions with the eight-crate workspace and fix doc drift |
| `E5` | Still correct | root `src/service/savings.rs` | Add `_meta.tokens_saved_per_ms` |
| `E1` | Re-scoped | root + storage + doc store | Add real per-lane count queries and truncation metadata |
| `E4` | Re-homed | `codesight-analysis` | Add structured recommendations to governance signal JSON |
| `E3` | Still useful | root `quality_more.rs` | Make `refactor_targets` support repo-wide mode |
| `E7` | Narrowed | root `content.rs` + docs | Replace unconditional tip with structured `next_step_hint` |
| `E2` | Mostly landed | root indexer + storage | Harden delete/rename path and surface delta stats |
| `E10` | Re-designed | root `savings.rs` + `manage_indexes` | Persist per-tool session stats in ledger file |
| `E6` | Re-based | root context layer + helper extraction | Build rename work-plan on `find_references` + `rename_safety` |
| `E9` | Still useful | shell harness | Extend existing `scripts/bench-hotpaths`; do not invent a new bench CLI |

Recommended implementation order:

1. `P0`
2. `E5`
3. `E1`
4. `E4`
5. `E3`
6. `E7`
7. `E2`
8. `E10`
9. `E6`
10. `E9`

---

## P0 — plan and architecture hygiene

Before feature work, remove stale references so the plan stops pointing at
modules that no longer exist.

### Files

- `EXCEL_PLAN.md`
- `README.md`
- `REFACTOR_WORKLOG.md`
- `ARCHITECTURE.md` only if additional drift is found during the patch

### Required fixes

- Replace every reference to root-owned `src/retrieval/**` with
  `crates/codesight-retrieval/**`.
- Replace every reference to root-owned `src/analysis/**` where the code now
  lives in `crates/codesight-analysis/**`.
- Update any "four-crate workspace" language to the current eight-member graph.
- Resolve the raw merge marker in `REFACTOR_WORKLOG.md`.

No behavior change here. This is a prerequisite for the rest of the plan being
actionable.

---

## E5 — `_meta.tokens_saved_per_ms`

This item remains low-risk and correctly scoped.

### Files

- `src/service/savings.rs`
- `tests/envelope_shape.rs`

### Implementation

Add a new `Meta` field:

```rust
pub tokens_saved_per_ms: f64
```

Compute it in `MetaBuilder::finish` from the same `timing_ms` already emitted.
Use a small timing floor to avoid pathological ratios on sub-millisecond calls:

```rust
let timing_ms = self.started_at.elapsed().as_secs_f64() * 1000.0;
let denom = timing_ms.max(0.1);
let tokens_saved_per_ms = tokens_saved as f64 / denom;
```

### Test

- Extend `tests/envelope_shape.rs::BASELINE_META_KEYS`
- Assert the value is present, finite, and non-negative

Command:

```bash
cargo nextest run --test envelope_shape
```

---

## E1 — real `total_matches` and truncation metadata

This item needs a deeper change than the original draft proposed.

### Goal

Every `search_catalog` response should expose real pre-truncation counts,
regardless of `output_mode`.

Proposed response shape:

```json
{
  "data": {
    "items": [...],
    "total_matches": 247,
    "total_files_with_matches": 19,
    "truncated": true
  },
  "_meta": {
    "truncated": true,
    "truncated_by": ["limit", "token_budget"]
  }
}
```

Use `truncated_by: []` instead of a single `truncated_reason` string; both
limit and token budget can legitimately fire in the same request.

### Files

- `src/service/agent_v1/catalog_search/mod.rs`
- `src/service/agent_v1/catalog_search_files.rs`
- `src/service/code_tools.rs`
- `crates/codesight-storage/src/storage/search.rs`
- `src/storage/doc_index/query.rs`
- `src/tools.json`
- extend `tests/retrieval_tools.rs` and/or add a focused search-catalog count test

### Implementation specifics

1. Add dedicated count helpers.

In `crates/codesight-storage/src/storage/search.rs`:

- `count_symbols(query, kind, scope) -> usize`

In `src/storage/doc_index/query.rs`:

- `count_sections(query, scope) -> usize`

In `src/service/agent_v1/catalog_search_files.rs`:

- `count_code_files_text_matches(...) -> (total_matches, total_files_with_matches)`

The file-text lane is not FTS-backed today, so file counts must still walk the
tree. That is acceptable for this item because file-text search is already a
walk-based path.

2. Centralize truncation after fusion.

Remove the dense-lane `items.truncate(lim)` in
`src/service/agent_v1/catalog_search/mod.rs`; let `format_output` own the final
packing/truncation stage so the formatter sees the full fused list it is about
to report on.

3. Thread explicit lane counts.

Introduce a small local struct in `catalog_search/mod.rs`:

```rust
struct SearchCounts {
    total_matches: usize,
    total_files_with_matches: usize,
}
```

Populate it from the lane-specific count helpers before formatting.

4. Keep `count` mode additive.

For `output_mode=count`, keep the existing `count` field for readability, but
also emit `total_matches` so all modes share the same cardinality field.

### Tests

Add targeted coverage for:

- limit truncation
- token-budget truncation
- both truncations together
- count-mode agreement with `items` mode
- hybrid search where symbol + section counts are both non-zero

Command:

```bash
cargo nextest run --test retrieval_tools --test envelope_shape
```

---

## E4 — actionable `quality_signal.recommendations[]`

This should live in `codesight-analysis`, not in the root crate.

### Files

- `crates/codesight-analysis/src/quality_signal.rs`
- likely new `crates/codesight-analysis/src/quality_signal_recommendations.rs`
- `crates/codesight-analysis/src/lib.rs` only if a new module is added
- `src/service/agent_v1/quality/mod.rs` only if the pass-through needs a minor tweak
- `tests/quality_signal.rs`
- optionally `tests/repo_health.rs` if repo-health output should assert the same field

### Implementation specifics

The key leverage point is `governance_signal_json(...)` in
`crates/codesight-analysis/src/quality_signal.rs`. Add recommendations there so
both:

- `evaluate_quality analysis=quality_signal`
- `evaluate_quality analysis=repo_health`

inherit the same recommendation block from one shared formatter.

Recommended shape:

```json
"recommendations": [
  {
    "dimension": "change_safety",
    "sub_metric": "change_locality",
    "severity": "high",
    "value": 0.38,
    "suggestion": "Reduce fan-in on hot files and split high-CC functions.",
    "first_step": {
      "tool": "evaluate_quality",
      "args": {
        "analysis": "refactor_targets",
        "file_path": "src/service/agent_v1/impact/blast_radius.rs"
      }
    }
  }
]
```

Keep the rule engine crate-local. Suggested design:

- a small `Rule` table keyed by sub-metric name
- one `build_recommendations(&GovernanceSignal, &IndexStore)` helper
- use existing coverage and hotspot data instead of inventing new scans

Examples:

- `change_locality` / `hotspot_containment` can point at `refactor_targets`
- `test_oracle_quality` can point at `test_gaps`
- `symbol_uniqueness` can point at `search_catalog` over ambiguous names

### Tests

Extend `tests/quality_signal.rs` to assert:

- `recommendations` exists
- at least one recommendation is emitted on the current multi-module fixture
- `first_step.tool` is present for low-score cases

Command:

```bash
cargo nextest run --test quality_signal
```

---

## E3 — repo-wide `refactor_targets`

This is still a good item and the code seams are straightforward.

### Files

- `src/service/agent_v1/args.rs`
- `src/service/agent_v1/quality_more.rs`
- `src/tools.json`
- `tests/refactor_targets.rs`

### Implementation specifics

Make `file_path` optional and add:

```json
"top_files": 10
```

Behavior:

- if `file_path` is present, preserve current file-scoped behavior
- if absent, repo-scope mode:
  - call `codesight_analysis::quality::hotspots_payload(...)`
  - take the hottest `top_files`
  - run the existing per-file candidate extractor across each file
  - merge and globally sort by score

Emit explicit scope metadata:

```json
{
  "analysis": "refactor_targets",
  "scope": "repository",
  "files_scanned": 10,
  "candidates": [...]
}
```

### Tests

Extend `tests/refactor_targets.rs` with:

- repo mode returns `scope == "repository"`
- repo mode spans multiple files
- `top_n` still globally truncates after merge

Command:

```bash
cargo nextest run --test refactor_targets
```

---

## E7 — structured `next_step_hint` for edit workflows

This item should be smaller than the earlier draft because part of it is
already present.

### Files

- `src/service/agent_v1/content.rs`
- `AGENTS.md`
- extend `tests/incremental.rs` or add a small focused read-content test

### Implementation specifics

Current behavior:

- `read_content` always emits `_meta.tip`

Desired behavior:

- remove the unconditional generic tip
- emit a structured `_meta.next_step_hint` only when:
  - `verify == true`, or
  - `include` contains `"hash"`

Suggested shape:

```json
"next_step_hint": {
  "tool": "Read",
  "reason": "Claude Code Edit/Write tracks read state separately from codesight read_content.",
  "when": "before editing the same file in the harness"
}
```

Do not add this on every read; the point is to reduce noise, not add it.

The byte-parity work should reuse what already exists:

- `tests/incremental.rs::run_crlf_preserved_in_cached_files`

Broaden that coverage to assert exact file-body parity for a plain file read,
instead of creating a duplicate test binary.

Command:

```bash
cargo nextest run --test incremental
```

---

## E2 — ingest delete/rename hardening and delta surfacing

This item is now a hardening pass over already-landed behavior.

### Files

- `src/indexing/code_index.rs`
- `src/service/code_tools.rs`
- `src/service/args.rs`
- `src/service/agent_v1/ingest.rs`
- `src/service/file_watcher.rs`
- `crates/codesight-storage/src/storage/index_store.rs`
- `tests/incremental.rs`
- optionally a new watcher parity test if async coverage is worth the extra harness

### What already exists

- full ingest computes `removed_files`
- delta reindex computes `removed_files`
- storage deletes removed file rows before replacing touched files
- code/doc schemas already use `ON DELETE CASCADE` for file-owned rows

### Remaining work

1. Add a hard guard against accidental mass deletion.

In `src/indexing/code_index.rs`, if the disk walk returns zero candidate files
while the existing index is non-empty, bail instead of treating it as "delete
everything".

2. Surface delta stats in the ingest response.

Do not over-spec `symbols_removed` unless we actually need it. The cheap,
immediately useful fields are:

```json
"delta": {
  "files_reindexed": 12,
  "files_removed": 3
}
```

If symbol-removal counts are still desired later, add them in storage with an
explicit pre-delete `COUNT(*)` query.

3. Keep vector deletion explicit.

`delete_removed_files` currently deletes symbol vectors explicitly before row
deletion. Keep that; vec tables are not magically covered by the SQL foreign key
graph.

4. Add explicit rename coverage.

Current tests prove delete behavior (`tests/incremental.rs`) but not the rename
case directly. Add a rename case there before inventing a separate binary.

### Tests

Extend `tests/incremental.rs` with:

- rename `a.py -> b.py` and verify the old path disappears from indexed paths
- empty-walk guard test
- optionally surface `files_removed` in the ingest JSON and assert it

Command:

```bash
cargo nextest run --test incremental
```

---

## E10 — `manage_indexes action=session_stats` backed by the ledger

This item needs redesign to match repo policy.

### Files

- `src/service/savings.rs`
- `src/service/mod.rs`
- `src/service/agent_v1/index_admin.rs`
- `src/tools.json`
- new `tests/session_stats.rs`

### Required design change

Do not add a process-local `DashMap`.

Instead, extend `SavingsState` in `src/service/savings.rs` with a current
session block:

```rust
struct SavingsState {
    total_tokens_saved: u64,
    total_cost_avoided: HashMap<String, f64>,
    current_session: SessionState,
}

struct SessionState {
    started_at: String,
    per_tool: HashMap<String, ToolSessionStats>,
}
```

Suggested per-tool stats:

```rust
struct ToolSessionStats {
    calls: u64,
    tokens_saved: u64,
    timing_ms_sum: f64,
    error_count: u64,
}
```

### Plumbing

Update `CodeService::dispatch` so that after a handler returns a response with
`_meta`, the service records:

- tool name
- `tokens_saved`
- `timing_ms`
- whether `ok` was false

into the ledger file under the existing ledger mutex.

Then add `manage_indexes action=session_stats` that reads the current session
block from the ledger and returns it.

Important schema fix:

- `src/tools.json` currently requires `source_id` for all `manage_indexes`
  calls
- `session_stats` is global
- change the schema so `action` is the only required field and validate
  `source_id` per action inside the handler

Do not promise result-cache integration yet. `ResultCache` exists in
`src/server_support/result_cache.rs`, but it is not currently wired into
`CodeService` dispatch. Add it to `session_stats` only after the cache is a real
service-owned runtime dependency.

### Tests

New `tests/session_stats.rs` should:

- dispatch at least two different tools
- call `manage_indexes { "action": "session_stats" }`
- assert per-tool call counts and token totals are present

Command:

```bash
cargo nextest run --test session_stats
```

---

## E6 — `assemble_context mode=rename_plan`

This remains a strong feature, but it should be built on reusable internals,
not handler-to-handler JSON calls.

### Files

- new `src/service/agent_v1/context_tail/rename_plan.rs`
- `src/service/agent_v1/context_tail/mod.rs`
- `src/service/agent_v1/graph_tail_b/find_references.rs` or a new extracted helper next to it
- `src/service/agent_v1/impact_more.rs` only if a shared helper is extracted there
- `src/tools.json`
- new `tests/rename_plan.rs`

### Implementation specifics

1. Extract a reusable reference collector.

Do not call `inspect_graph` from inside `assemble_context`.
Instead, pull the reusable search logic out of:

- `src/service/agent_v1/graph_tail_b/find_references.rs`

into a helper that returns structured matches without building an MCP envelope.

2. Pair it with rename safety.

Use:

- `codesight_analysis::impact::rename_safety_payload`

to populate collision warnings.

3. Group and rank by file.

For each file:

- classify matches as `definition`, `symbol_body_match`, or `text_match`
- estimate read cost from `IndexStore::file_bytes(file) / BYTES_PER_TOKEN`
- sort files as:
  - definition file first
  - then files with symbol-body matches by descending match count
  - then text-only files

Suggested output:

```json
{
  "mode": "rename_plan",
  "symbol": { "id": "...", "old_name": "foo", "new_name": "bar" },
  "files": [
    {
      "path": "src/a.py",
      "estimated_tokens_to_read": 120,
      "edits": [
        { "kind": "definition", "line": 4, "preview": "def foo():" }
      ]
    }
  ],
  "warnings": [...]
}
```

4. Reuse the existing `max_tokens` contract.

Stop once the packed file list crosses the requested budget. Emit truncation
metadata rather than silently clipping.

### Tests

New `tests/rename_plan.rs` should cover:

- definition file is listed first
- helper names like `foo_helper` are not mistaken for `foo`
- rename warnings surface collisions from `rename_safety_payload`

Command:

```bash
cargo nextest run --test rename_plan
```

---

## E9 — extend the existing hot-path bench harness

The earlier draft pointed at a non-existent `src/cli/bench.rs`. The repo
already has a shell harness:

- `scripts/bench-hotpaths`

Build on that instead of adding a new CLI surface.

### Files

- `scripts/bench-hotpaths`
- `REFACTOR_WORKLOG.md`

### Cases to add

- incremental delete
- incremental rename
- rename-plan small fixture, after `E6` lands

Use the existing `codesight-mcp ingest` and `codesight-mcp search` diagnostic
CLI commands. Only add a new CLI subcommand if the shell harness becomes
unmaintainable.

This is a perf follow-up item, not a prerequisite for the behavioral work.

---

## Out of scope

- semantic-retrieval internals in `crates/codesight-retrieval/**`; a separate
  stream already owns that area
- tokenizer replacement
- envelope-wide refactor beyond the additive meta fields in this plan
- new crate splits; the current priority is making the eight-crate architecture
  coherent and well-documented, not introducing crate nine
- session-in-RAM state

---

## Acceptance criteria

For any one item or item batch in this plan:

- update `src/tools.json` in the same patch as any tool contract change
- run the smallest proving test set for the touched area
- always keep `tests/crate_graph.rs` and `tests/envelope_shape.rs` green when
  touching crate boundaries or `_meta`
- update `REFACTOR_WORKLOG.md` in the same patch as the landed behavior

Default targeted verification by item:

- `E5`: `cargo nextest run --test envelope_shape`
- `E1`: `cargo nextest run --test retrieval_tools --test envelope_shape`
- `E4`: `cargo nextest run --test quality_signal`
- `E3`: `cargo nextest run --test refactor_targets`
- `E7`: `cargo nextest run --test incremental`
- `E2`: `cargo nextest run --test incremental`
- `E10`: `cargo nextest run --test session_stats`
- `E6`: `cargo nextest run --test rename_plan`

Run the full suite only for an explicit merge gate or when requested.
