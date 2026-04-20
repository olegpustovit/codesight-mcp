# FEEDBACK.md

Honest notes from one aggressive-code-sweep session (2026-04-19). All
observations are grounded in actual tool calls from that session — no
hypotheticals.

**TL;DR:** the retrieval primitives are great (outline-first, batched,
transparent token-savings). The *analysis* primitives confidently lie near
crate boundaries — `dead_code` flagged the entire protocol layer as
unreachable even though the binary's `main.rs` imports it directly. One
class of fix (workspace-crate + `pub use` edge resolution) eliminates most
of the confident-wrong verdicts in one shot.

---

## Where it rocks

### 1. `tokens_saved` is a real scoreboard, not vanity

Every call returns `_meta.tokens_saved`. During this session a single
`evaluate_quality analysis=hotspots` call reported **897,569 tokens saved**
vs. the equivalent native exploration. That is the number that would make a
team adopt this tool — show it on every call and the dopamine loop does the
selling.

### 2. `inspect_structure view=file_outline` with `file_paths=[…]` batching

One call, 5 files, 888k tokens saved. This is the single most valuable
retrieval primitive in the whole server — outline-first, then targeted
read. The alternative (`Read` each file end-to-end) would have pushed me
off a context cliff.

### 3. `evaluate_quality analysis=hotspots` + `refactor_targets` worked

Ranked, actionable, cross-file. I went from "find stupid code" → concrete
punch list in one call. The `caller_count` / `caller_files` / `cyclomatic`
triple is the right signal blend.

### 4. Stale-data transparency

`STALE_DATA_WARNING` + `files_in_flight` appeared exactly when expected
(right after Edit/Write). Didn't silently return pre-edit facts. The
PostToolUse auto-reindex is invisible-in-the-good-way.

### 5. `query_mode=regex` + `file_pattern` narrowing

Fast (sub-2 ms repeatedly).
`file_pattern="crates/codesight-storage/**/*.rs"` is the right shape.
Regex mode's auto-route-to-file-text-ignore-`unit_kinds` behavior is
documented and the `_meta.query_mode_effective` echo is gold for debugging.

### 6. `read_content` batch refs

`refs=[{kind: "symbol", …}, {kind: "file", start_line, end_line}, …]` in
one call. Paid off immediately when I needed to read six disjoint regions
in parallel.

---

## Where it sucks

### 1. CRITICAL: `dead_code` false positive nuked a production file

`evaluate_quality analysis=dead_code` flagged
`crates/mcp-rpc/src/server.rs` with **confidence 1.0, signal
`unreachable_from_live_roots`**. `src/main.rs` literally contains
`use mcp_rpc::{DispatchConfig, run_stdio}` and calls `run_stdio(...)` — it
is the entire JSON-RPC entry point. If I had trusted the tool I would have
deleted the protocol layer. Same for every symbol in that file: all listed
as dead_file at confidence 0.8.

Root cause guess: reachability is built from symbol-reference edges only,
and external-crate `use` statements to re-exported items in the binary's
`main.rs` aren't being followed. Workspace binary targets should be
automatic "live roots."

**This is the #1 bug to fix.** A confident-wrong dead-code verdict is worse
than no verdict.

### 2. `inspect_graph graph=importers` returns zero for cross-crate `use`

`file_path=crates/mcp-rpc/src/server.rs` → `importer_count: 0`. But
`src/main.rs::7` has `use mcp_rpc::{DispatchConfig, run_stdio};`. Same bug
family as (1): cross-crate `use` of a root-exported item isn't an edge.

### 3. Whole-file `read_content` dumps to a side file with an unreadable preview

When I batched three ~15 KB files through `read_content`, the response was
"Output too large (54.8KB)" with a "Preview" of JSON-escaped content
(`\"_meta\":…`), which is not usable by the model. The tool correctly
recommended `inspect_structure view=file_outline` — but the failure mode
should either:
- (a) return the first file fully and drop later ones with a clear per-ref
  marker,
- (b) auto-strip noisy `_meta` fields when over budget, or
- (c) not format the preview as escaped JSON.

### 4. Hotspots surfaces a repo-convention scripts dir as the #1 target

`scripts/generate_agent_v1_modules.py::main` → refactor_targets score
**2952**, the top hit. It's a one-shot codegen helper; of course it has 163
callers (it writes those files). `scripts/**`, `build.rs`, `tests/**`, and
`examples/**` need default exclusion or at least a score penalty,
otherwise the top hit is always noise.

