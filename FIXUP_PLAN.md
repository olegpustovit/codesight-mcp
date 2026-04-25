# FIXUP_PLAN.md — audited and corrected

This document replaces the original draft.

The original `FIXUP_PLAN.md` found many real defects, but it also mixed in:

- diagnoses that do not match the current code,
- response-shape proposals that would create unnecessary churn,
- items that are already partially implemented,
- and a few important gaps that were not listed at all.

This version keeps only validated work, ties every item to real code paths, and
breaks the work into implementation-ready tasks with explicit tests and
dependencies.

Scope of this audit:

- validated against the current tree on 2026-04-24,
- based on live tool calls plus handler inspection,
- with special focus on `search_catalog`, `evaluate_impact`, `evaluate_quality`,
  `inspect_structure`, `inspect_graph`, `assemble_context`, and
  `manage_indexes`.

Non-negotiable rules for all items below:

- No backcompat shims.
- Any response-shape change must update `crates/codesight-engine/src/tools.json`
  and the corresponding integration tests in the same patch.
- Do not call one MCP tool from inside another handler. Extract shared Rust
  helpers instead.
- Prefer fixing a wrong semantic over preserving an old field name.

---

## Audit summary

### Confirmed defects

- `rename_safety` can return `safe: true` for symbols with unresolved external
  references.
- `hotspots` emits no-op `actionable_next` rename suggestions.
- `search_catalog` keyword lookup has no symbol-name-only mode, so symbol-body
  matches dominate the tail.
- explicit `fuzzy=true` does not return fuzzy candidates when the exact lane is
  empty.
- empty identifier-like searches still return silent empties.
- `files_with_matches` is computed from already-truncated `items`.
- `scope_agent_readiness.score` is scaled down incorrectly.
- `blast_radius` duplicates file lists across multiple fields.
- `repo_outline` groups by path prefixes instead of real package/crate roots.
- `top_modules_by_loc` is actually `top_modules_by_size`.
- empty `scope=` matches return silent empties.
- `context_bundle` repeats imports once per symbol.
- `find_references` does not surface definition ambiguity at the top level.
- `quality_signal`, `repo_health`, and `hotspots` all miss their latency goals.
- `AGENTS.md` documents crates and backend layers that do not exist.
- `session_stats` uses a misleading grep/bash fallback field name.

### Confirmed issue, wrong original diagnosis or wrong original spec

- Original `P0.1`: the bug is real, but the proposed fix should not use the
  tool-layer `inspect_graph.references` response directly.
- Original `P0.3`: the problem is missing baselines, but current behavior is
  mostly omitted `tokens_saved`, not literal `0`.
- Original `P0.5`: score flattening is not caused by a `max(0.1, ...)` floor.
  It comes from `rerank_candidate()` collapsing many hits to the FTS-only
  branch.
- Original `P0.6`: the observed failure is primarily caused by the explicit
  fuzzy-empty-lane bug, not only by CamelCase/snake_case normalization.
- Original `P1.2`: there is no obvious hardcoded inner cap of 5 in the
  formatter; the underfilled result comes from building file counts from the
  already-limited item slice.
- Original `P1.3`: the score collapse is not caused by the scoped formula; the
  immediate bug is an engine-side `/ 10_000.0`.
- Original `P1.4`: the current system does not use LSP here. The real branches
  are `measured` vs `baseline_fallback`.
- Original `P1.6`: there is no `package_manifest_globs()` helper to reuse.
- Original `P4.4`: the tokenizer is not scattered across language modules; the
  current source of truth is `crates/codesight-retrieval/src/fuzzy.rs`.

### Already partially implemented

- `SYMBOL_NOT_FOUND_IN_CODE` already fires for the specific case "hybrid search
  returned docs-only hits and no code hits". It does not yet fire for empty
  identifier-like queries.
- `QUALITY_REGRESSION` already exists in `session_stats` / `session_end`.
- fuzzy normalization already lowercases and strips underscores, and retrieval
  tests already cover some camel/snake cases.

### Missing from the original plan

- `repo_health` renders `Governance score: <0..1>/10000`, which is obviously
  wrong and reintroduces a composite score that `governance_signal_json()`
  deliberately deemphasized.
- the current fuzzy path with `fuzzy=true` and zero exact hits needs a concrete
  response-shape decision; the original plan did not specify one.
- performance cache keys should be tied to index state, not only git commit
  state, because these tools operate on the current index snapshot.

---

## Corrected implementation plan

## Track 1 — Search and rename correctness

### T1.1 Reference-aware `rename_safety`

Replaces original `P0.1`.

Status: confirmed defect, original spec corrected.

Observed now:

- `evaluate_impact analysis=rename_safety` for
  `crates/codesight-engine/src/rpc.rs::dispatch_tool -> route_call`
  currently returns `safe: true`, `checked_files: 1`, `conflicts: []`.
- This is wrong because the symbol is referenced outside the defining site.

Current code:

- handler:
  `crates/codesight-engine/src/service/agent_v1/impact/rename_safety.rs`
- payload builder:
  `crates/codesight-analysis/src/impact.rs::rename_safety_payload`

What is wrong today:

- the analysis only checks name collisions in the defining file and importer
  outlines,
- plus trait-method lockstep edits,
- but it does not compute symbol-specific references that would need rewriting.

Why the original plan is wrong:

- do not make the handler call the JSON-returning `inspect_graph.references`
  tool path,
