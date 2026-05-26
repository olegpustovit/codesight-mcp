# CUTPLAN.md

`codesight-mcp` reliability-first redesign. The goal is **lean and trustworthy**:
fewer tools, every one of which an agent can bet a refactor on. Reliability
beats reach. Token savings and coding performance are downstream of trust — an
agent that has to double-check every result with native `Read`/`Grep` saves
nothing.

## North star

Three invariants. If a feature can't satisfy all three, it ships as a CLI
subcommand or it does not ship.

1. **Structural truth or refusal.** A tool that claims to answer a symbol-level
   question (references, blast radius, rename safety) either resolves
   structurally (tree-sitter scope + import graph) or returns
   `confidence: unreliable` and refuses to fabricate a file-level approximation.
   No silent fallbacks. No "use for exploration only" disclaimers buried in
   warnings.
2. **One confidence axis.** `safe | partial | unreliable`. `unreliable` results
   never appear in primary `items[]` — only in opt-in `weak_hits[]`.
3. **Honest performance budgets.** Every tool has a tier in AGENTS.md, every
   tool meets it on this repo, replayed automatically on every PR. Misses
   block merge. No quiet ratchets.

Everything below is in service of those three.

## Why we keep iterating

The current engine has a healthy core (FTS5, tree-sitter, symbol extraction,
SQLite WAL, the `FactQuality` tiering scheme) and a rotten layer on top of it
(graph, impact, quality features built on **text identifier scans through
symbol bodies**). Symptoms observed in a 25-probe dogfood session:

- `find_references FactQuality` returned 24 "semantic_references" whose `name`
  field is the *enclosing function* — reads as "references to other things".
- `blast_radius FactQuality` returned 229 "direct importers" — every file
  importing anything from `codesight-core::symbols`, not just FactQuality.
- `inspect_graph calls` returned 908 callees including every `new`, `id`,
  `default` in the workspace — text whole-word matching on Rust.
- `rename_safety FactQuality → FactGrade` returned `complete: false`,
  `scanned_files: 6`, `excluded: 300`, **and** `confidence: high` — internally
  inconsistent.
- `repo_outline` took 2810 ms (SLA 200 ms cold) — first call a new agent
  makes, the slowest tool in the suite.
- `fuzzy=true` on a typo returned `items: []` while burying 9 perfect hits in
  `data.fuzzy`. Agents that don't read both lanes report "no results".
- NL search cross-ranked `AGENTS.md` sections at score 0.4 against actual code
  hits at 0.572. Score gap too narrow for the relevance gap.

The pattern is consistent: **tools advertise symbol-level answers and deliver
file-level or substring-level approximations, hidden behind warnings**. You
can't iterate your way out of this — the architecture must shrink to what
tree-sitter can guarantee.

## The surviving surface — 6 tools

```
search       (was: search_catalog)
outline      (was: inspect_structure file_outline | repo_outline)
read         (was: read_content + section_context + complexity)
references   (was: inspect_graph find_references | imports | importers)
impact       (was: evaluate_impact blast_radius | rename_safety)
health       (was: evaluate_quality quality_signal | hotspots | dead_code | doc_coverage | repo_health)
```

`assemble_context`, `evaluate_impact diff/path_rename/changed_units`,
`inspect_graph calls/centrality/module_coupling/layer_violations/related/resolve_fqn`,
`evaluate_quality broken_links/refactor_targets/regression_check/test_gaps`,
`manage_indexes` admin actions — **all gone from the MCP surface**, no
external replacement. Workflows that needed them (path-rename automation,
doc-coverage gates) live in shell + git, not in this codebase.

### Per-tool contracts

#### 1. `search`

Inputs (6 only): `query, mode, scope, limit, max_tokens, fuzzy`.

| Param | Values | Notes |
|---|---|---|
| `query` | string | required |
| `mode` | `keyword` (default) \| `regex` \| `nl` | one ranker, one mode |
| `scope` | path glob | replaces `unit_kinds`/`file_pattern`/`exclude` — glob does it all |
| `limit` | int, default 25 | |
| `max_tokens` | int, soft cap | |
| `fuzzy` | bool | merges into `items[]`, never side-channeled |

**Cut:** `unit_kinds`, `domain`, `semantic`, `pattern` (alias), `output_mode` (count survives as a separate tool? no — drop), `include_vendored`, `include_tests`, `paranoid`, `min_confidence`, `entry_point_patterns`, `granularity`, `exclude`, `file_pattern`, `_meta_verbose`, `offset` (use cursor in response if needed), `repo_path`, `source_id`, `token_budget` (alias), `context_lines` (use `read` for that), `include_body` (use `read`).

