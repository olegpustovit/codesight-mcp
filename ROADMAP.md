# codesight-mcp ROADMAP

Derived from a hostile-dogfood evaluation session (~150M tokens of cumulative
traffic, 7 realistic tasks, 773 `search_catalog` calls, 225 `inspect_structure`
calls, 178 `read_content` calls). Ordered by *trust impact first*, then
*value-per-session second*, then polish.

The goal is a premium retrieval layer an agent can rely on without cross-
checking with `rg`. Today it's a good grep wrapper with a dangerous structural
layer. Every P0 below is about closing that gap.

## Current dogfood checkpoint — 2026-04-23

Verified on this repo before making corrections:

- `analysis_health`: `unresolved_use_statements=0`,
  `workspace_crate_edges=171`, `reachability.roots=12`.
- `repo_health`: `dead_code_pct=21.6`, `dead_count=118`, `cycle_count=1`,
  `navigability=0.405`, `change_safety=0.653`.
- `inspect_graph graph=importers` still returns zero importers for
  `crates/codesight-engine/src/service/agent_v1/catalog_search/mod.rs`.
- `find_references identifier=Language` still returned `text_match` rows with
  `line: 0` before this pass.
- `blast_radius` still returned zero risk for an unresolved structured ref
  before this pass.

Corrections landed in this pass:

- `find_references` fallback rows now carry a real source line and use
  `kind: "file_body_match"` instead of opaque `text_match line:0`.
- `blast_radius` now returns `NOT_FOUND` for unresolved structured refs that
  include `path`, `start_line`, or `symbol_name`.

---

## P0 — Trust-breakers. Ship-blocking, must fix first.

These produce **silent wrong answers** that cost users real work. Until they
land, the structural analyses should be marked `experimental` in
`tools.json` and given a warning on every response.

### P0.1 — Rust module-tree edges in the import graph

**Symptom.** `inspect_graph graph=importers` on
`crates/codesight-engine/src/service/agent_v1/catalog_search/mod.rs` returns
`importer_count: 0`. That file is the parent of the `search_catalog` tool —
demonstrably live. The graph does not walk `mod foo;` declarations, only `use`
statements. Every module-tree-only-reached file appears unreachable.

**Blast radius.** Every `dead_code` verdict for a Rust file is suspect.
`overall_risk_score` in `blast_radius` underreports. `reachability.roots=12`
for 546 files is a symptom, not a fact.

**Fix.**
1. During `indexing::code_index` construction, for every Rust file scan top-
   level `mod X;` / `mod X { ... }` nodes. Emit a
   `ImportEdge { kind: ModuleTree, from: parent_file, to: child_file }` into
   the graph.
2. Teach `codesight-analysis/src/graph/*` to treat `ModuleTree` edges as
   reachability edges (but not as "imports" for the `imports` tool — keep
   the distinction for UX).
3. Add a `tests/self_dogfood.rs` assertion:
   ```rust
   let importers = importers_for("crates/codesight-engine/src/service/agent_v1/catalog_search/mod.rs");
   assert!(!importers.is_empty(), "module tree edges regressed");
   ```

**Done when.** `evaluate_quality analysis=dead_code` run on this repo
produces zero entries from `src/`, `crates/*/src/`, and any file reachable
via `mod` declarations from a `lib.rs` / `main.rs`.

### P0.2 — Refs API must error on unresolved input

**Symptom.** `evaluate_impact analysis=blast_radius` with
`{path, symbol_name, start_line}` returns `risk: 0.0, 0 dependents` when the
tuple doesn't resolve — instead of failing. Switching to `stable_id` returns
18 dependents. Silent zero-risk is strictly worse than an error.

**2026-04-23 correction.** `blast_radius` now rejects unresolved structured
refs with `NOT_FOUND`. Remaining work is to move this from a local
`blast_radius` guard into shared `refs` resolution for every strict-ref tool,
and to remove legacy name-shaped `stable_id` tolerance from tests/callers.

**Fix.**
1. In `ValidatedArgs` for every tool that takes `refs`: after validation, run
   resolution. If `resolved_count < refs.len()`, return
   `ToolError::NotFound { unresolved: [ref_repr...] }` with category
   `not_found` — never a zero-result success envelope.