- and do not replace `safe` outright with `collision_free`; that is churn with
  no correctness win.

Implementation:

1. Extract a shared reference-scan helper from the logic behind
   `graph/references.rs` into a Rust helper module that accepts:
   `store`, `stable_id`, `symbol_name`, `file_path`, and a scan limit.
2. The helper must return typed rows, not JSON:
   `definition`, `symbol_body_reference`, `candidate_text_hit`.
3. Extend `rename_safety_payload()` to compute:
   - `collision_free`
   - `safe`
   - `unrewritten_references`
   - `reference_scan`
4. `safe` becomes:
   - `collision_free`
   - and `trait_dispatch.required_edits.is_empty()`
   - and `unrewritten_references.is_empty()`
5. Keep `safe` as the top-level decision field.
6. Add `collision_free` as a separate field so the caller can distinguish
   "rename collides" from "rename needs manual edits".

Proposed response shape:

```json
{
  "analysis": "rename_safety",
  "safe": false,
  "collision_free": true,
  "conflicts": [],
  "unrewritten_references": [
    {
      "file": "crates/codesight-engine/src/service/agent_v1/mod.rs",
      "line": 43,
      "kind": "symbol_body_reference",
      "enclosing_symbol": "dispatch"
    }
  ],
  "reference_scan": {
    "method": "symbol_body_substring",
    "complete": false,
    "scanned_files": 8
  },
  "trait_dispatch": { "...": "existing shape retained" }
}
```

Files:

- `crates/codesight-analysis/src/impact.rs`
- `crates/codesight-engine/src/service/agent_v1/impact/rename_safety.rs`
- new helper near `crates/codesight-engine/src/service/agent_v1/graph/`
  or `crates/codesight-engine/src/service/agent_v1/util/`
- `crates/codesight-engine/src/tools.json`

Tests:

- extend `tests/check_rename_safe.rs`
- add a case where two same-name definitions exist and only one is targeted
- assert `safe: false`, `collision_free: true`,
  `unrewritten_references.len() >= 1`

Acceptance:

- no `safe: true` result when a stable-id-targeted symbol still has external
  references that are not covered by the same patch plan.

---

### T1.2 Fix `hotspots` `actionable_next`

Replaces original `P0.2`.

Status: confirmed defect.

Current code:

- `crates/codesight-engine/src/service/agent_v1/quality/hotspots.rs`

What is wrong today:

- the handler emits `evaluate_impact:rename_safety` with
  `new_name == symbol_name`,
- so the suggested follow-up is a semantic no-op.

Implementation:

1. Replace the default follow-up with `evaluate_quality analysis=refactor_targets`.
2. Use file-level arguments, not rename arguments.
3. Preserve `reason`, but make it accurately about split planning.

New default:

```json
{
  "tool": "evaluate_quality",
  "args": {
    "source_id": "<source>",
    "analysis": "refactor_targets",
    "file_path": "<file>",
    "top_caller_files": 5
  },
  "reason": "Identify split candidates inside <symbol> before refactoring."
}
```

Files:

- `crates/codesight-engine/src/service/agent_v1/quality/hotspots.rs`
- `tests/hotspots.rs`
- `crates/codesight-engine/src/tools.json`

Acceptance:

- no `actionable_next` entry may call `rename_safety` with
  `new_name == symbol_name`.

---

### T1.3 Add `name_only` search mode for symbol lookup

Replaces original `P0.4`.

Status: confirmed defect.

Current code:

- args: `crates/codesight-engine/src/service/agent_v1/args.rs`
- planner: `crates/codesight-engine/src/service/agent_v1/catalog_search/plan.rs`
- handler: `crates/codesight-engine/src/service/agent_v1/catalog_search/mod.rs`
- storage search: `crates/codesight-storage/src/storage/search.rs`

What is wrong today:

- symbol search uses the full token bag and rerank features,
- there is no opt-out for body/signature/summary/docstring influence,
- so identifier queries are noisier than they need to be.

Implementation:

1. Add `name_only: Option<bool>` to `SearchCatalogArgs`.
2. Support it only for `query_mode=keyword`.
3. Wire it through to storage search.
4. In `name_only` mode:
   - match and rerank on symbol `name` and `qualified_name` only,
   - ignore summary/signature/docstring body features,
   - keep `file` and `kind` in the response unchanged.
5. Echo `_meta.name_only_applied: true`.
6. Add warning `WIDE_KEYWORD_RESULT` when:
   - keyword mode,
   - `name_only` is false,
   - and `total_matches > 50 * limit`.

Do not do in this patch:

- do not redesign all tokenization yet,
- do not introduce language-specific rules before the shared tokenizer extract
  is actually needed by more than one crate.

Files:

- `crates/codesight-engine/src/service/agent_v1/args.rs`
- `crates/codesight-engine/src/service/agent_v1/catalog_search/plan.rs`
- `crates/codesight-engine/src/service/agent_v1/catalog_search/mod.rs`
- `crates/codesight-engine/src/service/agent_v1/catalog_search/response.rs`
- `crates/codesight-storage/src/storage/search.rs`
- `crates/codesight-engine/src/tools.json`

Tests:

- extend `tests/search_catalog_defaults.rs`
- `dispatch_tool` with `name_only=true` should only return definition/alias-like
  symbol hits
- `dispatch_tool` without `name_only` should preserve current wide recall

Acceptance:

- identifier search can produce a name-focused answer in one call.