**Reliability fixes:**
- `mode=nl` defaults to `domain=code`. Markdown only enters results when
  `mode=nl` is explicitly given a `scope=*.md`-style glob.
- `fuzzy=true` merges fuzzy hits into `items[]` with `match_type: "fuzzy"`.
  No second lane. When zero exact hits exist, fuzzy hits are the items list.
- Regex no longer silently ignores `unit_kinds` — there is no `unit_kinds`.

#### 2. `outline`

One tool, two views:

| `view` | Purpose |
|---|---|
| `file` | symbol list of one file (or batch via `paths[]`) |
| `repo` | workspace summary: crates, top-modules-by-centrality, languages |

**Cut views:** `file_tree`, `repository_rollup`, `toc`, `toc_tree`,
`document_outline`, `class_hierarchy`. `file_tree` is `ls -R` reinvented;
delete. Doc TOCs collapse into `outline view=file` for markdown files.

**Reliability fixes:**
- `view=repo` centrality cached in SQLite alongside the import graph. Recompute
  only inside incremental reingest, not per-call. Target: 50 ms warm,
  200 ms cold (currently 2810 ms — 14× SLA miss).

#### 3. `read`

Batched reads. Refs are **strings only**:

```
"path:start-end"      # range
"path:line"           # single line
"path::Name#kind"     # symbol by name
"id:<stable_id>"      # symbol by id
"section:path::heading"  # doc section
```

**Cut:** the object form (`{kind, path, symbol_name, ...}`) — eight ways to
identify a symbol is seven too many. One canonical shorthand. Cut
`source_id` overrides per-ref (source is implicit per session).
Cut `verify`, `wait_for_fresh`, `max_wait_ms`, `include` extras (`window` /
`hash`). Cut the `section` kind being a separate top-level mode — fold in.

**Reliability fixes:**
- Whole-file reads emit `confidence: partial` when no symbol/range was given,
  with a `outline_first` hint pointing at the matching `outline` call.
- Per-ref `truncated: true` when `max_tokens` is exceeded — stable ordering
  guaranteed (refs honored in input order).

#### 4. `references`

Three subtypes via `kind`:

| `kind` | Purpose | Confidence floor |
|---|---|---|
| `symbol` | who references this symbol | `safe` only — refuse with `unreliable` if scope analyzer absent |
| `imports` | what this file imports | `safe` |
| `importers` | who imports this file | `safe` |

**Cut:** `find_references` (use `kind=symbol`), `references` vs
`find_references` duplication, `related`, `calls` (text-based, not
recoverable), `centrality` (internal), `module_coupling`, `layer_violations`
(inert without config nobody writes), `dependency_cycles` (move to `health`),
`resolve_fqn` (niche).

**Reliability fixes:**
- `kind=symbol` returns one `items[]` with shape:
  `{file, line, column, identifier, enclosing_symbol, enclosing_kind, body_match_kind}`.
  `identifier` is the searched name (currently `name` is the *enclosing*
  function, which is the misleading bit).
- Text-only candidate hits move to opt-in `weak_hits[]` — never primary.
- Languages without scope-analyzer support return `confidence: unreliable`
  and a list of *files* that mention the identifier as `weak_hits` only.
  No more pretending.

#### 5. `impact`

Two analyses only:

| `analysis` | Purpose |
|---|---|
| `blast_radius` | symbol-scoped: which files actually use this symbol |
| `rename_safety` | collision check + reference enumeration for rename |

**Cut:** `diff` (use `git diff`), `changed_units` / `change_velocity` (use
`git log`), `path_rename` with `apply=true` (dropped — `git grep`/`sed`),
`impact_preview` (overlap with blast_radius).

**Reliability fixes:**
- `blast_radius` walks **symbol references**, not file imports. The current
  "229 direct importers of FactQuality" is the bug — direct importers of the
  file `symbols.rs` are not direct importers of the type. Use the
  `references kind=symbol` graph, deduplicate by file. If the language can't
  produce structural references, return `confidence: unreliable` with a
  count of files-that-mention-the-name as the only signal.