2. Add an "unresolved tolerance" flag (`allow_unresolved: true`) for tools
   that legitimately tolerate misses (e.g., `read_content` batch reads — but
   those already stub truncation, keep as-is). Default must be strict.
3. Echo which lookup path was used (`resolution: "stable_id" | "symbol_name+path+line" | "name_only"`) in `_meta` so agents can debug.

**Done when.** The same ref that returns 18 dependents via `stable_id`
either returns 18 via the other shapes or returns `NOT_FOUND`. Never 0.

### P0.3 — Trait-dispatch awareness in rename / blast_radius

**Symptom.** Renaming `Language::extract_symbols` reported `safe: true,
conflicts: []`. In Rust that rename breaks every `impl Language for X`
block. The tool is checking name collisions, not semantic coherence of a
rename across a trait hierarchy.

**Fix.**
1. In `codesight-language-support/src/traits.rs` extraction, record, for
   each Rust function symbol, whether it is a `trait` method declaration or
   an `impl Trait for Type` method. Store the link
   `impl_method → trait_method`.
2. `rename_safety` on a trait method must enumerate every linked impl
   method and include them in the returned `required_edits` list. Do not
   mark `safe: true` unless the caller explicitly passes
   `include_impls: true` and all impls are listed.
3. `blast_radius` on a trait method: include the file of every `impl
   Trait for X` block as a direct dependent (depth 1), not just call sites.
4. Extend to other languages with equivalent dispatch — at minimum:
   - Go: interface method → all types satisfying the interface
   - Python: abstract/Protocol method → overriders
   - TS: interface method → implementers

**Done when.** `rename_safety` on `Language::extract_symbols` returns a
non-empty `required_edits` listing every `impl Language for X` method site.
`safe: true` only when the new name also has no collision AND caller opted
into the impl rewrite.

### P0.4 — Strip "text_match line:0" reference noise

**Symptom.** `inspect_graph graph=find_references identifier=Language`
returns ~80 entries of `{kind: "text_match", line: 0}` mixed with real
refs. These are "this substring appears somewhere in this file." They are
not references. They pollute results and poison any downstream analysis
that counts them.

**Fix.**
1. Stop emitting `text_match line:0` immediately. If a file-level fallback is
   still needed for compatibility, it must carry the real matching line and a
   distinct provenance kind (`file_body_match`) so downstream code never counts
   it as a semantic graph edge.
2. Next, split the response into `semantic_references` and
   `candidate_text_hits`, or emit a `PARTIAL_REFERENCES` warning when only the
   semantic subset is trustworthy. Agents must be able to exclude candidates
   without string-matching `kind`.
3. Separate concerns long-term: if an agent wants grep-style substring hits,
   it uses `search_catalog query_mode=regex`. `inspect_graph` is for the graph.
4. Every semantic reference carries `{file, line, column, enclosing_symbol,
   call_chain?}`. No zeros, no nulls.

**Done when.** `find_references identifier=Language` returns only
hits with `line > 0`, no row uses `kind: "text_match"`, and every row exposes
machine-readable provenance: `definition`, `symbol_body_match`,
`file_body_match`, or future language-specific reference kinds.

### P0.5 — Premium release gate for structural truth

**Gap found in this pass.** The roadmap lists the individual graph fixes, but
does not define a release gate. That makes it possible to ship "premium" while
the self-dogfood health still says `dead_code_pct=21.6`, `dead_count=118`,
`cycle_count=1`, `reachability.roots=12`, and `navigability=0.405`.

**Fix.**
1. Add a `codesight self-check --premium-gate` path that runs the P0 dogfood
   probes plus `analysis_health`, `repo_health`, `quality_signal`, and
   targeted latency checks.
2. Fail the gate while this repo has module-tree false negatives, dependency
   cycles from registry fan-out, `line:0` references, unresolved strict refs
   returning success, or fabricated token savings.
3. Publish the gate thresholds in one place, linked to
   `crates/codesight-analysis/src/thresholds.rs`, so "premium" is a
   measurable contract rather than a marketing word.

**Done when.** A release candidate cannot pass with current self-dogfood
numbers, and the failure tells the next engineer the first concrete tool call
to run.

---

## P1 — Correctness of high-value features.

These are features users are paying the index for. They're not dangerous,
but they're wrong often enough to undermine the premium positioning.

### P1.1 — Truthful token-savings accounting