---

### T1.4 Fix short-stem score flattening in storage reranking

Replaces original `P0.5`.

Status: confirmed issue, original root cause incorrect.

Current code:

- tokenization: `crates/codesight-storage/src/db.rs::tokenize`
- rerank: `crates/codesight-storage/src/storage/search.rs::rerank_candidate`

What is actually happening:

- there is no `max(0.1, ...)` floor in the current reranker,
- many weak matches collapse to `0.10` because all overlap features are zero and
  only the `0.10 * fts_score` branch contributes.

Implementation:

1. Improve `rerank_candidate()` before changing FTS weights.
2. Add additional non-binary name features:
   - prefix match on `name`
   - substring match on `name`
   - token overlap on a camel/snake-aware tokenizer
   - deterministic path-depth/name-length tie-break
3. Keep the final score in `[0.0, 1.0]`.
4. Emit `_meta.tie_break_applied: true` when a tie-break path changes the
   ranking among same-score candidates.

Files:

- `crates/codesight-storage/src/storage/search.rs`
- tokenizer support, likely via a shared helper extracted later in `T6.3`

Tests:

- extend `tests/search_catalog_defaults.rs`
- add a query set where the top 5 no longer all serialize as `0.1`

Acceptance:

- short/common keyword queries no longer produce a flat top-10.

---

### T1.5 Make explicit `fuzzy=true` work when exact hits are empty

Corrects original `P0.6`.

Status: confirmed defect, original diagnosis incomplete.

Current code:

- `crates/codesight-engine/src/service/agent_v1/catalog_search/mod.rs::search_code_side`
- `crates/codesight-engine/src/service/agent_v1/catalog_search/fuzzy.rs`

What is wrong today:

- when `fuzzy=true` and BM25 returns zero exact symbol hits, the code does not
  compute a fuzzy lane,
- because the explicit fuzzy branch only runs when `!symbols.is_empty()`.

This is the primary reason calls like
`search_catalog(query="ingestSorces", fuzzy=true)` can return empty.

Implementation:

1. Keep the two-lane contract for explicit fuzzy:
   - `items` = exact lane
   - `fuzzy` = fuzzy lane
2. When `fuzzy=true` and exact hits are empty:
   - compute the fuzzy lane anyway,
   - return `items: []`,
   - return `fuzzy: [...]`,
   - add warning `NO_EXACT_MATCHES_RETURNING_FUZZY`.
3. Preserve the current auto-fuzzy behavior for
   `fuzzy_threshold` + zero exact hits.
4. Only after this lands, re-evaluate whether additional canonicalization is
   still needed.

Files:

- `crates/codesight-engine/src/service/agent_v1/catalog_search/mod.rs`
- `crates/codesight-engine/src/service/agent_v1/catalog_search/fuzzy.rs`
- `crates/codesight-engine/src/service/agent_v1/catalog_search/response.rs`
- tests under `tests/fuzzy_search.rs` and/or `tests/fuzzy_two_lane.rs`

Tests:

- explicit `fuzzy=true` + no exact hit returns a non-empty `fuzzy` lane
- exact hits still stay in `items`
- auto-fuzzy behavior remains unchanged

Acceptance:

- `fuzzy=true` never silently behaves like `fuzzy=false`.

---

### T1.6 Extend `SYMBOL_NOT_FOUND_IN_CODE` to true silent-empty cases

Corrects original `P1.1`.

Status: partially implemented already.

Current code:

- `crates/codesight-engine/src/service/agent_v1/catalog_search/mod.rs`

What exists already:

- the warning fires when hybrid search returns docs-only hits and no code hits.

What is missing:

- keyword or natural-language identifier-shaped queries that return no hits at
  all still fail silently.

Implementation:

1. Keep the existing docs-only path.
2. Add a second path for silent empties when:
   - `items.is_empty()`
   - query mode is `keyword` or `natural_language`
   - query is a single identifier-like token
   - `fuzzy != true`
3. Include `suggested_tool` and a suggested retry:
   - `fuzzy=true`
   - or `name_only=true` when available

Files:

- `crates/codesight-engine/src/service/agent_v1/catalog_search/mod.rs`
- `tests/symbol_not_found_in_code.rs`

Acceptance:

- identifier-shaped empty searches never return a silent empty payload.

---

### T1.7 Fix `files_with_matches` counting and truncation

Corrects original `P1.2`.

Status: confirmed defect, original root cause incorrect.

Current code:

- `crates/codesight-engine/src/service/agent_v1/catalog_search/response.rs`

What is wrong today:

- `files_with_matches` is built from the already-truncated `items` vector,
- so it can underfill the page and undercount files,
- and it does not set `truncated=true` even when `total_files_with_matches` is
  larger than the returned slice.

Implementation:

1. Carry full per-file counts separately from the rendered item slice.
2. Apply `offset` and `limit` at the file-count layer for
   `output_mode=files_with_matches`.
3. Always emit:
   - `data.truncated`
   - `_meta.truncated`
   - `truncated_by`
   when returned files are fewer than total files.

Files:

- `crates/codesight-engine/src/service/agent_v1/catalog_search/mod.rs`
- `crates/codesight-engine/src/service/agent_v1/catalog_search/response.rs`
- `tests/truncated_flag.rs`

Acceptance:

- `files.len() < total_files_with_matches` implies `truncated=true`.

---

### T1.8 Warn on empty `scope=` with nearest suggestions