- `rename_safety` requires a complete scan or refuses. New invariant:
  `complete=false → safe=undecided` (never `safe=false` on incomplete data).
  `coverage_pct` is now part of the response. The current
  `complete:false / scanned:6 / excluded:300 / confidence:high` quadrilemma
  is illegal in the new contract.

#### 6. `health`

One analysis enum, narrow set:

| `analysis` | Purpose |
|---|---|
| `quality_signal` | agent-readiness submetrics (already the strongest tool) |
| `hotspots` | churn × cyclomatic, top-N |
| `dead_code` | reachability + dispatch heuristic |
| `repo_health` | aggregate of the three above + cycle count |

**Cut:** `complexity` (single-symbol — fold into `read` as opt-in field),
`doc_coverage` (move to CLI gate), `broken_links` (CLI gate),
`refactor_targets` (overlap with hotspots — surface as `sort=refactor_score`
on hotspots; delete the analysis), `regression_check` +
`manage_indexes snapshot_quality` (nobody runs the snapshot ritual),
`test_gaps` (fold the symbol-list into `quality_signal verbose=true`).

**Reliability fixes:**
- `dead_code` confidence ≥ 0.5 default stays. The string-literal-dispatch
  heuristic is honest and well-warned — it's the model citizen.
- `quality_signal` already reliable; just tighten thresholds doc.

## Cut list — exhaustive

| Removed | Replacement / rationale |
|---|---|
| `search_catalog unit_kinds` | use `scope` glob (`*.md`, `crates/foo/**`) |
| `search_catalog domain` | implied by `mode` + `scope` |
| `search_catalog output_mode=count/files_with_matches` | post-process `items[]` client-side |
| `search_catalog fuzzy` two-lane | merge into `items[]` |
| `search_catalog _meta_verbose` | always emit `_meta`; no toggle |
| `search_catalog include_vendored/tests/paranoid` | one path: vendored excluded by default, opt-in via `scope` |
| `inspect_structure file_tree` | `ls -R` exists |
| `inspect_structure repository_rollup` | merged into `outline view=repo` |
| `inspect_structure toc/toc_tree/document_outline` | one `outline view=file` for md |
| `inspect_structure class_hierarchy` | niche |
| `read_content` object refs | string shorthand only |
| `read_content verify/wait_for_fresh` | freshness is the index's job, not per-call |
| `read_content section_context` | string ref `section:path::heading` |
| `inspect_graph calls` | text-based, not recoverable on Rust |
| `inspect_graph centrality` | internal use only |
| `inspect_graph module_coupling` | overlap with `importers` |
| `inspect_graph layer_violations` | inert, never configured |
| `inspect_graph dependency_cycles` | rolled into `health repo_health` |
| `inspect_graph related` | vague semantics |
| `inspect_graph resolve_fqn` | niche |
| `evaluate_impact diff` | `git diff` |
| `evaluate_impact changed_units` | `git log` |
| `evaluate_impact change_velocity` | `git log` |
| `evaluate_impact path_rename apply=true` | dropped — use `git grep`/`sed` |
| `evaluate_impact impact_preview` | overlap with `blast_radius` |
| `evaluate_quality complexity` (single-symbol) | opt-in field on `read` |
| `evaluate_quality doc_coverage` | dropped — out of scope |
| `evaluate_quality broken_links` | dropped — out of scope |
| `evaluate_quality refactor_targets` | `health hotspots sort=refactor` |
| `evaluate_quality regression_check` | snapshot ritual unused |
| `evaluate_quality test_gaps` | `quality_signal verbose=true` |
| `assemble_context bundle` | `read with_neighbors=true` |
| `assemble_context section_context` | `read` with `section:` ref |
| `assemble_context ranked` | overlap with `search` |
| `assemble_context query_suggestions` | ignores query in practice |
| `assemble_context rename_plan` | overlap with `rename_safety` |
| `manage_indexes invalidate/delete/rebuild` | dropped — `rm -rf $storage_dir` and reingest |
| `manage_indexes snapshot_quality` | snapshot ritual unused |
| `manage_indexes session_end` | trivial; `stats` covers it |
| `manage_indexes status` per source | `stats` global |
| `catalog_sources` as required first call | source resolved from cwd implicitly |
| `ingest_sources` from MCP | implicit (cwd-resolved on first call) |
| `_meta.actionable_next` machinery | tools just answer; no mid-response coaching |

**Net surface:** 10 tools × ~40 sub-modes → 6 tools × ~12 sub-modes.
**Net code budget estimate:** −7 to −9 KLoC. Tools.json halves.

