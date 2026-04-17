# Codesight-mcp: dogfood findings → concrete plan

Source session: 2026-04-17 refactor that folded `src/storage/` into
`codesight-storage` and rewired ~30 call sites across the workspace.
This document is the plan that session produced — workstreams, file-level
targets, acceptance criteria.

---

## 0. Baseline captured this session

One key up-front decision: **we do not have a before/after metric for this
work because we only looked at `_meta` per-call**. The session counters
kept climbing (`total_tokens_saved: 28.5M → 31.5M` across ~60 calls) but
there is no persisted "session started at X, ended at Y, delta = Z" record.
WS-1 fixes exactly this.

Per-call `_meta` samples from this session (concrete evidence):

| Call shape                                             | naive_tokens | response_tokens | tokens_saved |
|---                                                     |-------------:|----------------:|-------------:|
| `catalog_sources`                                      | 0            | 1,223           | 0            |
| `inspect_structure view=file_tree path=crates/…`       | 827,087      | 892             | 826,195      |
| `inspect_structure view=file_outline` (3 KB file)      | 431          | 227             | 204          |
| `inspect_structure view=file_outline` (empty module)   | 198          | 32              | 166          |
| `search_catalog query_mode=regex` — 1 hit              | 153          | 92              | 61           |
| `search_catalog query_mode=regex` — 10 hits            | 15,027       | 1,118           | 13,909       |
| `search_catalog query_mode=keyword` — broad            | 185,111      | 2,640           | 182,471      |
| `read_content refs=[9 file ranges]`                    | 1,720        | 2,727           | **0**        |

The last row is the most telling: batched peek reads *under-report* savings
because the tool defines "saved" as `naive_tokens − response_tokens` on the
single call, not against the alternative "9 separate Read calls with full
file prefixes each."

---

## 1. Findings (short form)

Kept only the ones that drive workstreams. Evidence from this session.

| # | Finding | Evidence | Workstream |
|---|---|---|---|
| F1 | Session start/end delta is not captured | `total_tokens_saved` only lives in `_meta` per-call | WS-1 |
| F2 | `_meta` envelope carries 15+ fields on every reply, most session-constant | every call in this transcript | WS-2 |
| F3 | Module-path renames have no dedicated tool; 20+ sequential Edits required | this session's core work | WS-3 |
| F4 | `evaluate_impact analysis=blast_radius` is a perfect pre-refactor fit but I forgot it exists | never called it this session | WS-4, WS-6 |
| F5 | `STALE_DATA_WARNING` is generic — doesn't name the in-flight path in per-call warnings (it does in `catalog_sources`) | `src/service/agent_v1/freshness.rs:67-82` | WS-5 |
| F6 | `search_catalog offset` does not paginate in regex mode; returned same 50 hits for `offset=0` and `offset=50` | live repro this session | WS-7 |
| F7 | `inspect_structure view=file_outline` shows `symbol_count: 0` on modules that only re-export via `pub use` | `src/config/mod.rs` outline call | WS-7 |
| F8 | Routing skill has no "session start / refactor recipe / session end" checklist | `plugins/…/codesight-routing.md` | WS-6 |
| F9 | Deferred tools require a ToolSearch handshake before use; adds ≥1 turn of latency at session start | this session | WS-6 |
| F10 | `evaluate_quality` already exposes **agent-readiness dimensions** (navigability, comprehensibility, verifiability, change_safety) and `hotspots` / `complexity` / `test_gaps` / `regression_check`, but no routing guidance tells agents *when* to call them, *what thresholds* to hold against, or *how to react* to a regressing score. The machinery exists; the agent-facing recipe doesn't. | `crates/codesight-analysis/src/agent_readiness.rs`, `src/tools.json:319`, `AGENT_READINESS_V2.md` | **WS-8** (new), WS-6 |

---

## 2. Metric model — what we track going forward

Two classes of metric:

**A. Per-call (for agent feedback loop):**

- `timing_ms` — keep, useful
- `tokens_saved` — keep, *but redefine* for batch calls (see WS-2)
- `warnings[]` — promoted to **top-level** of response, not under `_meta`

**B. Per-session (for retrospective / CI / worklog):**

- `session_id` — new, issued by `catalog_sources`
- `started_at`, `ended_at`
- `calls_by_tool` — `{tool_name: count}`
- `tokens_saved_total` — sum of per-call deltas in this session only
- `fallbacks_to_grep_or_bash` — count of hook-blocked native calls
- `evaluate_impact_usage` — `{blast_radius: N, rename_safety: M, path_rename: K, …}`
- `stale_warnings_fired_with_in_flight_paths[]` — named paths that were
  reindexing when a staleness warning fired