Replaces original `P1.8`.

Status: confirmed defect.

Implementation:

1. Before final formatting, detect when `scope` matches zero indexed files.
2. Emit warning `SCOPE_MATCHES_NO_FILES`.
3. Include nearest suggestions derived from indexed file paths.
4. Add a copy-pasteable `suggested_tool` retry with the corrected scope.

Files:

- `crates/codesight-engine/src/service/agent_v1/catalog_search/mod.rs`
- likely a small helper under `agent_v1/util/`
- `tests/search_scope_exclusion.rs`

Acceptance:

- typoed scopes produce warnings, not silent empties.

---

## Track 2 — Quality and impact response correctness

### T2.1 Add real baselines to graph/impact/quality handlers

Corrects original `P0.3`.

Status: confirmed issue, original wording corrected.

What is wrong today:

- many graph/impact/quality handlers create `MetaBuilder::new(0)`,
- which means `_meta.tokens_saved` is omitted entirely,
- not consistently emitted as literal `0`.

Current code:

- savings primitives:
  `crates/codesight-engine/src/service/savings/{baseline.rs,meta_builder.rs}`
- affected handlers across:
  `graph/`, `impact/`, and `quality/`

Implementation strategy:

- do not land this as one giant patch,
- add baselines in slices so each slice has trustworthy tests.

Slice A:

- `inspect_graph:find_references`
- `inspect_graph:references`
- `inspect_graph:imports`
- `inspect_graph:importers`

Slice B:

- `evaluate_impact:rename_safety`
- `evaluate_impact:blast_radius`
- `evaluate_impact:diff`

Slice C:

- `evaluate_quality:hotspots`
- `evaluate_quality:quality_signal`
- `evaluate_quality:repo_health`
- `evaluate_quality:refactor_targets`
- `evaluate_quality:test_gaps`

Keep as null baseline:

- `manage_indexes:*`
- truly administrative handlers with no sensible native-tool counterfactual

Implementation notes:

- reuse `sum_code_file_sizes()` where the naive fallback is "read touched files"
- use row-count/line-count approximations where the naive fallback is grep-like
  output
- add per-handler helpers instead of inventing one opaque mega-baseline API

Tests:

- add `tests/tokens_saved_audit.rs`
- assert presence of `_meta.tokens_saved` only for handlers with implemented
  baselines
- do not require positive savings for administrative handlers

Acceptance:

- graph/impact/quality tools stop disappearing from the savings scoreboard once
  a baseline exists.

---

### T2.2 Fix scoped agent-readiness scaling

Corrects original `P1.3`. Re-audited 2026-04-24 to narrow scope.

Status: confirmed defect, original diagnosis incorrect, follow-up rewrite also
over-reached — this revision narrows the fix to the single provably-wrong site
and pins an investigation step before touching the rest.

Current code (all confirmed by direct read):

- `crates/codesight-engine/src/service/agent_v1/quality/mod.rs:316`
  (`scope_agent_readiness.score`)
- `crates/codesight-engine/src/service/mod.rs:489` (defining site of
  `record_agent_readiness_probe`)
- `crates/codesight-engine/src/service/agent_v1/session_admin.rs:289-294`
  (reads `baseline.{agent_readiness_score, structural_score, score}` and
  divides each by `10_000.0`)
- `crates/codesight-engine/src/service/savings/mod.rs:52` and
  `crates/codesight-engine/src/service/savings/pricing.rs:23`
  (use `* 10_000.0` / `round_cost`; intentional fixed-point rounding helpers —
  **do not touch**)

What is provably wrong today:

- `quality/mod.rs:316` divides `scoped.signal.score` (a normalized `0..1`
  float) by `10_000.0`, while its sibling dimensions (`navigability`,
  `comprehensibility`, `verifiability`, `change_safety`) are emitted
  undivided. Direct evidence: the live call
  `evaluate_quality analysis=quality_signal scope=…` returns
  `scope_agent_readiness.score: 0.0000766` alongside dimensions in the
  `0.3..0.95` range.

What is NOT yet known, so it must not be "fixed" blindly:

- The numeric format of `QualitySnapshot.{agent_readiness_score,
  structural_score, score}` as persisted by `snapshot_quality`. If those
  fields are stored as **basis-point u64/u32 integers** (0..10_000), the
  `/ 10_000.0` conversions in `session_admin.rs:289-294` are **correct unit
  conversions** and removing them would corrupt every `session_end`
  `quality_delta` report. If they are stored as **f64 0..1**, those
  divisions are redundant and must go.

Implementation — required order:

1. **Step 1 — investigate, do not edit.** Read the definition of
   `QualitySnapshot` in `codesight-storage` (and the body of
   `record_agent_readiness_probe` in `service/mod.rs:489`) to determine the
   persisted format of each `*_score` field. Record the finding at the top
   of the patch PR description.
2. **Step 2 — always applies regardless of step 1.** Remove
   `/ 10_000.0` from `scope_agent_readiness.score` at
   `quality/mod.rs:316`. This site divides a value that is already `0..1`;
   it is wrong in all scenarios.
3. **Step 3 — conditional on step 1 finding.**
   - If persisted format is **f64 `0..1`** → remove the `/ 10_000.0`
     divisions in `session_admin.rs:289-294` AND any matching division in
     `record_agent_readiness_probe` / `service/mod.rs:489`. Update
     `tests/session_stats.rs` expectations in the same patch.
   - If persisted format is **basis-point integer** → leave
     `session_admin.rs:289-294` and `record_agent_readiness_probe` alone.
     Add a one-line doc comment above each division explaining the unit
     conversion so the next reader doesn't mistake it for a bug.