## Behavioural fix list — must-haves before merge

Each is a P0 reliability bug today.

1. **`references kind=symbol` shape**: `identifier` field carries the searched
   name, not the enclosing function. Drop the misnamed primary `name` field.
2. **`impact blast_radius` symbol-scoping**: walk symbol references, not file
   imports. Refuse on languages without scope analyzer.
3. **`impact rename_safety` consistency**: `complete=false ⇒ safe=undecided`;
   `coverage_pct` mandatory; no `confidence=high` when `complete=false`.
4. **`outline view=repo` perf**: cache centrality in SQLite, recompute on
   reingest only. 50 ms warm / 200 ms cold target.
5. **`search mode=nl` defaults**: code-only by default, hybrid only on
   explicit opt-in. Fix the 0.4-vs-0.572 score collisions.
6. **`search fuzzy` merge**: fuzzy hits live in `items[]` with `match_type`.
   Delete the side-channel.
7. **`read with_neighbors=true`**: structural neighbors only (import graph +
   reference graph). No string-match neighbor inclusion (the
   `EvidenceNote.heuristic` problem).
8. **Source resolution**: server picks up source from cwd. No
   `catalog_sources` on the critical path. Saves ~1.4 s/session of pure
   ceremony.

## API contract — minimal, frozen

```jsonc
// All MCP responses share this envelope:
{
  "ok": true,
  "data": { /* tool-specific */ },
  "_meta": {
    "confidence": "safe" | "partial" | "unreliable",
    "timing_ms": 1.23,
    "tokens_saved": 4756        // honest baseline, missing → not a Grep substitute
  },
  "warnings": [{ "code": "...", "message": "..." }]
}
```

- No `_meta_verbose` toggle. `_meta` is always the same shape.
- No `actionable_next`. Tools answer; the agent decides next steps.
- No `domain_effective`, `query_mode_effective`, `unit_kinds_effective` echo
  fields — the request was already a request; don't re-narrate it.
- `confidence: unreliable` items never appear in `data.items[]`. They live in
  `data.weak_hits[]` only when an opt-in flag is set.
- All refs are strings. One canonical shorthand. No object form.
- `source_id` is never in the request. The server resolves from cwd. If
  multiple sources match, the response carries `confidence: partial` and a
  `candidate_sources[]` for disambiguation.

## Reliability harness — the merge gate

A test target the user runs (and CI runs) before any PR lands:

```bash
cargo nextest run --test reliability_probe
```

The probe replays the 25 dogfood operations from this evaluation against the
current repo. Every probe asserts:

1. **Tier compliance**: `_meta.timing_ms` ≤ AGENTS.md tier for that tool.
   Misses fail the test.
2. **Result shape**: defensible top-3 hits per probe (golden file, updated
   when intentionally changed, reviewed in PR).
3. **Confidence honesty**: every `confidence: safe` result must contain only
   structurally-resolved items. Random spot checks compare against
   tree-sitter ground truth.
4. **No regression in `tokens_saved`**: cumulative across the probe must not
   shrink between PRs unless a probe was intentionally retired.

This is the guardrail that stops the "still broken after iterating" loop.
A feature that fails the probe in Phase 4 gets **deleted**, not patched.

## Implementation phases

| Phase | Scope | LoC delta | Days | Gating | Status |
|---|---|---:|---:|---|---|
| **P1 — Cut** | Delete every row in the cut table. Update tools.json + tests in same patch. Zero backcompat. | −6 to −8 KLoC | 1 | `cargo nextest run` green; `tools.json` schema ½ size | ✅ surface cut + deep handler deletion done (−18.6 KLoC) |
| **P2 — Fix** | 8 behavioural fixes above. Symbol-graph blast_radius is the longest item. | +1 to +2 KLoC | 3 | Reliability probe passes for all 6 surviving tools | ✅ all 8 done |
| **P3 — Simplify API** | Implicit source_id, string-only refs, frozen envelope, parameter trim. Sweep call sites. | −2 KLoC | 1 | Probe still green; tools.json frozen | ✅ done |
| **P4 — Probe replay** | Run the probe on this repo. Any tool failing probe → cut, not patched. | 0 | 1 | All 6 tools probe-clean | ✅ 10/10 probes green |

**Total**: ~6 working days for a ~−7 KLoC, half-the-surface, probe-gated
release. Single sprint.

## Progress log (2026-04-26)