Exposed via **`manage_indexes action=session_stats`** (already exists at
`src/service/agent_v1/index_admin.rs:137`) — extend its payload.

Persisted to `$storage_dir/_savings.json` so `cargo run -- check --savings`
and the routing skill can surface the score without a live MCP call.

---

## 3. Workstreams

Each workstream has: **goal**, **files to change** (with line anchors where
known), **API / behavior**, **test**, **acceptance**.

---

### WS-1 — Session bookkeeping: start & end deltas

**Goal.** Replace "count numbers inside `_meta` and hope" with a concrete
session-scoped metric contract.

**Files to change.**

- `src/service/savings/` (module that owns `MetaBuilder`) — add a
  `SessionLedger` type that holds `{session_id, started_at, call_counts,
  tokens_saved_total}`, loaded/persisted to `_savings.json`.
- `src/service/agent_v1/catalog.rs:27-30` — on the first call per process,
  mint a new `session_id` (UUID v4), write start row to `_savings.json`,
  and include it in the response: `{"session": {"id": "...", "started_at":
  "..."}}` at the **top level** (not under `_meta`).
- `src/service/agent_v1/index_admin.rs:137` (`session_stats`) — extend
  response with: `{ session_id, started_at, calls_by_tool, tokens_saved_total,
  fallbacks_to_grep_or_bash, evaluate_impact_usage }`.