4. **Do not** touch `savings/mod.rs:52` or `savings/pricing.rs:23` under
   any scenario — those are deliberate 4-decimal rounding helpers.
5. Add a parity test: on a repo where scope == entire repo,
   `scope_agent_readiness.score` must fall within 0.05 of
   `agent_readiness_score`. This test must fail on today's code and pass
   after step 2.
6. Add a second test in `tests/session_stats.rs` that exercises
   `manage_indexes action=session_end` with a known baseline snapshot and
   asserts `quality_delta.agent_before ∈ [0, 1]` — this is the regression
   guard against removing the wrong divisions in step 3.

Files:

- `crates/codesight-engine/src/service/agent_v1/quality/mod.rs`
  (always edited — step 2)
- `crates/codesight-engine/src/service/agent_v1/session_admin.rs`
  (edited only if step 1 says f64)
- `crates/codesight-engine/src/service/mod.rs`
  (edited only if step 1 says f64 and `record_agent_readiness_probe` also
  divides)
- `crates/codesight-storage/src/storage/quality_snapshot.rs` (or wherever
  `QualitySnapshot` is defined — adjust format if step 1 reveals a
  writer/reader mismatch)
- `tests/quality_signal.rs` (parity test)
- `tests/session_stats.rs` (regression guard)

Do NOT touch:

- `crates/codesight-engine/src/service/savings/mod.rs:52`
- `crates/codesight-engine/src/service/savings/pricing.rs:23`

Acceptance:

- Scoped `scope_agent_readiness.score` is always within the same numeric
  range (`0..1`) as its sibling dimensions.
- `manage_indexes action=session_end` still emits `quality_delta` values
  in `0..1` with no regression vs the prior release.
- The PR description states, in one sentence, which branch of step 3 was
  taken and why.

---

### T2.3 Expand type-annotation provenance using current backend reality

Corrects original `P1.4`.

Status: confirmed gap, original proposed branches incorrect.

Current code:

- `crates/codesight-analysis/src/agent_readiness/submetrics.rs`
- `crates/codesight-analysis/src/quality_signal/governance.rs`
- `crates/codesight-language-support/src/facade.rs`

Reality today:

- the system distinguishes `Measured` vs `BaselineFallback`,
- not `lsp | treesitter | baseline_fallback`.

Implementation:

1. Keep the primary method values aligned to current code:
   - `measured`
   - `baseline_fallback`
2. Add provenance detail:

```json
{
  "type_annotation": {
    "method": "measured",
    "reason": "language_support_measured_types",
    "measured_files": 120,
    "fallback_files": 34,
    "total_files": 154
  }
}
```

3. Reasons should be drawn from real code paths:
   - `language_support_measured_types`
   - `language_has_no_measured_types`
   - `measured_density_unavailable`
4. Emit `TYPE_ANNOTATION_FALLBACK` only when fallback dominates the scoped or
   global sample.

Acceptance:

- callers can tell whether the score came from measured language support or a
  heuristic fallback, and at what coverage ratio.

---

### T2.4 Remove `blast_radius` duplication

Replaces original `P1.5`.

Status: confirmed defect.

Current code:

- `crates/codesight-engine/src/service/agent_v1/impact/blast_radius.rs`

Implementation:

1. Replace:
   - `confirmed`
   - `potential`
   - duplicated inline `findings[].files`
2. With:
   - `direct_importers`
   - `transitive_importers`
   - `findings: [{ depth, count, risk_score }]`
3. Keep counts and `actionable_next`.

New shape:

```json
{
  "analysis": "blast_radius",
  "direct_importers": [],
  "transitive_importers": [],
  "findings": [
    { "depth": 1, "count": 4, "risk_score": 1.0 },
    { "depth": 2, "count": 12, "risk_score": 0.5 }
  ]
}
```

Acceptance:

- payload shrinks materially with no information loss.

---

### T2.5 Surface ambiguous definitions + sub-classify body matches in `find_references`

Replaces original `P2.4`. Expanded 2026-04-24 re-audit.

Status: confirmed gap; add sub-classification for body matches.

Current code:

- `crates/codesight-engine/src/service/agent_v1/graph/find_references.rs`
- `split_semantic_and_candidate` at line 289 in the same file

Implementation:

1. Count rows whose `kind == "definition"`.
2. Add top-level fields:
   - `definition_count`
   - `definitions` (mirror of the definition rows, broken out)
3. Emit warning `AMBIGUOUS_DEFINITION` when `definition_count > 1`.
4. Sub-classify every existing `symbol_body_match` row into one of:
   - `call_site` — the match appears in a call expression
     (`foo(…)` / `obj.foo(…)` / `foo::<T>(…)`)
   - `use_stmt` — match appears inside an `use`, `import`, `require`, or
     equivalent language-specific import form
   - `type_ref` — match appears in a type position
     (`let x: foo`, `impl foo for T`, `Box<foo>`, TS `: foo`, Py `-> foo`)
   - `comment_or_string` — match appears inside a comment or string literal
     (should generally be filtered out or deprioritized by callers)
   - `other` — structural position the tree-sitter query didn't recognize