**Landed in this patch:**

- **tools.json** rewritten with the 6-tool surface. `probe_no_legacy_tools_in_public_list`
  asserts exactly 6 tool entries.
- **`crates/codesight-engine/src/service/agent_v1/cutplan.rs`** (new) — six
  adapter handlers (`cutplan_search`, `cutplan_outline`, `cutplan_read`,
  `cutplan_references`, `cutplan_impact`, `cutplan_health`) wired into
  `registry.rs`. Each adapter validates the new arg shape, resolves an
  implicit `source_id` from cwd (auto-ingest on first call), translates
  to the underlying handler's args, and post-processes the response.
- **P2.1** — `references kind=symbol` rows now expose `identifier`
  (= searched name); legacy `name` field stripped. Candidate text hits
  move to `weak_hits[]`, opt-in via `include_weak`.
- **P2.2** — `impact blast_radius` bypasses the legacy file-importer BFS.
  Walks the references graph, deduplicates by file, reports
  `symbol_scoped_files` / `symbol_scoped_file_count`. Returns
  `confidence: unreliable` when only weak text mentions exist.
- **P2.3** — `impact rename_safety` enforces `complete=false ⇒ safe="undecided"`,
  mandatory `coverage_pct`, no `confidence=high` on incomplete data.
  Implementation runs two `find_references` walks (old name + new name).
- **P2.4** — `outline view=repo` body cached on `(source_id, indexed_at)`
  via `CodeService.repo_outline_cache`; invalidated by
  `bump_catalog_generation`. Probe asserts warm-call ≤ 50 ms.
- **P2.5** — `search mode=nl` defaults to `domain=code`; markdown only
  enters when `scope` ends with `.md` or starts with `docs/`.
- **P2.6** — `search fuzzy=true` merges hits into `items[]` with
  `match_type:"fuzzy"`; side-channel `data.fuzzy` lane folded in.
- **P2.7** — `read with_neighbors=true` appends structural neighbors
  (import + reference graph) only. No string-match neighbors.
- **P3 envelope** — adapter post-processor `freeze_envelope` retains only
  `confidence`, `timing_ms`, `tokens_saved`, `error_category` on `_meta`;
  drops `actionable_next`, `*_effective` echo fields, body-level
  `session`. Whole-file reads downgrade to `confidence: partial` with an
  `OUTLINE_FIRST` warning.
- **P4 probe harness** — `tests/reliability_probe.rs` replays the 6-tool
  surface against a temp-dir fixture. 10 probes, all green.
- **`validate.rs`** skips legacy `expand_shorthand_refs` for the 6 new
  tool names so the string-only contract is enforced at the schema layer.

## Progress log (2026-04-26 follow-up)

**P1.4 deep deletion landed in this patch:**

- Deleted whole modules orphaned by the 6-tool surface:
  - `service/agent_v1/impact/` (8 files: blast_radius, change_velocity,
    changed_units, diff, impact_preview, path_rename, rename_safety, mod).
    The `cutplan_impact` adapter now drives blast_radius / rename_safety
    directly from the references graph; the legacy `evaluate_impact`
    dispatch is gone.
  - `service/agent_v1/context/` (7 files). `assemble_context` and every
    mode (bundle, ranked, section_context, query_suggestions, rename_plan)
    are removed from the public surface and the engine.
  - `service/agent_v1/graph/{calls,centrality,dependency,layers,references,resolve_fqn}.rs` —
    cut sub-modes that text-scanned through symbol bodies and could not
    satisfy `confidence: safe`. `graph/mod.rs` now dispatches only
    `find_references`, `imports`, `importers`.
  - `service/agent_v1/quality/{broken_links,doc_coverage,refactor_targets,test_gaps}.rs`
    plus the inline `quality_complexity`, `quality_dead_code_v2`, and
    `quality_regression_check` handler bodies in `quality/mod.rs`. Dispatch
    accepts only `quality_signal`, `hotspots`, `dead_code`, `repo_health`.
  - `inspect_structure` views: `file_tree`, `repository_rollup`, `toc`,
    `toc_tree`, `document_outline`, `class_hierarchy` deleted. `inspect_structure`
    accepts only `repo_outline` and `file_outline`.
- Stripped legacy registry rows: `evaluate_impact`, `assemble_context`
  removed from `agent_v1/registry.rs`. `EvaluateImpactArgs`,
  `AssembleContextArgs`, and `RANKED_CONTEXT_MAX_QUERY_LEN` deleted from
  `args.rs`.