**Symptom.** `repository_rollup` returns a 10-line directory map and
reports `tokens_saved: 938318`. Cumulative `tokens_saved_total:
152,103,868`. The baseline is fictional (assumes the agent would have
read the entire corpus to get the answer).

**Fix.**
1. Replace the synthetic baseline with a **per-call counterfactual** per
   tool:
   - `read_content symbol`: baseline = full-file read of same path.
   - `inspect_structure file_outline`: baseline = full-file read.
   - `search_catalog`: baseline = `rg -n` output size for the same
     query, capped at a reasonable top-N.
   - `assemble_context`: baseline = sum of full-file reads for included
     refs.
   - Tools that don't have a sensible baseline (`repository_rollup`,
     `catalog_sources`, `manage_indexes`): report `tokens_saved: null`,
     not a fabricated number.
2. Distinguish `tokens_saved_vs_full_file` (honest) from
   `tokens_saved_vs_index_dump` (marketing). Ship the honest one by
   default; keep the other behind `_meta_verbose` if anyone still wants
   it.
3. Session `tokens_saved_total` becomes the sum of honest per-call
   values. Expect this to drop by ~90%. That's fine — a trustworthy
   15M/session is more valuable than a bullshit 150M.

**Done when.** A calibration test: for a known set of tool calls,
`tokens_saved` ≤ `len(file) / 4` per read, and the cumulative is within
~20% of a measured "what would `rg` + `cat` have returned" baseline.

### P1.2 — NL search: deduplicate and downweight tests

**Symptoms.**
- Same symbol appears twice in a result set (`resolve_imports_to_files`
  at 0.516 and 0.3) — the hybrid ranker fuses symbol-layer and section-
  layer hits without merging on `stable_id`.
- "How are imports resolved" returned 5 of 8 test functions before real
  code.

**Fix.**
1. In the hybrid fusion layer (`codesight-retrieval` ranker), merge hits
   with the same `stable_id` using max-score, not concat.
2. Add a default penalty (multiplier ~0.4) for symbols whose file
   matches `Language::is_test_file(path)` or sits under a conventional
   tests dir. Gate behind `include_tests: bool` (default `false` for
   `natural_language` mode, `true` for `keyword`/`regex`).
3. Add an assertion test on this repo: query "how are imports resolved"
   must place `extract_imports` / `resolve_import_edge` / `resolve_rust_specifier`
   in the top 3 before any `tests/*.rs` function.

### P1.3 — `repo_outline` should be an outline

**Symptom.** The "outline" returns the alphabetically-first 40 symbols
from one subdirectory. Useful signal (`most_central_symbols`) is in the
tail.

**Fix.** Redesign the payload (bump in place — zero backcompat per
AGENTS.md):
```json
{
  "workspace": {
    "crates": [
      { "name": "codesight-analysis", "files": 34, "public_symbols": 189, "central_files": [...] },
      ...
    ]
  },
  "top_modules_by_centrality": [ ... 10 ... ],
  "top_modules_by_loc":        [ ... 10 ... ],
  "entrypoints": [ ... detected main / lib / bin ... ],
  "languages":  { ... }
}
```
No alphabetical symbol dump. If the user wants symbols, that's
`file_outline` / `search_catalog`.

**Done when.** A cold agent running one `repo_outline` call can name the
crates, their roles, and the top 3 files to read first.

### P1.4 — Fuzzy search that actually fuzzes

**Symptom.** `dispatch_tol` with `fuzzy=true` returned `DISPATCH_SEM`,
`DISPATCH_CONCURRENCY` at the same score as the real fuzzy target
`dispatch_tool`, all labelled `match_type: "exact"`.

**Fix.**
1. Run the fuzzy lane independently of BM25, not as an augmentation.
2. When `fuzzy=true`, the ranker returns two lanes: `exact: [...]` and
   `fuzzy: [...]`, labelled. Agent picks.
3. Fuzzy lane ranks by trigram-Jaccard *plus* Levenshtein distance on
   the normalized compare key, not by BM25 word-match.
4. If `fuzzy=true` and an exact-match hit exists for the same token,
   drop fuzzy candidates that only differ from the exact hit by ≥2
   edits — reduces near-duplicate noise.

**Done when.** `search fuzzy=true query="dispatch_tol"` puts
`dispatch_tool` first in the `fuzzy` lane.