5. Expose the sub-kind on the row as `body_match_kind: "<one of above>"`,
   keeping the existing `kind: "symbol_body_match"` unchanged.
6. Language coverage for sub-classification follows the standard tiers —
   land Rust / TS / Python / Go first (shared tree-sitter query patterns);
   everything else falls back to `"other"` until queries land.

Acceptance:

- callers do not have to infer ambiguity by scanning the whole row list.
- agents filtering out `comment_or_string` hits never drop a real call site
  as collateral.

---

### T2.6 Fix `repo_health` governance summary units

New item missing from the original plan.

Status: confirmed defect.

Current code:

- `crates/codesight-engine/src/service/agent_v1/quality/repo_health.rs`

What is wrong today:

- it formats `gov.score` as `<value>/10000`,
- but `gov.score` is normalized to `0..1`.

Implementation:

1. Stop printing the misleading composite as `/10000`.
2. Align the summary with `governance_signal_json()`:
   - report `structural_score`
   - report `agent_readiness_score`
   - report `bottleneck`
3. If a composite is kept at all, render it as a percent.

Acceptance:

- `repo_health` no longer reports impossible units.

---

## Track 3 — Structural output correctness

### T3.1 Make `repo_outline` package-aware

Corrects original `P1.6`.

Status: confirmed defect, original helper reference incorrect.

Current code:

- `crates/codesight-engine/src/service/agent_v1/structure.rs::build_workspace_crates`
- `crate_name_for_path()` currently groups by path prefixes

Why the original spec is wrong:

- there is no `package_manifest_globs()` helper to reuse,
- but the repo already has language-aware package discovery via
  `codesight-language-support/src/packages/` and per-language
  `package_name_for_dir()`.

Implementation:

1. Replace path-prefix grouping with real package-root detection via the
   per-language `Language::package_name_for_dir(&Path) -> Option<String>`
   method (confirmed present for at least csharp, go, javascript, jsx,
   python at 2026-04-24 — extend coverage as a follow-up, not in this
   patch).
2. For Rust, only directories with a real `Cargo.toml` that declares
   `[package]` or `[lib]` should land in `workspace.crates`.
3. Add `extra_roots[]` with a closed `role` vocabulary — callers parse on
   it, so expand the enum explicitly in a follow-up when needed, never
   implicitly:
   - `role: "tests"` — integration-test roots (`tests/`)
   - `role: "scripts"` — automation / one-off scripts (`scripts/`,
     `tools/`)
   - `role: "plugins"` — plugin manifests (`plugins/`)
   - `role: "config"` — build/runtime config roots (`.cargo/`,
     `.config/`)
   - `role: "docs"` — documentation-only roots
   - `role: "other"` — unclassified; MUST include `hint: <first-file>` in
     the entry so the agent can guess from one probe
4. Keep `workspace.crates` for the current contract, but populate it with
   real crate/package roots only.

Proposed entry shape:

```json
{
  "extra_roots": [
    { "path": "tests",    "role": "tests",   "file_count": 89 },
    { "path": "scripts",  "role": "scripts", "file_count": 7  },
    { "path": ".",        "role": "other",   "file_count": 8,
      "hint": "AGENTS.md" }
  ]
}
```

Files:

- `crates/codesight-engine/src/service/agent_v1/structure.rs`
- helper support from `crates/codesight-language-support/src/packages/`
  and per-language `package_name_for_dir()` implementations
- `crates/codesight-engine/src/tools.json` (response-shape bump)
- `tests/get_file_tree.rs`

Acceptance:

- non-package directories never appear in `workspace.crates`.

---

### T3.2 Rename `top_modules_by_loc` to match its sort key

Corrects original `P1.7`.

Status: confirmed defect.

Current code:

- response field name in `repo_outline`
- implementation helper:
  `crates/codesight-engine/src/service/agent_v1/structure.rs::top_modules_by_size`

Implementation:

1. Rename the response field to `top_modules_by_size`.
2. Update `tools.json` and tests.
3. Do not compute real LoC in the same patch.

Acceptance:

- field name matches what the code actually does.

---

## Track 4 — Context and UX polish

### T4.1 Standardize warning payloads with `suggested_tool`

Corrects original `P2.1`.

Status: good direction, needs normalization.

Note:

- `QUALITY_REGRESSION` already exists and should be brought into the same
  warning contract instead of being rediscovered.

Implementation:

1. Every high-signal warning added in Tracks 1 and 2 should include:

```json
{
  "code": "SCOPE_MATCHES_NO_FILES",
  "message": "...",
  "suggested_tool": {
    "tool": "search_catalog",
    "args": { "...": "..." }
  },
  "detail": { "...": "..." }
}
```

2. Apply this shape to:
   - `SYMBOL_NOT_FOUND_IN_CODE`
   - `SCOPE_MATCHES_NO_FILES`
   - `WIDE_KEYWORD_RESULT`
   - `TYPE_ANNOTATION_FALLBACK`
   - `QUALITY_REGRESSION`

Acceptance:

- warnings become immediately actionable instead of merely descriptive.

---

### T4.2 Deduplicate imports in `context_bundle`

Replaces original `P2.2`.

Status: confirmed defect.

Current code:

- `crates/codesight-engine/src/service/agent_v1/context/shape.rs`

Implementation:

1. In multi-symbol mode, move imports to a file-level map:

```json
{
  "mode": "context_bundle",
  "files": {
    "path/to/file.rs": { "imports": ["use ..."] }
  },
  "symbols": [
    { "file": "path/to/file.rs", "name": "...", "source": "..." }
  ]
}
```