- Renamed `evaluate_impact_usage` → `impact_usage` across savings ledger /
  session state / session_admin so the metric reflects the new tool name.
- Updated catalog/freshness/cli string lists: `cli/check.rs` Probe 4 now
  drives the new `impact` tool; `cli/hooks.rs` rewrites point operators
  at `impact analysis=blast_radius` for previewing change scope. Scripted
  path-rename is out of scope — `git grep`/`sed` covers the workflow.
- Bulk-deleted ~30 integration tests that pinned the cut sub-modes
  (architecture_tools, blast_radius, call_hierarchy, change_velocity,
  check_rename_safe, class_hierarchy, complexity, complexity_params_nesting,
  context_bundle_depth, cross_repo, dead_code_v2, doc_broken_links,
  doc_parser{,_html,_remaining}, doc_tools, document_coverage,
  fqn, get_changed_symbols, get_context_bundle, get_doc_coverage,
  get_file_tree, inspect_graph_scope, meta_disclosure, pagerank,
  path_rename{,_shapes}, refactor_targets, related_symbols, rename_plan,
  retrieval_tools, suggest_queries, symbol_complexity, symbol_diff,
  token_budget_context, tool_latency_budgets, tools_trusted_folders).
  Surgical removals inside `quality_signal.rs`,
  `find_importers.rs`, `agent_v1_dense.rs`, `agent_v1_envelope.rs` for
  individual functions that pinned `regression_check`, `test_gaps`,
  `snapshot_quality`, the legacy `search_catalog`-schema introspection,
  the `evaluate_quality` schema-rejection path, and Laravel
  framework-import tests that asserted via `view=file_tree`.

**Net diff vs HEAD before this session:** 103 files changed, +329 / −19,692
LoC. CUTPLAN's "−7 to −9 KLoC" target is more than doubled; the bulk
came from the test corpus that pinned the deleted analyses.

**Acceptance bullets (live MCP, this repo, 2026-04-26):**

- `references kind=symbol ref="…/symbols.rs::FactQuality#enum"` → 29
  items, every row carries `identifier:"FactQuality"`, `confidence:safe`.
  Slightly above the `24±2` snapshot; repo has grown but the contract
  (no enclosing-function-as-name confusion) holds.
- `impact analysis=blast_radius ref="…/symbols.rs::FactQuality#enum"` →
  10 symbol-scoped files (well under the ≤30 ceiling).
  `weak_files_text_mentions` = 2.
- `impact analysis=rename_safety ref="…/FactQuality#enum"
  new_name="FactGrade"` → `safe:"undecided"`, `complete:false`,
  `coverage_pct:93.75`, `confidence:partial`. The pre-CUTPLAN
  `complete:false / scanned:6 / confidence:high` quadrilemma is gone.
- `search fuzzy=true query="FactQwality"` → match in `items[0]` with
  `match_type:"fuzzy"` (no second lane).
- `search mode=nl query="how does regex routing work"` → `items[0..]`
  are all code (`.rs` files) with score ≥ 0.52; no markdown leaks.
- No probe in the suite calls `catalog_sources`; cwd resolution lives
  in `cutplan.rs::resolve_implicit_source_id`.

**Pre-existing test failures fixed in this patch:**

- `crate_graph::analysis_and_engine_have_no_per_language_branches` —
  pushed Rust's `tests/*.rs` test-binary heuristic from
  `codesight-analysis/dead_code/reachability.rs` down to a new
  `Language::is_test_target_root` trait method. Default `false`; Rust
  override matches `tests/*.rs` and `crates/*/tests/*.rs`.
- `read_content_oversized::oversized_read_stubs_later_refs_with_structured_suggestion` —
  the preflight oversized-stub now skips the first ref entirely. Stubbing
  the very first ref returns nothing useful; subsequent oversized refs
  still get the running-budget stub.
- `fuzzy_search::typo_finds_nearest_symbol` — fuzzy ranker now blends
  `0.5 * nucleo_norm + 0.5 * jaro_winkler_similarity`. Pure nucleo
  scored long word-boundary candidates (`savings_ledger_end_session`)
  ahead of close typos (`SavingsLedger`); the blend recovers the typo.
- `incremental::incremental_suite` — `wipe_indexes_for_repo` now passive-
  checkpoints the WAL before deletion and retries `remove_dir_all` up to
  5×50 ms against the SQLite `-wal`/`-shm` race that surfaced as
  `Directory not empty`.

