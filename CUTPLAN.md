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
`manage_indexes` admin actions — **all gone from the MCP surface**.

CLI lives separately under `codesight`:

```
codesight index           # ingest / rebuild / status
codesight rename          # path_rename with apply
codesight check           # broken-link / doc-coverage gates for CI
```

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
`git log`), `path_rename` with `apply=true` (move to CLI as
`codesight rename`), `impact_preview` (overlap with blast_radius).

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
| `evaluate_impact path_rename apply=true` | CLI: `codesight rename` |
| `evaluate_impact impact_preview` | overlap with `blast_radius` |
| `evaluate_quality complexity` (single-symbol) | opt-in field on `read` |
| `evaluate_quality doc_coverage` | CLI: `codesight check` |
| `evaluate_quality broken_links` | CLI: `codesight check` |
| `evaluate_quality refactor_targets` | `health hotspots sort=refactor` |
| `evaluate_quality regression_check` | snapshot ritual unused |
| `evaluate_quality test_gaps` | `quality_signal verbose=true` |
| `assemble_context bundle` | `read with_neighbors=true` |
| `assemble_context section_context` | `read` with `section:` ref |
| `assemble_context ranked` | overlap with `search` |
| `assemble_context query_suggestions` | ignores query in practice |
| `assemble_context rename_plan` | overlap with `rename_safety` |
| `manage_indexes invalidate/delete/rebuild` | CLI: `codesight index` |
| `manage_indexes snapshot_quality` | snapshot ritual unused |
| `manage_indexes session_end` | trivial; `stats` covers it |
| `manage_indexes status` per source | `stats` global |
| `catalog_sources` as required first call | source resolved from cwd implicitly |
| `ingest_sources` from MCP | CLI: `codesight index` |
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

| Phase | Scope | LoC delta | Days | Gating |
|---|---|---:|---:|---|
| **P1 — Cut** | Delete every row in the cut table. Update tools.json + tests in same patch. Zero backcompat. | −6 to −8 KLoC | 1 | `cargo nextest run` green; `tools.json` schema ½ size |
| **P2 — Fix** | 8 behavioural fixes above. Symbol-graph blast_radius is the longest item. | +1 to +2 KLoC | 3 | Reliability probe passes for all 6 surviving tools |
| **P3 — Simplify API** | Implicit source_id, string-only refs, frozen envelope, parameter trim. Sweep call sites. | −2 KLoC | 1 | Probe still green; tools.json frozen |
| **P4 — Probe replay** | Run the probe on this repo. Any tool failing probe → cut, not patched. | 0 | 1 | All 6 tools probe-clean |

**Total**: ~6 working days for a ~−7 KLoC, half-the-surface, probe-gated
release. Single sprint.

## Acceptance criteria — when is this done

- [ ] `tools.json` lists exactly 6 tools.
- [ ] No tool dispatches to a sub-mode that text-scans for symbol resolution
      and reports `confidence: safe`.
- [ ] `outline view=repo` returns in ≤200 ms cold on this repo.
- [ ] `references kind=symbol FactQuality` returns 24±2 hits whose `identifier`
      field equals `"FactQuality"`. No enclosing-function-as-name confusion.
- [ ] `impact blast_radius FactQuality` returns ≤30 files (only files using
      the type), not 229.
- [ ] `impact rename_safety FactQuality → FactGrade` either returns
      `safe=true` with complete coverage, or `safe=undecided` with
      `coverage_pct` < 100. Never the current contradiction.
- [ ] `search fuzzy=true query=FactQwality` returns FactQuality in
      `data.items[0]`.
- [ ] `search mode=nl query="how does regex routing work"` returns code as
      `items[0..2]`, no markdown above score 0.3.
- [ ] No call to `catalog_sources` is required to use any other tool.
- [ ] Reliability probe passes in CI.

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
ship the rest as CLI subcommands or not at all. That is the lean reliable
product.