2. Keep the single-symbol compact shape unchanged unless the caller requested a
   multi-symbol bundle or budget report.
3. When budget trimming is needed, drop duplicated import material first.

Acceptance:

- imports appear once per file, not once per symbol.

---

### T4.3 Treat NL ranking work as benchmark-first, not heuristic-first

Corrects original `P2.3`.

Status: defer until measured.

Reason:

- the original proposal is directionally plausible but under-specified.
- Do not ship ad hoc verb heuristics before there is a labeled benchmark.

Implementation plan:

1. Add a small benchmark corpus:
   - 20 to 30 natural-language queries
   - expected top-1 or top-3 targets
2. Measure current rank quality.
3. Only then change ranking features.

Files:

- benchmark fixture under `tests/` or `crates/codesight-retrieval/tests/`

Acceptance:

- any ranking change is justified by benchmark movement, not intuition.

---

## Track 5 — Performance

### T5.1 Cache `quality_signal` by index state, not just git state

Corrects original `P3.1`.

Status: confirmed performance issue, cache-key spec corrected.

Current code:

- `crates/codesight-engine/src/service/agent_v1/quality/mod.rs`
- `crates/codesight-analysis/src/quality_signal/governance.rs`
- existing disk cache module:
  `crates/codesight-engine/src/server_support/agent_readiness_cache.rs`

Important correction:

- these handlers operate on indexed data,
- so invalidation must key off index state such as `index_version` /
  `indexed_at`, not only `HEAD`.

Implementation:

1. Cache global governance/quality-signal computation by:
   - `source_id`
   - `index_version` or equivalent index generation marker
2. Cache scoped readiness by:
   - `source_id`
   - `index_version`
   - `scope`
3. Reuse or extend existing cache infrastructure; do not bolt on a second,
   parallel cache file format without reason.

Targets:

- cold: <= 400 ms
- warm: <= 50 ms

---

### T5.2 Make `repo_health` reuse cached sub-analyses

Corrects original `P3.2`.

Status: confirmed performance issue.

Current code:

- `repo_health` recomputes governance, dead code, and hotspots inline.

Implementation:

1. Reuse the global `quality_signal` cache from `T5.1`.
2. Reuse cached churn/hotspot data from `T5.3`.
3. Avoid nested recomputation inside a single `repo_health` call.

Targets:

- cold: <= 300 ms
- warm: <= 30 ms

---

### T5.3 Cache churn input for `hotspots`

Corrects original `P3.3`.

Status: confirmed performance issue.

Current code:

- `crates/codesight-analysis/src/quality.rs::hotspots_payload`
- recomputes `compute_file_churn(root)` each call

Implementation:

1. Cache churn table by:
   - repo root
   - git `HEAD` when available
2. Invalidate when `HEAD` changes.
3. If no git repo is present, skip the churn cache rather than inventing a weak
   invalidation story.

Targets:

- cold: <= 300 ms
- warm: <= 20 ms

---

## Track 6 — Documentation and telemetry cleanup

### T6.1 Fix `AGENTS.md` layout/backend drift

Combines original `P4.1` and `P4.2`.

Status: confirmed drift.

Implementation:

1. Remove the phantom `crates/mcp-rpc/` row.
2. Remove or rewrite references to non-existent `codesight-backend`,
   `CodeBackend`, and `BackendRouter`.
3. Document the actual layering that exists in the current tree.
4. Do not create new crates just to satisfy stale docs.

Acceptance:

- `AGENTS.md` matches the repository, not an abandoned design.

---

### T6.2 Rename grep/bash fallback telemetry for honesty

Replaces original `P4.3`.

Status: confirmed defect.

Current code:

- `crates/codesight-engine/src/service/agent_v1/session_admin.rs`

Implementation:

1. Rename `fallbacks_to_grep_or_bash` to `blocked_grep_bash_attempts`.
2. Rename `stale_warnings_fired_with_in_flight_paths` (45 chars, awkward to
   log-scrape) to `stale_warnings_in_flight` — keep the inner array shape.
3. Update tests and `tools.json` for both renames.
4. Document that `edits_after_codesight_read` being non-zero is healthy
   (means the agent read-then-edited, not edited blindly).
5. No deprecated aliases — rename in place, update every caller in the same
   patch (per repo no-backcompat rule).

Acceptance:

- session telemetry describes what actually happened.
- no field name in `session_stats` exceeds ~30 characters.

---

### T6.3 Extract a shared naming tokenizer only when multiple crates need it

Corrects original `P4.4`.

Status: worthwhile, but not the first fix.

Current reality (verified 2026-04-24):

- the reusable tokenizer logic currently lives in
  `crates/codesight-retrieval/src/fuzzy.rs` — specifically the public
  symbols:
  - `trigrams(&str) -> HashSet<String>`
  - `trigram_jaccard(&str, &str) -> f64`
  - `edit_distance(&str, &str) -> usize`
  - `normalize_fuzzy_compare(&str) -> String` (the canonicalizer —
    lowercases, strips underscores)
  - `tokenize_name(&str) -> HashSet<String>` plus private
    `split_camel(&str) -> Vec<String>`
  - `score_name_pair(&str, &str) -> FuzzyPairScore`
  - `fuzzy_candidate_passes(...)`
  - existing tests `camel_typo_matches_snake_case_target`,
    `camel_matches_snake_exact_letters`,
    `identical_snake_case_scores_one`