### 5. `limit` defaults too low and silently truncates

`search_catalog` default `limit: 10` cost me a re-query when I actually
needed 26 `McpClient` matches. The `truncated: true` +
`truncated_by: ["limit"]` meta flag is clear, but defaulting to 10 for a
tool whose pitch is "structured retrieval" feels miscalibrated. 25–50
would be saner; keep 10 as an explicit knob.

### 6. Hook-level redirection sometimes requires more typing than the blocked tool

`Grep SymbolKind path=<one-file>` is shorter than
`search_catalog query="SymbolKind" query_mode=regex file_pattern="<one-file>"`
for the exact same intent. The blocking rule is correct in spirit ("don't
skip the index"), but for single-file single-symbol the index isn't
helping — there are no token savings, just friction. Consider a
pass-through for `Grep path=<file>` when the path is a single file.

### 7. No "wait for reindex" semantics

When `STALE_DATA_WARNING` fires, I just have to re-issue the query. A
`wait_for_fresh=true` flag — or an `ingest_sources action=wait` — would
remove the manual retry loop.

### 8. `refactor_targets` `caller_files` arrays are unbounded

Seven entries in the output had 30–75 `caller_files` each. That's the
bulk of the response token weight and 95% of it is "obviously common infra"
(e.g. `IndexStore.open`, 42 callers). `top_caller_files: N` parameter with
a sensible default would shave this enormously.

### 9. `_meta.tokens_saved` can be misleading for whole-file reads

When I read three whole files, `tokens_saved` was reported as `0` — fair,
since there was no retrieval win. But the tool's own `next_step_hint`
nudged to `inspect_structure` instead. That's the right nudge; it just
reads a little like "tokens_saved is 0 because you used us wrong" rather
than "because you asked for the full file." A phrasing tweak.

### 10. Language coverage for semantic analysis is uneven

This session stayed in Rust-land and everything worked. But note: when I
looked at `language_profile` in
`crates/codesight-analysis/src/agent_readiness.rs`, it had 40+ extensions
hand-coded (kotlin, scala, clojure, haskell, erlang, ocaml, fsharp, …).
None of those are in the workspace. Dead coverage = maintenance cost with
no payoff. Either drive the list from actual indexed-language stats, or
trim to the 15 that the test suite exercises.

### 11. Pre-existing test-suite rot (caught while doing this sweep)

`tests/quality_signal.rs` (8 tests) and `tests/tools_trusted_folders.rs`
(some tests) built `CodeService` via `CodeService::new(config)`, which
creates an empty `BackendRouter`. Every `ingest_sources` call then
SIGABRTs with `"no code-intelligence backend registered"`. These tests
were failing on clean `dev` before I touched anything. The cause is that
`test_support::make_service` exists and wires the tree-sitter backend, but
individual test files don't reach for it — so regressions are invisible
until you read the code. *Fixed this session*, but the class of bug is
"new test author reinvents `make_service` locally and the new copy lacks
the backend."

---

# Implementation plan

Ordered by blast-radius reduction, not by LoC.
Each phase is one PR unless noted; each phase has a concrete acceptance
gate that goes into CI so the regression doesn't crawl back.

## Phase 0 — Regression harness (land first)

**Problem.** Every issue below is testable, but the repo has no
"codesight-as-seen-by-itself" test. Issue #1 (dead-code false positive on
`mcp-rpc/src/server.rs`) has existed long enough to ship; that means
nobody is gating on the output. Fix that first or every other fix
regresses silently.

**Files to add.**

- `tests/self_dogfood.rs` — integration test that runs codesight against
  the working repo and asserts structural invariants.
- `tests/fixtures/dogfood/` — minimal synthetic workspace
  (binary crate + library crate + `pub use` re-export chain) so the
  assertions don't depend on the real tree's evolution.

**Concrete asserts.**

```rust
// tests/self_dogfood.rs
#[test]
fn dead_code_respects_binary_live_roots() {
    let svc = make_service(tmp.path());
    let sid = ingest(&svc, Path::new("tests/fixtures/dogfood"));
    let r = svc.dispatch("evaluate_quality",
        json!({"source_id": sid, "analysis": "dead_code"}));
    let dead: Vec<&str> = r["data"]["dead_files"].as_array().unwrap()
        .iter().map(|f| f["file"].as_str().unwrap()).collect();
    // Re-exported via `pub use` in the library; reached via `use lib::*` in main.
    assert!(!dead.contains(&"crates/lib/src/server.rs"),
        "regression: pub-use chain reachable from bin `main` marked dead");
}

#[test]
fn importers_resolves_cross_crate_use() {
    // Same fixture. Grab importers of crates/lib/src/server.rs;
    // must contain src/main.rs because it has `use lib::run_stdio;`.
}
```