### P1.5 — `assemble_context` as a real ticket bundle

**Symptom.** Given `dispatch_tool`, returned only the function source +
its imports. For a ticket kickoff you'd want the handler table it
iterates (`AGENT_V1_HANDLERS`), at least one handler implementation, the
service struct `CodeService`, the error type `ToolError`. Zero tail
context.

**Fix.**
1. `context_bundle` accepts a `depth` knob (default 2):
   - depth 0: just the symbol + imports (today's behavior).
   - depth 1: + types referenced in the signature + types constructed
     in the body.
   - depth 2: + one representative caller + one representative callee
     per direct edge.
2. Cap with `max_tokens`; greedy pack by centrality (PageRank) of the
   candidate symbols.
3. Add `include_tests: bool` (default true) — include the nearest test
   fixture.
4. Fix the import rendering (see P1.6).

**Done when.** `context_bundle` on `dispatch_tool` at depth=2 returns:
`dispatch_tool`, `AGENT_V1_HANDLERS`, one handler body, `CodeService`,
and one representative test. All real Rust syntax.

### P1.6 — Render imports in the source language

**Symptom.** `assemble_context` renders Rust imports as
`"import { Value } from 'serde_json'"`. An 80%-Rust repo should not
produce JavaScript-shaped import strings.

**Fix.**
1. Add `Language::render_import_line(&self, edge: &ImportEdge) ->
   String` with JS as one implementation. Rust: `use serde_json::Value;`.
   Python: `from X import Y`. Go: `import "path"`. etc.
2. Route through the registry — no hardcoded JS string in
   `context_bundle`.
3. Add parser-roundtrip test per language: the rendered line, fed back
   through the import extractor for that language, must recover the
   original `ImportEdge`.

---

## P2 — UX / ergonomics pain points.

Nothing wrong semantically, but friction high enough that agents bail
to `rg` (126 fallbacks recorded in session stats). Closing these raises
retention.

### P2.1 — Differentiate "no symbol matches, only doc hits"

When `CodeBackend` is searched and returns 5 hits all from `.md` files,
emit `WARNING: SYMBOL_NOT_FOUND_IN_CODE — all 5 hits are in documentation;
symbol likely refactored away`. Simple to add, huge trust win.

### P2.2 — Auto-ingest docs when `domain=hybrid` default would otherwise be code-only

`DOC_INDEX_MISSING` warning on natural-language queries is user-
hostile. Options:
1. Auto-trigger `ingest_sources domains=docs` on first NL query, block
   up to `max_wait_ms` (default 5000), continue on timeout with a
   warning. Most repos have under 50 markdown files — fast.
2. Or — on session start, if `domains=["code"]` only, surface a single
   `PROMOTE_TO_HYBRID` hint once, not on every query.

### P2.3 — Read efficiency coaching

Session stats show **40% of `read_content` calls are whole-file reads**
with 42 warnings. The hints are too passive.

**Fix.** When `read_content refs=[{kind:"file", no start/end}]` is
called and the file has a cached `file_outline`, return an
`OUTLINE_FIRST_HINT` warning including the outline inline (not just
advice), so the agent can retry with symbol refs in one round-trip.

### P2.4 — Cut the cost of `catalog_sources`

47 calls / 5.7 s in the session (120 ms avg). Used as a warmup ritual.
Two fixes:
1. Cache the response for 60 s per source root. Second call within
   window returns in < 5 ms.
2. Expose `catalog_sources lightweight: true` that returns only
   `{source_id, status, indexed_at, file_count}` — no language
   histogram, no capabilities array.

### P2.5 — Quality metrics: one score, one dimension breakdown

`quality_signal` returns `agent_readiness_score: 0.7076` alongside
`agent_readiness.score: 0.00006766`. Two scores that disagree by 4
orders of magnitude. Users can't act on this.

**Fix.**
1. Pick one top-level score. Remove the other.
2. If there are two distinct concepts, rename — `aggregate_readiness`
   vs `geometric_readiness` or similar, with a sentence on when each
   applies.
3. Collapse `agent_dimensions` and `agent_readiness` (which repeat the
   same four keys with slightly different numbers) into one block.

### P2.6 — Stable reindex cadence, fewer stale warnings

Session recorded ~130 `STALE_DATA_WARNING` events. The incremental
reindex hook fires on every Edit/Write, often during batches of
edits. Each fires a warning on subsequent queries until the reindex
drains.

**Fix.**
1. Debounce: collapse reindex events fired within 500 ms into one
   scheduled reindex.
2. Escalate the warning only when `staleness.indexing_started_ago_ms >
   500` *and* the query's `scope` overlaps an `in_flight` file. Don't
   warn for unrelated queries.

### P2.7 — File tree default usable for orientation

`file_tree` with no `path` returns 50 files and stops mid-crate.
Default should be 2 levels deep (crate + top modules), not a flat 50.
Symbol counts are useful; JSON structure nesting levels are noise at
this zoom.

### P2.8 — Per-tool `_meta.actionable_next`

After any analysis that flags a gap, emit `_meta.actionable_next: [{tool, args, reason}]`. Already done for `quality_signal.recommendations`
— generalize to `dead_code`, `hotspots`, `blast_radius`. Agents love
concrete pointers; users love audit trails.

---

## P3 — Polish, nice-to-haves.

### P3.1 — `evaluate_impact analysis=diff` between two refs in different files
Currently requires same-file. Broaden to any two symbol refs — useful
for before/after comparisons across commits.

### P3.2 — `inspect_graph graph=calls` should distinguish direct vs indirect
Today: one flat list. Add `depth` and `kind` (direct | via_trait_obj |
via_fn_ptr).

### P3.3 — `search_catalog` supports `unit_kinds=["doc_section"]` explicitly
Make doc domain a first-class filter instead of a `domain=hybrid`
implicit behavior.

### P3.4 — CLI for running the suite locally
`codesight self-check` that runs the P0 / P1 regression tests listed
above, usable as a pre-release gate.

### P3.5 — Structured `tools.json` versioning
Currently frozen as single contract. Add `capabilities_version` per
tool (e.g. `evaluate_impact v2 with strict refs`) so agents can detect
upgrade-incompatible changes.

### P3.6 — Budget enforcement on `evaluate_quality`
`quality_signal` ran 1.5 s. Add `max_wait_ms` and `partial_ok` so
agents can bound analysis cost on the caller side.

---

## Enforcement & CI

To prevent regression on the trust fixes:

1. **`tests/self_dogfood.rs`** — assert known-live paths are reachable
   (P0.1), assert refs resolve or error strictly (P0.2), assert trait
   rename enumerates impls (P0.3), assert zero `line:0` refs (P0.4).

2. **Calibration snapshot test** — capture a known directory's full
   index, run the P0/P1-touched tools, compare against frozen
   expected outputs.

3. **Token-savings audit** — periodic test that runs a synthetic
   session and asserts reported savings are within ±20% of a measured
   baseline (vs `rg` + `cat` equivalents).

4. **Fallback counter** — the session-stats
   `fallbacks_to_grep_or_bash` counter becomes a tracked SLI.
   Regression = any PR that raises the per-session median fallback
   count on the dogfood harness.

5. **Premium gate** — `codesight self-check --premium-gate` is the release
   blocker. It fails on P0 regressions, dishonest savings accounting,
   structural-health thresholds below target, or tool latency budget misses.

---

## Sequencing / sprint shape

- **Sprint 1 (P0 — must-ship):** P0.1 module edges, shared P0.2 strict
  refs, complete P0.4 reference provenance, and P0.5 premium gate. Mark
  structural analyses `experimental` until done.
- **Sprint 2 (trust, part 2):** P0.3 trait-dispatch, P1.1 honest token
  savings, P1.2 NL dedup + test downweight.
- **Sprint 3 (reshape):** P1.3 repo outline, P1.4 fuzzy, P1.5 context
  bundle depth, P1.6 language-aware imports.
- **Sprint 4 (retention):** P2.* ergonomics batch.
- **Sprint 5+ (polish):** P3.*

At the end of Sprint 2, the premium positioning is defensible. Before
that, the tool is a high-quality retrieval layer with landmines — ship
it as "beta" on structural analyses or pay reputational cost per wrong
rename.

---

## The one sentence

**Make the graph tell the truth, make the refs API refuse to guess,
and stop inflating the savings number.** Everything else is a matter
of craft; those three are what separate a grep-plus-plus from a
retrieval layer an agent trusts.