- camel/snake normalization already works for simple cases; do NOT reinvent
  it in a new module without benchmarking against these.

Implementation:

1. Land `T1.3`, `T1.4`, and `T1.5` first; each may only need the existing
   fuzzy helpers — reach for them before copy-pasting.
2. Only if `codesight-storage` AND `codesight-engine` both independently
   need the same tokenizer (not just `codesight-retrieval` internally),
   extract the above symbols into `crates/codesight-core/src/naming.rs`
   and re-export from `codesight-retrieval::fuzzy` for existing callers —
   no backcompat shim, just move + update imports in the same patch.
3. The extracted module must cover, at minimum, the symbols listed above;
   do not drop `fuzzy_candidate_passes` or `score_name_pair` in the move.
4. `tests/fuzzy_search.rs` and the in-crate `p1_2_tests` module must pass
   unchanged after the move.

Do not do this first:

- do not rewrite multiple crates just to satisfy an abstract cleanup goal
  before there is a second real caller.

Acceptance:

- one tokenizer implementation is shared only when sharing actually removes
  duplication, and every existing test in `codesight-retrieval::fuzzy` still
  passes.

---

## Rollout order

Phase 1:

- `T1.1` reference-aware rename safety
- `T1.2` hotspots actionable-next
- `T1.5` explicit fuzzy-empty behavior
- `T2.2` scoped score scaling
- `T2.6` repo_health governance summary units

Phase 2:

- `T1.3` name-only keyword search
- `T1.6` silent-empty warning extension
- `T1.7` files_with_matches correctness
- `T1.8` empty-scope warnings
- `T3.2` top_modules field rename

Phase 3:

- `T2.1` baseline slice A
- `T2.3` type-annotation provenance
- `T2.4` blast-radius shape cleanup
- `T2.5` find-references ambiguity
- `T4.1` warning normalization

Phase 4:

- `T2.1` baseline slices B and C
- `T3.1` package-aware repo_outline
- `T4.2` context-bundle import dedupe
- `T1.4` short-stem ranking improvements

Phase 5:

- `T5.1` through `T5.3` performance caches
- `T6.1` through `T6.3` docs and cleanup work
- `T4.3` benchmark-driven NL ranking work

Dependencies:

- `T1.4` may depend on `T6.3` if the tokenizer needs to be shared across
  storage and retrieval.
- `T5.2` depends on `T5.1` and `T5.3`.
- `T3.1` should land after `T3.2` if both touch `repo_outline` shape.
- `T2.1` must be delivered in slices, not as a single giant patch.

---

## Verification matrix

Run these after the whole plan lands:

1. `search_catalog query="dispatch_tool" query_mode="keyword" unit_kinds=["symbol"]`
   returns the same broad recall as today.
2. The same query with `name_only=true` returns only symbol-name matches.
3. `search_catalog query="ingestSorces" query_mode="keyword" fuzzy=true`
   returns `items: []` and a non-empty `fuzzy` lane plus a warning.
4. `search_catalog query="BackendRouter"` returns
   `SYMBOL_NOT_FOUND_IN_CODE`.
5. `search_catalog query="dispatch_tool" output_mode="files_with_matches" limit=5`
   returns `truncated=true` and correct `total_files_with_matches`.
6. `search_catalog query="dispatch_tool" scope="crates/mcp-rpc/"` returns
   `SCOPE_MATCHES_NO_FILES`.
7. `evaluate_impact analysis="rename_safety"` on
   `rpc.rs::dispatch_tool -> route_call` returns `safe: false` and non-empty
   `unrewritten_references`.
8. `evaluate_quality analysis="hotspots"` never emits a rename suggestion with
   `new_name == symbol_name`.
9. `evaluate_quality analysis="quality_signal" scope="crates/codesight-engine/src/service/"`
   returns a `scope_agent_readiness.score` in `0..1`, not `0.0000x`.
10. `inspect_structure view="repo_outline"` returns only real crates/packages in
    `workspace.crates`, with non-code roots moved to `extra_roots`.
11. `inspect_graph graph="find_references" identifier="dispatch_tool"` returns
    `definition_count > 1` and `AMBIGUOUS_DEFINITION`.
12. `evaluate_quality analysis="repo_health"` no longer prints a governance
    score as `/10000`.
13. `manage_indexes action="session_stats"` reports
    `blocked_grep_bash_attempts`, not `fallbacks_to_grep_or_bash`.
14. Warm `quality_signal`, `repo_health`, and `hotspots` calls meet the cache
    budgets in Track 5.
15. `search_catalog query="..." output_mode="count"` on a hit set larger than
    any internal cap returns the correct `count`, does NOT emit a misleading
    `truncated=true`, and passes a round-trip check where the same query with
    `output_mode=items limit=<count>` returns the same number of items.
16. `inspect_graph graph="find_references" identifier="dispatch_tool"` returns
    rows where every `symbol_body_match` carries a `body_match_kind` in
    `{call_site, use_stmt, type_ref, comment_or_string, other}`, and rows
    inside comment/string literals are tagged `comment_or_string` on at
    minimum Rust / TS / Python / Go fixtures.
17. `manage_indexes action="session_stats"` reports
    `stale_warnings_in_flight` (not
    `stale_warnings_fired_with_in_flight_paths`).

If the matrix passes, the fixup plan is complete and internally consistent.