Also add a **self-test**: `tests/self_dogfood_real.rs` that runs against
the repo root itself and asserts:

- `crates/mcp-rpc/src/server.rs` ∉ `dead_files`
- `inspect_graph graph=importers file_path=crates/mcp-rpc/src/server.rs`
  has `importer_count >= 1`.

Behind `#[ignore]` by default (so it doesn't run on every `cargo nextest
run --lib`), but CI runs it explicitly with `cargo nextest run --test
self_dogfood_real --run-ignored all`.

**Acceptance.** Both tests fail on `dev` today. After Phase 1 lands, they
pass. CI runs them on every PR.

**LoC delta.** +~150 (test + fixture).

---

## Phase 1 — Graph correctness (the CRITICAL fix)

**Problem.** The dependency/reference graph misses three kinds of edge:

1. **Workspace-crate `use`**: `use mcp_rpc::foo` from anywhere in the
   workspace should resolve to `crates/mcp-rpc/src/lib.rs`.
2. **`pub use` re-export walks**: if `lib.rs` does
   `pub use server::run_stdio`, a caller doing `mcp_rpc::run_stdio` should
   create an edge to `server.rs::run_stdio`, not just to `lib.rs`.
3. **Binary-crate `main.rs` as implicit live root**: every
   `[[bin]]` / `src/main.rs` / `src/bin/*.rs` is a live root by definition;
   nothing should be required to point *at* `main.rs` for it to count.

Today all three are missing, which is why `mcp-rpc/server.rs` — reached
by `main.rs::use mcp_rpc::run_stdio` → `lib.rs::pub use server::run_stdio`
→ `server.rs::run_stdio` — is flagged dead.

**Files to touch.**

- `crates/codesight-imports/src/imports/resolve.rs` — add workspace-crate
  resolution. Parse root `Cargo.toml` + member `Cargo.toml`s at ingest
  time to build a `crate_name → crate_root` map keyed on the `[package]
  name` of each workspace member.
- `crates/codesight-imports/src/imports/fallback/rust.rs` — during Rust
  import extraction, when the first segment of a `use` path matches a
  workspace-crate name, emit an edge to `<crate_root>/src/lib.rs`.
- `crates/codesight-extractors/src/extractors/…/rust` (wherever Rust
  queries live) — emit `pub use` re-export facts as a new
  `ReexportEdge(from_file, from_name, to_file, to_name)` fact.
- `crates/codesight-analysis/src/graph/…` (the import→edge wiring) —
  resolve `ReexportEdge` transitively when walking the dependency graph
  (with a cycle guard; re-export cycles exist in real code).
- `crates/codesight-analysis/src/dead_code/entrypoints.rs` — add every
  workspace `[[bin]]` target's source file to the live-root set
  automatically; remove any per-language heuristic that excluded them.
- `crates/codesight-analysis/src/dead_code/reachability.rs` — follow
  `ReexportEdge` during the reachability walk so "reached via re-export"
  isn't different from "reached directly."

**Sub-fix (same PR).** `inspect_graph graph=importers` should follow the
same re-export walk in reverse: "who imports file X" includes "who
imports the re-exporter of X." Expose this via an `include_reexports: bool`
flag (default `true`).

**Acceptance gates.**

- Phase-0 tests now pass.
- Regression test on the synthetic fixture: a `pub use` chain of depth 3
  still counts the terminal file as reachable.
- `evaluate_impact analysis=blast_radius` on a re-exported symbol must
  surface its *callers*, not the empty set.
- Existing `tests/find_dead_code.rs`, `tests/dead_code_v2.rs`,
  `tests/find_importers.rs` still pass (guardrail for no regression on
  intra-crate graph).

**LoC delta.** +~300, −~50 (remove special-case binary-heuristic code).
This is the single highest-leverage change in the plan.

---

## Phase 2 — Trait-impl live-root rule (sibling of Phase 1)

**Problem.** Even after Phase 1, a struct that exists only to satisfy a
foreign trait (e.g. `impl Drop for X { … }`) has no incoming reference
edge — Drop is called via vtable at runtime. Today the struct would be
flagged dead.

**Files to touch.**

- `crates/codesight-extractors/…/rust` — emit `TraitImplEdge(file, impl_target_path, trait_path)`.
- `crates/codesight-analysis/src/dead_code/entrypoints.rs` — any symbol
  that is the target of a `TraitImplEdge` where the trait is foreign
  (not defined in this repo) is a live root.
- Same for `impl Dispatcher for CodeService` in
  `crates/codesight-engine/src/rpc.rs` — the trait is foreign
  (`mcp_rpc::Dispatcher`), so `CodeService` should be reachable by virtue
  of being a trait impl even without explicit callers.

**Acceptance gate.**

- Synthetic fixture: crate with `impl Drop for Foo` and no other users of
  `Foo` — `Foo` must not be flagged dead.
- Real repo: `crates/codesight-engine/src/rpc.rs::<impl Dispatcher for
  CodeService>` is tracked as reachable.

**LoC delta.** +~100.

---

## Phase 3 — Response-shape cleanup

### 3a. Oversized `read_content` responses

**Problem.** When three ~15 KB file reads overflow the response budget,
the whole response goes to a side file and the "Preview" is JSON-escaped
(`\"_meta\": \{...`). That's unreadable.

**File to touch.**
`crates/codesight-engine/src/service/agent_v1/content.rs` (read_content
handler).

**Concrete change.**

- Per-ref trimming, not whole-response truncation: return refs in order,
  append each full; when the budget would be exceeded, stop and append
  a stub for every remaining ref:
  ```json
  {
    "kind": "file",
    "path": "...",
    "truncated": true,
    "reason": "token_budget_exceeded",
    "suggestion": {
      "tool": "inspect_structure",
      "args": {"view": "file_outline", "file_path": "..."}
    }
  }
  ```
- Emit a `TOKEN_HEAVY_READ` warning *per trimmed ref*, not one global one.
- Strip `_meta.next_step_hint` from the body when the response is already
  over budget — it's duplicative and expensive.

**Acceptance gate.** New test `tests/read_content_oversized.rs`: batch
five 20 KB files at budget 8k tokens; expect first file full, others
stubbed with the structured suggestion above; response parses as JSON
(not escaped).

**LoC delta.** +~60, −~30.

### 3b. `_meta.tokens_saved` phrasing

**Problem.** `tokens_saved: 0` for a whole-file read reads like "you used
us wrong." Rename to make the baseline explicit.

**File to touch.** `crates/codesight-engine/src/service/…` (wherever
`_meta.tokens_saved` is populated).

**Concrete change.** Split into two fields:

```json
"_meta": {
  "tokens_saved_vs_grep": 88942,       // when the baseline is "grep + cat"
  "tokens_saved_vs_full_read": 0,      // when you already asked for the whole file
  "retrieval_class": "whole_file_read"
}
```

`retrieval_class` is one of: `structural`, `targeted_range`,
`whole_file_read`, `metadata_only`, `search`. Agents learn: *zero-savings
is fine if `retrieval_class == whole_file_read`; it's a red flag if
`retrieval_class == search`*.

**Acceptance gate.** Unit tests on the MetaBuilder covering each class.

**LoC delta.** +~40, −~20.

---

## Phase 4 — Signal quality (hotspots / refactor_targets)

### 4a. Auto-exclude conventional dirs

**Problem.** `scripts/generate_agent_v1_modules.py::main` is the top
refactor_target with a score of 2952 — it's a codegen helper, not
production code.

**File to touch.** `crates/codesight-analysis/src/quality.rs` (hotspots
implementation) and the `refactor_targets` handler.

**Concrete change.** Add a `ConventionalDirs` constant:

```rust
pub const CONVENTIONAL_DIRS: &[&str] = &[
    "scripts/", "tests/", "examples/", "benches/",
    "build.rs", "fixtures/", "testdata/", "stress/",
];
```

Both hotspots and refactor_targets default-exclude files matching these
prefixes. Expose `include_conventional: bool` (default `false`) so an
operator can opt back in.

**Acceptance gate.** New test: synthetic workspace with a high-complexity
`scripts/gen.py` and a low-complexity `src/lib.rs`; assert that `lib.rs`
ranks above `scripts/gen.py` in `hotspots` output with default args, and
`gen.py` appears only when `include_conventional=true`.

**LoC delta.** +~40.

### 4b. Cap `caller_files` arrays

**Problem.** Several refactor_targets entries had 30–75 `caller_files`
each — those dominate response token weight and 95% is "common infra"
noise.

**File to touch.** `crates/codesight-analysis/src/…/refactor_targets.rs`.

**Concrete change.**

- New arg: `top_caller_files: usize` (default 10, max 50).
- When truncated, populate `caller_files_total: usize` alongside the
  trimmed array.
- Pick the top-N by: (a) shortest path depth, then (b) alphabetical, so
  output is deterministic.

**Acceptance gate.** Unit test: 100 synthetic callers → response has
10 listed + `caller_files_total: 100`.

**LoC delta.** +~30.

### 4c. Hotspot score-weighting for tests/docs

**Problem.** A test file with cyclomatic 19
(`tests/quality_signal.rs::session_end_reports_quality_regression`) ranked
above real production functions. Tests have inherently higher cyclomatic
(Given/When/Then branches) without being "refactor targets."

**Concrete change.** Multiply the score by 0.3 when the file is in
`tests/` or ends in `_test.py` / `.test.ts` etc. This is softer than 4a's
exclusion and keeps test quality visible without dominating.

**Acceptance gate.** Unit test: same fixture as 4a with the test included;
test ranks below prod even when its cyclomatic is higher.

**LoC delta.** +~20.

---

## Phase 5 — UX knobs (small, high-return)

### 5a. Raise `search_catalog` default `limit` from 10 → 25

**File to touch.**
`crates/codesight-engine/src/service/agent_v1/catalog_search/mod.rs` +
`crates/codesight-engine/src/tools.json` (update schema default).

**Acceptance gate.** Trivial unit test on the validated-args constructor:
default value is 25; explicit 10 still honored.

**LoC delta.** +~5, but the UX win is disproportionate.

### 5b. `wait_for_fresh` flag on read tools

**Problem.** When a reindex is in flight, the only recourse today is
resubmitting.

**Files to touch.**
`crates/codesight-engine/src/service/mod.rs` (`ReindexStateManager`) —
expose a `wait_until_idle(source_id, timeout_ms)` method; call it from
every read handler when `wait_for_fresh: true` is in args.

**Schema change.** Add to every read tool's argument schema:

```json
"wait_for_fresh": {
  "type": "boolean",
  "default": false,
  "description": "If a reindex is in flight for this source_id, block until it completes or max_wait_ms elapses."
},
"max_wait_ms": {"type": "integer", "default": 2000, "maximum": 30000}
```

**Acceptance gate.** Integration test: trigger reindex on a big fixture,
immediately query with `wait_for_fresh=true`; result must NOT carry
`STALE_DATA_WARNING`.

**LoC delta.** +~80.

### 5c. `top_caller_files` — see 4b (already covered).

---

## Phase 6 — Polish

### 6a. Consolidate extension → language mapping

**Problem.** The repo has *three* hand-coded extension maps:

- `crates/codesight-imports/src/imports/mod.rs::language_for_path` (18
  langs)
- `crates/codesight-analysis/src/agent_readiness.rs::language_profile`
  (40 langs, returns `LanguageProfile`)
- Implicit maps in each framework/extractor plugin.

**Concrete change.** One shared `lang_from_path(path: &str) ->
&'static str` in `codesight-core`, generated from (or cross-checked
against) `runtime/queries/<lang>/` + the actual grammar list. Languages
with no grammar and no file in the test suite get *removed*, not added
speculatively.

**Acceptance gate.**

- `cargo nextest run` on every existing test still passes.
- Unit test: every extension enumerated in `tags.scm` directories
  resolves; every extension not enumerated returns `"unknown"`.
- The `language_profile` function becomes
  `LanguageProfile::for_language(lang: &str)` — takes a resolved name,
  not a path.

**LoC delta.** −~80 (dedup), +~40 (shared helper + test) = net −40.

### 6b. `scope` / `file_pattern` on `inspect_graph`

**Problem.** Only `search_catalog` accepts narrowing; `inspect_graph
graph=references` on a heavily-used symbol returns 100s of edges across
the whole workspace when I only care about one crate.

**File to touch.** `crates/codesight-engine/src/service/agent_v1/graph.rs`
+ `tools.json`.

**Concrete change.** Accept `scope: Option<String>` and
`file_pattern: Option<String>` on `references`, `related`, `calls`,
`importers`, `imports`. They filter edges whose endpoint doesn't match.

**Acceptance gate.** Reuse `crates/codesight-storage/src/storage/row_utils.rs::push_scope_clause`
(exists post-sweep) — drop-in; write one test per graph mode.

**LoC delta.** +~60.

### 6c. Kill dead languages in `language_profile` (the list of 40)

Handled by 6a.

---

## Phase 7 — Cross-cutting: observability

**Problem.** Every issue above is hard to catch because there's no dashboard.

**Concrete change.**

- `manage_indexes action=session_stats` already exists and is great.
- Add a `manage_indexes action=analysis_health source_id=X` that
  returns:
  ```json
  {
    "reachability": {"roots": 12, "unreached_files": 3,
                     "potential_false_positives_from_reexports": 0},
    "graph": {"total_edges": 8912,
              "workspace_crate_edges": 143,
              "reexport_edges": 87,
              "unresolved_use_statements": 0},
    "staleness": {"in_flight": false, "last_reindex_ms_ago": 12004}
  }
  ```
- `unresolved_use_statements > 0` is a red flag — import resolver has
  blind spots.
- `potential_false_positives_from_reexports` is computed by counting
  dead_files whose only path to reachability is through a `pub use` that
  the resolver didn't follow.

**Acceptance gate.** Adds one new test; bench-mcp harness records the
metrics across commits so regressions are visible.

**LoC delta.** +~120.

---

## Acceptance matrix

| Phase | Fixes issue | Gate | Who blocks on it |
|---|---|---|---|
| 0 | — (enables all) | `tests/self_dogfood.rs` asserts invariants | Every future PR |
| 1 | #1, #2 | Phase-0 tests + existing dead_code/importer tests | Phase 2, 7 |
| 2 | trait-impl live-roots | Phase-1 gates + `impl Drop` fixture | — |
| 3a | #3 | `tests/read_content_oversized.rs` | — |
| 3b | #9 | MetaBuilder unit tests | — |
| 4a | #4 | hotspot-ranking fixture test | — |
| 4b | #8 | refactor_targets cap test | — |
| 4c | #4 (partial) | hotspot weighting test | — |
| 5a | #5 | catalog_search default test | — |
| 5b | #7 | reindex-stall integration test | — |
| 6a | #10 | lang-mapping unit test + `cargo test` green | — |
| 6b | — (enhancement) | per-graph-mode narrowing tests | — |
| 7 | — (observability) | new analysis_health test | Everything downstream |

---

## Out-of-scope / deferred

- **Hook friction (#6).** Lives in the agentpack plugin, not
  codesight-mcp. File separately against agentpack. A codesight-side
  palliative is 5a (raising default limits) which makes the blocked
  Grep path less painful.
- **Non-Rust `pub use` equivalents.** This plan's Phase-1 fix is
  Rust-centric. Python `__init__.py` re-exports (`from .foo import *`)
  have the same bug shape and should be fixed by analogy in a follow-up
  PR, keyed off the existing Python import resolver. Explicitly noted
  here so it's not forgotten.
- **Dense retrieval / embedder warmth.** Orthogonal to everything
  above. Left alone.

---

## Execution order (one sprint, four PRs)

1. **PR 1 — Phase 0.** Harness lands, Phase-1 tests are red. Proves the
   bug is real in CI.
2. **PR 2 — Phase 1 + Phase 2.** The critical fix. Phase-0 tests go
   green; existing graph tests stay green.
3. **PR 3 — Phase 3a + 3b + 4a + 4b + 5a + 5b.** Response-shape and UX
   bundle. High LoC density of small, independent changes.
4. **PR 4 — Phase 6a + 6b + 7.** Polish + observability.

Total estimated delta: **+~1100 LoC, −~250 LoC** (net +~850) for
permanently removing the worst failure mode the tool has. Tests and
fixtures dominate — production code grows maybe 400 lines.

---

## One-line summary

**It rocks at:** outline-first retrieval, cross-file ranking, transparent
token-savings, freshness warnings.
**It sucks at:** trusting its own dead-code / importer graph near crate
boundaries, default limits, and graceful handling of oversized read
responses.

The single fix that would matter most — **Phase 1 of this plan** — makes
workspace-crate `use` edges and `pub use` re-exports first-class in the
dependency graph and auto-promotes binary-target `main` to a live root.
That one change eliminates a whole class of confident-wrong outputs —
the worst failure mode a code-analysis tool can have.