- New: `manage_indexes action=session_end` — writes the final row and
  returns the rollup. Optional (session can also be inferred as "the time
  between `catalog_sources` calls").
- `plugins/agentpack-bundle/skills/codesight-routing.md` — add "session
  start" + "session end" steps (also covered in WS-6).

**Behavior.** On every `catalog_sources` response:
```json
{
  "session": {"id": "sess_2026-04-17T…", "started_at": "…"},
  "data": {...}
}
```
On every other response: omit `session.*` (the agent already knows the id).

**Test.** Integration: call `catalog_sources`; call 3 more tools; call
`session_stats`. Assert `calls_by_tool.catalog_sources == 1`, other counts
match, `tokens_saved_total > 0`.

**Acceptance.** At the end of *this* session I can answer: "how many tokens
did I save during this refactor?" with one call. Right now I can't.

---

### WS-2 — Envelope slimming: bucket `_meta` by churn rate

**Goal.** Stop paying ~280 bytes × N calls for session-constant fields.

**Current `_meta` (from live samples above):**
```
_meta.{
  build_sha, build_time, build_version,          // session-constant
  powered_by, retrieval_engine, retrieval_version,// session-constant
  cost_avoided.{opus,sonnet,gpt4o},              // per-call, derived
  naive_tokens, response_tokens,                 // per-call
  timing_ms, tokens_saved, tokens_saved_per_ms,  // per-call
  total_cost_avoided.{opus,sonnet,gpt4o},        // session-rolling
  total_tokens_saved,                            // session-rolling
  // per-mode fields only if relevant
  query_mode_effective, domain_effective, fuzzy_applied,
  semantic_applied, unit_kinds_effective,
  baseline_scope, estimate_method, truncated_by
}
```

**Target shape (default `_meta_verbose=false`):**
```
_meta.{
  timing_ms: 1.23,
  tokens_saved: 5000,
  // + mode hints only when they were actually used (e.g. fuzzy_applied
  //   only shown if true). Never show fields whose value is the default.
}
```

Anything omitted from per-call `_meta` is retrievable via:
- `catalog_sources → session.{build_sha, build_time, retrieval_version, …}`
- `manage_indexes action=session_stats → tokens_saved_total, …`

**Files to change.**

- `src/service/savings/mod.rs` (where `MetaBuilder` lives) — split `build()`
  into `build_minimal()` and `build_verbose()`; the minimal form is the
  new default.
- `src/service/agent_v1/validate.rs` or `args.rs` — add `_meta_verbose:
  bool` to the base argument shape. Default false.
- `src/tools.json` — declare `_meta_verbose` as an optional bool on every
  tool. **Breaking**: old clients that read `_meta.build_sha` stop working
  unless they pass `_meta_verbose=true`. AGENTS.md §2 says no back-compat,
  so this is a clean flip.
- `src/service/agent_v1/catalog.rs` — first `catalog_sources` response
  emits the session-constant block (see WS-1).

**Token-saving estimate.** 280 bytes × 60 calls ≈ 17 KB per session. Not
huge, but free once the refactor is done, and it makes responses
human-readable again.

**Test.**
```rust
// tests/meta_envelope.rs
#[test] fn meta_default_is_minimal() {
    let r = svc.catalog_sources(args()).unwrap();
    assert!(r["_meta"]["build_sha"].is_null());
    assert!(r["session"]["build_sha"].is_string());
    assert!(r["_meta"]["timing_ms"].is_number());
}
#[test] fn meta_verbose_round_trip() {
    let r = svc.search_catalog(json!({..., "_meta_verbose": true})).unwrap();
    assert!(r["_meta"]["build_sha"].is_string());
}
```

**Acceptance.** Response body `jq '._meta | keys | length'` on a default
call returns ≤4; on `_meta_verbose=true` returns the current 15+.

---

### WS-3 — Module-path rename: new `evaluate_impact analysis=path_rename`

**Goal.** The biggest single productivity win. Replace "20 sequential
Edits to change `crate::storage::X` → `codesight_storage::storage::X`"
with one tool call that returns a dry-run diff, then a second call with
`apply=true`.

**Why it's a new `analysis` enum value, not a new tool.** AGENTS.md §2:
"no parallel tool names. Extensibility goes through `view`/`analysis` enums."
`evaluate_impact` is the existing home for change-oriented operations
(rename_safety, diff, blast_radius). Adding `analysis=path_rename` is
consistent with that pattern.

**API.**
```json
{
  "analysis": "path_rename",
  "source_id": "OlegHQ/codesight-mcp",
  "from": "crate::storage::doc_index::DocStore",
  "to":   "codesight_storage::storage::doc_index::DocStore",
  "scope": "src/",                            // optional
  "apply": false,                              // default: dry-run
  "handle_partial_group_imports": "warn"      // "warn" | "split" | "skip"
}
```

**Response (dry-run).**
```json
{
  "ok": true,
  "data": {
    "analysis": "path_rename",
    "files_affected": [
      {
        "path": "src/service/agent_v1/catalog.rs",
        "changes": [
          {"line": 11, "col": 1,
           "before": "use crate::storage::doc_index::{DocStore, list_doc_summaries};",
           "after":  "use codesight_storage::storage::doc_index::{DocStore, list_doc_summaries};",
           "node_kind": "use_declaration"}
        ]
      }
    ],
    "summary": {
      "files": 22,
      "changes": 27,
      "unhandled": [
        {"path": "src/service/X.rs", "line": 5,
         "reason": "partial match inside group import use X::{A, crate::storage::B}; run with handle_partial_group_imports=split"}
      ]
    },
    "applied": 0
  }
}
```

**Response (`apply=true`): same shape + `applied: 27`; writes land
via normal file I/O + triggers the existing PostToolUse reindex hook.**

**Implementation.**

- New module: `crates/codesight-analysis/src/impact/path_rename.rs`.
- Use the existing Rust tree-sitter grammar to walk:
  - `use_declaration` → `scoped_use_list` / `scoped_identifier`
  - `scoped_identifier` (fully-qualified paths in type, expr, trait positions)
  - `use_declaration` with aliases (`use X as Y;` — preserve alias)
- MVP **in-scope:**
  - Simple `use` prefix replace
  - Fully-qualified paths in types, expressions, and trait bounds
  - `use X::{A, B}` when the *whole group* shares the prefix
- MVP **out-of-scope** (warned, skipped):
  - Partial group imports (`use X::{A, other::B}`) — warn, let agent fix manually
  - Rewriting inside doc comments / string literals (off by default; flag for future)
  - Rewriting inside macros whose bodies are opaque
- Collision guard: refuse `apply=true` if the target path already exists
  with a conflicting definition in the same scope.

**Wiring.**
- `src/service/agent_v1/impact.rs` → dispatch `analysis=path_rename` to
  `codesight_analysis::impact::path_rename::rewrite_paths(...)`.
- `src/tools.json` → add `path_rename` to the `analysis` enum and
  document the `from` / `to` / `apply` / `scope` /
  `handle_partial_group_imports` arguments.
- `src/service/agent_v1/validate.rs` → schema.

**Tests.** Build on `tests/check_rename_safe.rs` patterns.
- `tests/path_rename.rs`
  - `renames_simple_use_statement`
  - `renames_grouped_use_when_whole_group_matches`
  - `warns_on_partial_group_match`
  - `preserves_use_aliases`
  - `rewrites_fully_qualified_type_position`
  - `refuses_apply_on_collision`
  - `apply_triggers_post_tool_reindex` (verify subsequent
    `search_catalog` sees the new paths)

**Acceptance.** Re-running this session's refactor with `path_rename`
turns step 4 ("rewrite 20+ callers") from ~30 sequential Edits into ≤2
MCP calls. Success metric: **edit-tool calls during the rename phase
drop ≥80%**.

---

### WS-4 — `evaluate_impact` steering & usage metric

**Goal.** Make blast_radius the default first move on any refactor, and
track whether agents actually do it.

**Sub-parts.**

1. **Measurement.** Extend `SessionLedger` (WS-1) to track:
   ```
   evaluate_impact_usage: {
     blast_radius: N, rename_safety: N, diff: N,
     path_rename: N, changed_units: N, impact_preview: N,
     change_velocity: N
   }
   ```
   Surface in `session_stats` and `_savings.json`.

2. **Suggestion on refactor-shaped edits.** When the PostToolUse hook sees
   an Edit that touches multiple files with the same `use` prefix, emit a
   one-line suggestion via `additionalContext`:
   > "Touching 5+ files with `use crate::X::*` — consider
   > `evaluate_impact analysis=path_rename` (or
   > `analysis=blast_radius` to preview impact)."

3. **Routing-skill nudge.** WS-6 adds a "Before a refactor" recipe.

**Files.**
- `src/server_support/savings_ledger.rs` (or wherever SessionLedger
  lands) — counters.
- `plugins/agentpack-bundle/hooks/post_tool_use.sh` — emit the
  `additionalContext` when Edit pattern matches.
- `plugins/agentpack-bundle/skills/codesight-routing.md` — add the recipe.

**Acceptance.** A session that performs a module move without calling
`evaluate_impact analysis=blast_radius` at least once emits a warning in
`session_stats.anti_patterns[]`. The routing skill surfaces the pointer
before the first relevant Edit.

---

### WS-5 — Staleness transparency: name the in-flight path

**Current gap.** `src/service/agent_v1/freshness.rs:67-82` (function
`annotate_with`) fires this warning on any non-catalog_sources response:
```json
{"code": "STALE_DATA_WARNING",
 "message": "reindex in progress; results may reflect a previous index state.",
 "stale_since_ms": 110}
```
It does **not** say which repo is being reindexed or which paths are in
flight. The `catalog_sources` path *does* (see `freshness.rs:35-55` which
attaches `indexing: [...]`).

**Fix.** In `annotate_with`, look up the specific repo being reindexed
and include: `indexing_repo`, `indexing_started_ago_ms`, and — critically —
`files_in_flight: ["src/storage/doc_index/mod.rs", ...]` (cap to first 5).

**Files.**
- `src/service/agent_v1/freshness.rs:67-85` — extend the JSON object.
- `src/server_support/reindex_state.rs` — make the in-flight file list
  readable through `get_status(repo)`.

**Also fix the placement.** Move the warning from
`response.warnings[]` to **top-level** `response.warnings[]`
(it already appears there for most tools) and ensure no tool buries it
under `_meta.warnings`. Search result `search_catalog` puts its warnings
in `_meta.warnings` (saw this live: `"warnings":[{"code":"TRUNCATED", …},
{"code":"STALE_DATA_WARNING", …}]` under `_meta`). That's the wrong place
— `_meta` gets ignored; top-level `warnings` does not.

**Acceptance.** After editing `src/storage/` in-flight, a `search_catalog`
call during the reindex window returns:
```json
"warnings": [{
  "code": "STALE_DATA_WARNING",
  "indexing_repo": "OlegHQ/codesight-mcp",
  "indexing_started_ago_ms": 110,
  "files_in_flight": ["src/storage/doc_index/mod.rs", ...]
}]
```
at the response top level, not under `_meta`.

---

### WS-6 — Routing skill & plugin guidance updates

**Goal.** Encode the session-start / refactor / session-end recipes so
future agents don't forget the tools they have.

**File.** `plugins/agentpack-bundle/skills/codesight-routing.md` (or
wherever the skill markdown lives).

**Sections to add.**

1. **Session start (before any retrieval):**
   ```
   1. catalog_sources — get source_id, session_id, session constants.
   2. manage_indexes action=session_stats — record baseline.
   ```

2. **Before a refactor (module move, rename, API change):**
   ```
   1. evaluate_impact analysis=blast_radius refs=[{...}] — preview
      which files break if this symbol changes.
   2. For cross-module renames: evaluate_impact analysis=path_rename
      from="..." to="..." apply=false — dry-run diff.
   3. If dry-run is clean: re-run with apply=true.
   4. Trust the PostToolUse hook to reindex; verify with a
      search_catalog call whose results should reflect the new paths.
   ```

3. **After edits — staleness trust rule:**
   ```
   Within the first ~1s after Edit/Write, search_catalog results may be
   stale. If warnings include `STALE_DATA_WARNING` with
   `files_in_flight` covering your edit, either:
     - wait one more retrieval call (reindex is usually <100ms)
     - or call `manage_indexes action=status` to confirm fresh
   ```

4. **Session end:**
   ```
   manage_indexes action=session_stats — record delta vs. baseline.
   Paste the delta into REFACTOR_WORKLOG.md for the next session.
   ```

5. **Update the "Default → codesight" mapping table:**
   ```
   | Native                              | Codesight                                |
   | Edit across N callers to rename mod | evaluate_impact analysis=path_rename      |
   | Edit across N callers to rename sym | evaluate_impact analysis=rename_safety    |
   | Is this file easy for me to edit?   | evaluate_quality analysis=quality_signal scope=<file> |
   | Will this edit regress quality?     | manage_indexes action=snapshot_quality + regression_check |
   | Is this code a hotspot?             | evaluate_quality analysis=hotspots min_complexity=20 |
   | Public API with no docs?            | evaluate_quality analysis=doc_coverage   |
   | Untested entry points?              | evaluate_quality analysis=test_gaps      |
   ```

6. **Deferred tools handshake (tackles F9).** If the session is starting
   and `catalog_sources` hasn't been called yet, name the exact
   `ToolSearch` pre-fetch: `ToolSearch query="select:catalog_sources,
   search_catalog,inspect_structure,read_content,evaluate_impact,
   evaluate_quality,manage_indexes,inspect_graph"` — copy-pastable,
   one call.

7. **Quality-driven editing recipe (new section).**
   ```
   Before editing <file>:
   1. evaluate_quality analysis=quality_signal scope=<file>
      Check: agent_readiness ≥ 0.70 per dimension?
      If not → read `top_offenders`, decide: refactor first, or accept the risk.

   While editing:
   - Keep per-fn cyclomatic ≤ 20 (AGENTS.md §1).
   - Don't drop any dimension's score — if you must, note in worklog.

   After a meaningful edit (every ≥3-file change):
   1. manage_indexes action=snapshot_quality snapshot_name=mid_session
   2. evaluate_quality analysis=regression_check
      (compares against the auto-captured "session_start" snapshot)
      Verdict "regressed" → stop and look at quality_delta before moving on.

   At session end:
   - manage_indexes action=session_end
      → returns quality_delta, hotspot_delta, readiness_delta.
   - Paste the delta into REFACTOR_WORKLOG.md.
   ```

8. **Thresholds table (single source of truth — mirrors WS-8a).**
   Render the table inline in the routing skill. If a future
   `evaluate_quality` response disagrees with the table, update the
   skill in the same patch — not two sources of thresholds.

9. **Composition with rust-modularity / rust-design-patterns.**
   When `quality_signal` flags `comprehensibility` low on a Rust file,
   the recipe points at `agentpack-bundle:rust-modularity` (for module
   splits) or `agentpack-bundle:rust-design-patterns` (for
   enum-vs-trait, Builder, etc.) rather than giving in-line advice.
   Codesight identifies the symptom; the sibling skills own the cure.

**Acceptance.** The skill's table-of-contents lists "Session start",
"Before a refactor", "After edits", "Session end". A cold agent reading
only the skill can complete a module move without forgetting
`evaluate_impact`.

---

### WS-8 — Agent-readiness & complexity optimization guidance

**Goal.** Turn `evaluate_quality` from "detection machinery the agent
forgets to call" into a **default loop step** with concrete thresholds,
pre/post hooks, and session-level deltas. Agents should leave the repo
with *measurably higher* agent-readiness than they found it.

**What already exists (don't rebuild).**
- `evaluate_quality analysis=quality_signal` → 4 agent-readiness
  dimensions + structural root causes. Implementation in
  `crates/codesight-analysis/src/agent_readiness.rs`.
- `evaluate_quality analysis=hotspots` → complexity + churn hotspots.
- `evaluate_quality analysis=complexity` → per-symbol cyclomatic.
- `evaluate_quality analysis=test_gaps` → untested entry points.
- `evaluate_quality analysis=doc_coverage` → public API documentation.
- `evaluate_quality analysis=regression_check` → compare two snapshots.
- `manage_indexes action=snapshot_quality` → capture a named snapshot.
- Docs at `AGENT_READINESS_V2.md`.

**What's missing.**
1. **Concrete thresholds.** Agents see scores but don't know what "good"
   is. The AGENTS.md cyclomatic>20 rule is the only number published.
2. **Pre-edit probe**: no hook or recipe that runs a quality check on a
   file *before* the agent modifies it, so the agent can decide whether
   to restructure first.
3. **Post-edit regression gate**: `regression_check` exists but isn't
   wired into the session-end flow.
4. **Per-file agent-readiness**: `quality_signal` returns a repo-wide
   rollup. Agents editing one file want that file's score and worst
   dimension, not the global mean.
5. **Session delta**: no readiness-score-before vs. readiness-score-after
   is ever reported. (Ledgered after WS-1.)

#### WS-8a — Publish the thresholds table

One canonical table, referenced from `AGENTS.md`, `CONFIGURATION.md`,
the routing skill, and every `evaluate_quality` response's
`_meta.thresholds` (only when `_meta_verbose=true`).

| Signal                               | Target        | Action when missed                            |
|---                                   |---            |---                                            |
| Cyclomatic complexity (per fn)       | ≤ 20          | Split; extract helpers                        |
| File LoC                             | ≤ 500 soft    | Only split on mixed responsibility (AGENTS.md §1) |
| `navigability`                       | ≥ 0.70        | Add `pub use` re-exports, shallow module tree |
| `comprehensibility`                  | ≥ 0.70        | Rename for clarity, shrink nesting depth      |
| `verifiability`                      | ≥ 0.70        | Add tests OR add type annotations             |
| `change_safety`                      | ≥ 0.70        | Reduce fan-in, tighten visibility (`pub(crate)`)|
| `agent_readiness_score` (geo-mean)   | ≥ 0.70        | Tackle the weakest dimension first            |
| Doc coverage (public API)            | ≥ 60%         | Document `pub fn`s without doc-comments       |
| Untested entry points (`test_gaps`)  | 0             | Add nextest case or remove `pub`              |
| `structural_score`                   | ≥ 0.60        | See `quality_signal.root_causes`              |
| Per-file blast radius (touched-by-change) | ≤ 10 files | Extract + narrow visibility                   |

Thresholds live in `crates/codesight-analysis/src/thresholds.rs` as a
single struct with env-var overrides (`CODESIGHT_THRESHOLD_*`) so
stricter repos can tighten. Default values are the numbers above.

#### WS-8b — Per-file `quality_signal scope=<path>`

`evaluate_quality analysis=quality_signal` today returns a repo-level
rollup. Extend so when `scope=<file_or_dir>` is passed, the response
returns *that file's* 4 dimensions + weakest-dimension hint + top-3
offending symbols.

Response addition:
```json
{
  "file": "src/service/agent_v1/catalog_search/mod.rs",
  "agent_readiness": {
    "score": 0.61,
    "navigability": 0.78,
    "comprehensibility": 0.52,   // ← weakest
    "verifiability": 0.68,
    "change_safety": 0.65
  },
  "weakest_dimension": "comprehensibility",
  "top_offenders": [
    {"symbol": "materialize_dense_symbol_items", "line": 28, "cyclomatic": 27, "nesting_depth": 6},
    ...
  ],
  "thresholds_failed": ["comprehensibility<0.70", "agent_readiness<0.70"]
}
```

Files:
- `crates/codesight-analysis/src/agent_readiness.rs` — add a
  `compute_for_file()` alongside the existing repo-level function.
- `src/service/agent_v1/quality*.rs` — route `scope=` through.
- `src/tools.json` — document the new `scope` parameter shape for
  `quality_signal`.

#### WS-8c — Pre-edit quality probe (optional-on, hook-driven)

Add a PreToolUse hook for Edit/Write that, **only if** the file scored
below threshold at last probe, prints a one-line `additionalContext`:
> "Editing `path.rs`: file agent_readiness=0.61 (threshold 0.70),
> weakest=comprehensibility. Consider refactor before adding code.
> Run `evaluate_quality analysis=quality_signal scope=path.rs` for detail."

Crucially: **the hook does not block.** Agents override routinely; the
hint is a "knowledge nudge", not a gate. If the file passes thresholds
the hook is silent.

Files:
- `plugins/agentpack-bundle/hooks/pre_tool_use.sh` (or equivalent) —
  add the score lookup path.
- `$storage_dir/_agent_readiness_cache.json` — cheap cache indexed by
  `file_path → (score, computed_at)` so the hook doesn't have to call
  the full `quality_signal` on every Edit. Staleness tolerance: 5 min
  or last ingest, whichever is fresher.

#### WS-8d — Session-start and session-end quality snapshots

Tie into WS-1 (session ledger).

At **session start** (first `catalog_sources` call):
- Auto-capture `snapshot_quality name="session_start"` in the
  background. Don't block the response. Record snapshot_id in the
  ledger.

At **session end** (explicit `manage_indexes action=session_end`, or
elapsed-time heuristic, or agent-initiated):
- Capture `snapshot_quality name="session_end"`.
- Run `regression_check from=session_start to=session_end`.
- Surface the delta in the session_stats response:

```json
"quality_delta": {
  "agent_readiness_score": { "before": 0.64, "after": 0.71, "delta": +0.07 },
  "per_dimension": {
    "navigability":      {"before": 0.72, "after": 0.76},
    "comprehensibility": {"before": 0.55, "after": 0.68},
    "verifiability":     {"before": 0.63, "after": 0.65},
    "change_safety":     {"before": 0.67, "after": 0.74}
  },
  "new_hotspots": [{"file": "src/foo.rs", "cyclomatic": 28}],  // regressions
  "resolved_hotspots": [{"file": "src/bar.rs"}],               // wins
  "verdict": "improved"   // "improved" | "neutral" | "regressed"
}
```

If `verdict == "regressed"`: the session_stats response emits a
top-level `warnings: [{code: "QUALITY_REGRESSION", ...}]` so the agent
notices *before* signing off.

Files:
- `src/service/agent_v1/index_admin.rs:137` — extend `session_stats`
  response.
- New: `src/service/agent_v1/quality_session.rs` — delta computation.
- `crates/codesight-analysis/src/regression_check.rs` — new module if
  not already split out.

#### WS-8e — Symbol-level complexity budget for the agent's edits

When `path_rename` (WS-3) or other large edits land, emit a
"complexity budget check" in the `apply` response:

```json
"complexity_budget": {
  "before": { "total_cyclomatic": 1_234, "max_per_fn": 27 },
  "after":  { "total_cyclomatic": 1_234, "max_per_fn": 27 },
  "verdict": "unchanged"
}
```

Ensures that a mechanical rename that accidentally inlines a fn (or
anything else sneaky) is flagged.

#### Cross-skill composition

The `agentpack-bundle:rust-modularity` and
`agentpack-bundle:rust-design-patterns` skills *already* know how to
restructure. WS-8's nudges should reference them by name:

> "File scores `comprehensibility=0.52`. The `rust-modularity` skill
> has patterns for this class of problem — invoke it if unsure how to
> split."

Routing skill entry for that mapping (added in WS-6).

#### Acceptance for WS-8

- `evaluate_quality analysis=quality_signal scope=<file>` returns a
  per-file 4-dim score + weakest-dimension hint.
- `session_stats` includes `quality_delta` with before/after
  agent-readiness and a verdict.
- A test case that artificially regresses complexity (adds a
  high-cyclomatic function) produces
  `session_stats.quality_delta.verdict == "regressed"` and a top-level
  `QUALITY_REGRESSION` warning.
- The thresholds table is one source of truth, referenced from routing
  skill, AGENTS.md, and the `evaluate_quality` tool description in
  `tools.json:319`.
- For this session (no WS-8 yet), the baseline is: **0 quality probes,
  0 snapshots, 0 regression checks, unknown agent-readiness delta.**
  Compare against the next session's rollup.

---

### WS-7 — Fix the discovered bugs

Small, local fixes. Bundle them so they ship together.

**W7a. `search_catalog offset` in regex mode.**
- Symptom: `offset=50` returns identical hits to `offset=0` when
  `query_mode=regex` and total matches > limit.
- Likely location: `src/service/agent_v1/catalog_search/mod.rs`
  (regex path). The offset is probably dropped before the file-text scan.
- Fix: apply `offset` after sorting, before `truncate(limit)`.
- Test: seed fixture with N matches across M files; assert
  `offset=0, limit=L` + `offset=L, limit=L` returns disjoint hits that
  together equal `limit=2L`.

**W7b. `inspect_structure view=file_outline` doesn't count `pub use`.**
- Symptom: `src/config/mod.rs` has 10 `pub use` re-exports, outline
  reports `symbol_count: 0`.
- Fix: in the extractor query pack for Rust, add a `use_declaration` with
  `pub` modifier → emit as `kind: "reexport"`. This lets agents see
  "this module only re-exports" at a glance without reading the file.
- Files: `runtime/queries/rust/tags.scm` (or wherever the rust pack
  lives).
- Test: `inspect_structure view=file_outline file_path=src/config/mod.rs`
  → returns ≥1 symbol with `kind: "reexport"`.

**Acceptance.** Both bugs get regression tests. Both tests currently fail.
After WS-7 both pass.

---

## 4. Phasing

Ordered by ROI for the *next* refactor session:

**Phase 1 — quick wins (1 session).**
- WS-5 (staleness transparency)
- WS-7 (two bug fixes)

**Phase 2 — biggest lever (1 session).**
- WS-3 (`path_rename`) — the single feature that would have halved this
  session's wall-clock.

**Phase 3 — measurement foundation (1 session).**
- WS-1 (session bookkeeping, ledger, session_id)
- WS-2 (envelope slimming) — depends on WS-1 for the session block.

**Phase 4 — steering & quality loop (1–2 sessions).**
- WS-4 (evaluate_impact usage metric + hook suggestion)
- **WS-8a + WS-8b** (thresholds table + per-file `quality_signal`) —
  prereq for the quality recipes.
- WS-6 (routing skill updates, session recipes, quality recipe,
  thresholds cross-reference).

**Phase 5 — quality gating (1 session).**
- **WS-8c** (pre-edit probe hook).
- **WS-8d** (session-start/end snapshots + `quality_delta` in
  `session_stats`).
- **WS-8e** (complexity budget on `path_rename apply=true`).

---

## 5. Overall acceptance

A session that repeats this exercise (moving a module between crates) in
the post-plan world should produce:

1. A `session_stats` delta at the end showing N calls, X tokens saved —
   written to `_savings.json` and `REFACTOR_WORKLOG.md`.
2. ≤2 MCP calls (one dry-run + one apply) for the cross-crate rename
   phase, replacing ~20 sequential Edit calls.
3. Zero `_meta.build_sha`/`_meta.build_time`/`_meta.retrieval_version`
   noise on routine responses.
4. When a `STALE_DATA_WARNING` fires post-edit, the warning names which
   path is being reindexed so the agent can decide whether to wait.
5. The routing skill's "Before a refactor" checklist was followed —
   `evaluate_impact analysis=blast_radius` called once before the
   rewrite, `path_rename` dry-run called before apply.
6. **Quality loop ran:** at session start a `snapshot_quality` was
   auto-captured; at session end `session_stats.quality_delta` reports
   agent-readiness before/after and a verdict. If the verdict is
   `regressed`, a top-level `QUALITY_REGRESSION` warning fired.
7. **Per-file quality was probed** at least once before a non-trivial
   edit (pre-edit quality probe hook fired OR the agent explicitly
   called `quality_signal scope=<file>`). Thresholds from WS-8a were
   the single source of truth used to decide "edit vs. refactor first".
8. **Cyclomatic & readiness did not regress silently.** The session
   ends with `quality_delta.new_hotspots == []` OR an explicit,
   worklog-documented reason for the regression.

---

## 6. Appendix — raw session numbers (for diffing against Phase 3)

Captured during this session's refactor:

- Calls made: ~60 (rough count; formal count pending WS-1)
- `total_tokens_saved` at session start: 26,879,377
- `total_tokens_saved` at session end: 31,503,537
- Session-level delta: **~4.6M tokens saved**
- `evaluate_impact` calls made: **0** (anti-pattern — see WS-4)
- Fallbacks to native tools blocked by codesight hook: **2**
  (one `Grep`, one `Bash head`) — both re-routed successfully.
- Files changed by the refactor itself: 35 (including tests, scripts).
- Cargo check iterations: 2. Clippy iterations: 2. Nextest runs: 3.
  All green after the first iteration fix (missing `use std::fs;` in the
  ported test).

That ~4.6M figure is the upper-bound benchmark. With WS-3 in place, we
expect a similar session to touch **fewer files** (path_rename handles
the bulk) *and* save *more* tokens per edit (no manual re-reads to
verify). The two effects may offset partly — we'll know once WS-1 gives
us a clean before/after.

**Quality baselines we should have captured but didn't:**

- `evaluate_quality analysis=quality_signal` (repo-wide) at session start: **not captured**.
- Same at session end: **not captured**.
- Per-file `quality_signal scope=src/storage/doc_index/mod.rs` before deletion: **not captured** (would have been a pre-deletion justification: "low readiness because of `crate::storage` shim layering").
- Per-file `quality_signal scope=crates/codesight-storage/src/storage/doc_index/mod.rs` after move: **not captured** (would have validated the move actually improved `change_safety` by removing the shim).
- Hotspots before/after: **not captured**.
- `test_gaps` delta: **not captured** (unknown whether the ported WAL test regressed coverage).

Post-WS-8 these become a 5-line session-end table. Today they're
unanswerable — which is the whole point of the workstream.