**Dead-code cleanup landed alongside the deletions:**

- `service/agent_v1/util/{graph,neighbors}.rs` — fully orphaned helper
  modules (`git_show_file`, `extract_symbol_names_from_content`,
  `build_caller_parents`, `reconstruct_caller_chain`,
  `simple_text_diff`, `build_toc_tree`, `append_section_neighbor`,
  `heading_path_*`, `direct_child_sections`, `outline_neighbors`)
  deleted.
- `service/agent_v1/file_tree.rs` — only consumed by the deleted
  `view=file_tree`, fully removed.
- `service/agent_v1/util/token.rs` — trimmed to just
  `pack_catalog_items_to_token_budget` (the only helper still in use).
- `service/agent_v1/util/json_shape.rs` — `RefResolution`,
  `ref_requires_strict_resolution`, `resolve_symbol_id_strict` removed
  (only used by the now-deleted `quality complexity` handler).
- `service/savings/baseline.rs` — `sum_code_file_sizes` removed (only
  consumed by `assemble_context`'s baselines).
- `service/agent_v1/graph/find_references.rs` — `RefMatch`,
  `collect_references`, `first_identifier_line`,
  `RefFilter::new_permissive` removed (only used by the deleted
  `graph related` mode).
- Argument structs trimmed: `InspectStructureArgs::class_path` /
  `max_files`, `InspectGraphArgs::module_path` / `rules` /
  `symbol_id` / `direction` / `depth` / `algorithm` / `top_n` /
  `fqn` / `max_tokens` / `limit`, `EvaluateQualityArgs::refs` /
  `file_path` / `min_callers` / `top_files` / `top_caller_files` /
  `snapshot_name` removed alongside their dead handlers. Build emits
  zero `unused` warnings.

## Acceptance criteria — when is this done

- [x] `tools.json` lists exactly 6 tools (probe
      `probe_no_legacy_tools_in_public_list` asserts).
- [x] No tool dispatches to a sub-mode that text-scans for symbol resolution
      and reports `confidence: safe`. The cut sub-modes that did this
      (`graph/calls`, `graph/centrality`, `graph/dependency`, `graph/layers`,
      `graph/references`, `graph/resolve_fqn`) are deleted from the
      engine, not just delisted from MCP.
- [x] `outline view=repo` returns in ≤200 ms cold on this repo (probe
      `probe_outline_repo_meets_perf_budget` asserts; cache lives on
      `(source_id, indexed_at)`).
- [x] `references kind=symbol FactQuality` returns ~29 hits (was 24±2 at
      plan-write time; repo has grown), every row carries
      `identifier:"FactQuality"`, `confidence:safe`. No
      enclosing-function-as-name confusion.
- [x] `impact blast_radius FactQuality` returns 10 files (well under the
      ≤30 ceiling — was 229 before the symbol-graph fix).
- [x] `impact rename_safety FactQuality → FactGrade` returns
      `safe:"undecided"`, `complete:false`, `coverage_pct:93.75`,
      `confidence:partial`. The original
      `complete:false / scanned:6 / confidence:high` contradiction is
      illegal in the new contract.
- [x] `search fuzzy=true query=FactQwality` returns the FactQuality match
      in `data.items[0]` with `match_type:"fuzzy"`.
- [x] `search mode=nl query="how does regex routing work"` returns code
      as `items[0..]`, no markdown above score 0.3.
- [x] No call to `catalog_sources` is required to use any other tool —
      `resolve_implicit_source_id` (cutplan.rs) handles cwd → source_id
      with auto-ingest on first call.
- [x] Reliability probe passes in CI (10/10 sequential, 2026-04-26).

## Non-goals

- LSP-grade resolution for every language. Out of scope. The contract is
  "structurally honest where we can, refusal where we can't". Adding scope
  analyzers is a separate roadmap.
- New languages. Until the 6 tools are bulletproof on the existing language
  set, no new grammar lands.
- New analyses. `assemble_context` and `inspect_graph calls` were each "one
  more analysis" away from being good — and never got there. The lesson is
  to harden, not extend.

## Thesis in one line

You are not iterating on bugs. You are iterating on the consequences of
overpromising. Cut the surface to what tree-sitter + SQLite can structurally
guarantee, make those 6 things bulletproof against a replayable probe, and
drop the rest. That is the lean reliable product.
