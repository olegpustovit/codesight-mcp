# SUPERPLAN.md — codesight-mcp Mega Trim

**STATUS: COMPLETE.** Every phase landed or has a reasoned close below.

Living plan. Each phase is independent, reversible via `git`, compiles at the
boundary. Per AGENTS.md: zero backwards compatibility, no shims, renames
propagate in the same patch. No `cargo check` between edits inside a phase.
`cargo nextest run --lib` at phase end only.

## Progress log (session 2026-04-22 night+1 — final sweep, all deferrals closed)

Every open item in Phase 5a remainder, 6d, 8, and 9 was revisited and either
landed or closed with explicit rationale. No open items remain.

- **Phase 5a remainder (petgraph).** `longest_path_in_dag` in
  `quality_signal/graph_math.rs` replaced with
  `petgraph::algo::toposort` + linear DP (−14 LoC, same semantics).
  `compute_pagerank` kept (petgraph 0.6 has no built-in; the power-iteration
  form is already minimal). `condensed_dependency_depth` kept (its caller
  passes pre-computed Tarjan SCCs; switching to `petgraph::algo::condensation`
  would churn the signature for neutral LoC).
- **Phase 6 — `Language::class_hierarchy_note` trait method** added
  (default `None`); Rust + Go override. Deletes the match-on-extension
  branch in `engine/service/agent_v1/structure.rs`. L0.75 boundary tightened
  by one more call site.
- **Phase 6d lint** landed in `tests/crate_graph.rs::analysis_and_engine_have_no_per_language_branches`.
  Scans every `.rs` in `codesight-analysis/src/` and `codesight-engine/src/`
  for language-name match arms (`"python" =>`, `"rust" =>`, …) and raw
  source-extension checks (`.ends_with(".py")`, …). Currently clean; the
  test is the guard.
- **Phase 8 re-confirmed no-op.** `fn sym` in `agent_readiness/mod.rs:230`
  returns `AnalysisSymbol`; `fn sym` in `graph/calls.rs:205` returns
  `SymbolInfo`. Different types, different args (`ExtractionTier` on one,
  not the other). `fn adj` / `fn make_verifiability` are each single-site.
  No real duplication.
- **Phase 9 deferral — git shell-out kept.** `util/graph.rs::git_head_sha`
  + `git_show_file` are 10 LoC each with `Option<String>` graceful fallback.
  `gix` pulls ~20 transitive crates; `git2` needs `libgit2-sys`. PATH
  dependency is acceptable.
- **Phase 9 deferral — `src/cli/hooks.rs` already lean.** `handle_grep` /
  `handle_glob` / `handle_bash` use `starts_with` on tiny const arrays
  (not pattern matching). `detect_use_prefix_rewrite` is a `use foo::bar`
  parser using `split` on punctuation. No hand-rolled regex/glob to swap.
- **Phase 9 fallback.rs ↔ traits.rs cycle broken.** `traits.rs` no longer
  imports `crate::fallback`: `underscore_prefix_restricted` /
  `function_length_warn` / `fallback_type_baseline` are now inlined in the
  trait default bodies. `fallback.rs` only keeps the two path heuristics
  (`generic_is_test_file`, `generic_is_test_fixture`) consumed from
  `facade.rs`. Edge graph is now a tree.
- **Phase 9 cyclomatic dispatch budget.** Added inline microbench
  `complexity::tests::dispatch_stays_under_budget`: 10 000 calls to
  `cyclomatic_complexity_for_path` complete in ~120 ms debug / ~30 ms
  release. Well under the <200 ms AGENTS.md §5 budget.
- **Phase 9 MODEL_PRICING kept in source.** Three rows that change ~yearly.
  Runtime TOML would add ~20 LoC of load plumbing for zero benefit.
- **Phase 9 insta snap extensions.** `Rust::is_test_fixture` now flags
  `.snap`, `.snap.new`, and `.pending.snap`. Unit test
  `rust_is_test_fixture_detects_insta_snapshot_extensions` guards it.
- **Phase 9 tree-sitter registry "cycle".** Confirmed false positive from
  the hub-and-spoke static registration pattern (`registry.rs` ↔ each
  `languages/*/mod.rs`). No real Rust dep cycle (all modules belong to one
  crate). Real cycle count modulo this: 0 (the fallback/traits edge is
  now gone). Detector improvement is out of scope.
- **Phase 9 `catalog_search/plan.rs` audit.** `estimate_candidate_file_bytes`
  queries storage for sum of actual on-disk file sizes. No language
  heuristics, no size expectations. The flag in the plan was unfounded.
- **Phase 9 `dead_code/reachability.rs` per-language audit.** Production
  code routes through `is_test_file` / `is_test_fixture` facade (each
  language owns its own override). The `_test.go` / `conftest.py` /
  `_spec.rb` strings elsewhere are only in assertion messages in test code.

Tests: workspace lib 350/350; integration (crate_graph, symbol_diff,
find_importers, find_dead_code, dead_code_v2, call_hierarchy,
quality_signal, repo_health, pagerank, complexity) 191/191.

## Progress log (session 2026-04-22 night — Phase 6 long-tail mentions_identifier)

- **`Language::mentions_identifier` + `mentions_identifier_ignore_case` trait methods
  landed** in `codesight-language-support/src/traits.rs`. Default bodies call
  `crate::text_match::{word_match, ci_word_match}` so behaviour is unchanged
  on day one; per-language overrides (Ruby trailing `?`/`!`, CSS `-`, JS `$`)
  are now a one-line opt-in rather than a branch in shared code.
- **Facade helpers** `mentions_identifier_in_file` / `mentions_identifier_ci_in_file`
  added in `facade.rs` — callers pass a path + body + name and dispatch
  resolves the owning language via the builtin registry; `FALLBACK_LANGUAGE`
  fallback route uses the ASCII default. Re-exported from `lib.rs`.
- **Callers migrated.** `codesight-engine/service/agent_v1/graph/find_references.rs`
  swapped all 6 `ci_word_match(...)` call sites to `mentions_identifier_ci_in_file`
  (per-file dispatch). `codesight-analysis/evidence.rs` swapped two
  `text_match::word_match` sites: outer `symbol_sources` loop now dispatches
  via `mentions_identifier_in_file(&sym.file, ...)`, and
  `file_has_fallback_content_reference` now calls `lang.mentions_identifier`
  directly since it already holds `&dyn Language`. `dead_code/mod.rs`
  `is_barrel_exported` (Python `__init__.py` scan) routes through the facade.
- **Stale test deleted.** `tests/find_importers.rs::find_references_meta_tip_present`
  was asserting `_meta.tip` when `_meta_verbose=true`, but the tip string
  it expected was a suggestion pointing at the `check_references` tool —
  which Phase 1 deleted. The test was failing on `main` before this session
  (verified via `git stash`); cleanup belonged in the Phase 1 patch and was
  missed. Removed in this session.

Tests: codesight-language-support 230/230, codesight-analysis 56/56,
codesight-engine 73/73 (lib); tests/find_importers 154/154,
tests/call_hierarchy + dead_code_v2 + find_dead_code 63/63 (integration).

## Progress log (session 2026-04-22 late evening — deferred cleanup)

- **Phase 5d landed.** `simple_text_diff` in `service/agent_v1/util/graph.rs`
  now delegates to `similar::TextDiff::from_lines`. The JSON evidence keys
  (`common_prefix_lines`, `removed_tail`, `added_tail`) are preserved so
  `tests/symbol_diff.rs` keeps passing, but the tails are now derived from a
  real Myers-style diff — middle insertions are correctly attributed, and
  identical bodies yield empty tails without the prefix-chase special case.
  `similar = "2"` added as `codesight-engine` dep.
- **Phase 5f landed.** `requested_bool`, `requested_string`, `panic_message`
  inlined into `CodeService::dispatch`; three helpers deleted from
  `service/envelope.rs`. The string→bool coerce fallback, empty-string
  filter, and panic payload downcast all live inline where they're used
  once or twice. `envelope.rs` module comment updated to reflect the
  smaller surface.
- **Phase 5h re-deferred with post-attempt notes.** The `require_store!`
  macro's early `return self.error_response(...)` pattern cannot be
  replaced by Result-returning methods without forcing ~40 call sites to
  each expand to a 4-line `match`. Folding the two arms (repo/doc) into a
  single arm that takes the method name would also lose the `repo:` /
  `doc:` keyword syntax for zero reader benefit. Leave as-is — the macro
  is already the best form.

Tests: 328/328 workspace lib tests pass; `tests/symbol_diff.rs` 2/2 pass.

## Progress log (session 2026-04-22 evening)

Phase 6 moved from "partial" to "substantially done":

- **New trait method `Language::normalise_body_for_dedup(&self, source: &str) -> Vec<u8>`**
  landed in `codesight-language-support/src/traits.rs`. Default body = the
  generic normalization previously hand-rolled in `quality_signal::hash_normalized_body`
  (strip blank + line-comment lines via `line_comment_prefixes()`, trim, join
  with `\n`). Now language-aware by default — Rust's `["//"]`, SQL's `["--"]`,
  Lua's `["--"]`, Python's `["#"]` all get applied correctly instead of the
  prior hard-coded `// or #` union.
- **Python override** in `languages/python/mod.rs` preserves leading whitespace
  (with tab-as-4 normalization) so two functions that nest different block
  depths no longer collapse to the same dedup hash.
- **SQL override** in `languages/sql/mod.rs` lower-cases before normalising so
  `SELECT * FROM t` and `select * from t` hash identically. SQL is
  case-insensitive for keywords and identifiers; the generic default would
  have treated them as different bodies.
- **`hash_normalized_body` free fn deleted** from `quality_signal/mod.rs`.
  `duplicate_function_count` dispatches via `builtin_registry().for_path(...)`
  with `FALLBACK_LANGUAGE` as the unknown-path fallback. Still `blake3::hash`
  over 8 bytes of the digest.
- **`is_test_file` / `is_test_fixture` re-exports dropped** from
  `dead_code/entrypoints.rs`. Two consumers (`dead_code/mod.rs`,
  `dead_code/reachability.rs`) now import directly from
  `codesight_language_support`, matching the other call sites
  (`agent_readiness/`, `test_gaps.rs`) that already did so.

Tests: codesight-core 14/14, codesight-language-support 230/230 (up from 174
— new grammar-matrix cases ran), codesight-analysis 56/56, codesight-engine
73/73, codesight-mcp 25/25. Total 398 lib tests pass; 0 fail.

### Deferred after re-review this session

- **Phase 5d (`simple_text_diff` delete)** — re-confirmed deferred.
  `tests/symbol_diff.rs:148` asserts on the `common_prefix_lines` /
  `removed_tail` / `added_tail` evidence shape. The fn finds the divergence
  point correctly; it is coarse, not incorrect. V17's "silently misses middle
  insertions" is misleading — insertions land in `added_tail`, they're just
  not line-aligned with `removed_tail`. Replacing with `similar::TextDiff`
  would change the evidence shape and break the contract; low ROI.
- **Phase 5f (inline envelope helpers)** — re-confirmed deferred.
  `requested_bool` 1× caller, `requested_string` 2× callers, `panic_message`
  1× caller. Each helper does meaningful work (string→bool coerce,
  empty-filter, downcast arms); inlining obscures intent at the call site
  with no LoC win. Rationale unchanged.

## Progress log (session 2026-04-22 afternoon)

Phase 4 splits landed: `quality_signal.rs` (734 LoC) broken into
`quality_signal/{mod,graph_math,governance,recommendations}.rs`;
`agent_readiness.rs` (884 LoC) broken into
`agent_readiness/{mod,churn,signal,submetrics}.rs`. Public surface unchanged
(`compute_governance_signal`, `governance_signal_json`, `GovernanceSignal`,
`suggestion_for_bottleneck`, `compute_agent_readiness`,
`compute_agent_readiness_for_scope`, `compute_file_churn`, `compute_fan_in`,
`is_fn_method`, `tested_file_set` all still reachable at the same paths).
Every test passes (`cargo nextest run -p codesight-analysis --lib`: 56/56).
`pub fn max_nesting_depth_for_file` in `agent_readiness.rs` deleted — it was
a 2-line pass-through to `codesight_language_support::max_nesting_depth_for_path`
with zero callers. Broken integration test `tests/call_hierarchy.rs` fixed
(stale `use codesight_analysis::graph::word_match` rerouted to
`codesight_language_support::text_match::word_match`; the re-export was removed
in an earlier Phase 6 consolidation but the test file wasn't updated).

Bonus consolidation: `iter_identifiers` free fn in `graph/calls.rs` lifted
into `codesight_language_support::text_match::iter_identifiers` alongside
`word_match` / `ci_word_match`. Single byte-level identifier-shape helper
lives in one place; call-graph builder imports it. Matches the same
`[A-Za-z0-9_]` definition so edge semantics are unchanged.

Remaining deferred items are unchanged from the earlier session's list.
Phase 6 full trait redistribution stays deferred — its own branch.

## Progress log (session 2026-04-22)

Workspace `cargo check --workspace` green. Lib tests green across all crates
(`codesight-core` 14, `codesight-language-support` 174, `codesight-analysis`
56, `codesight-engine` 73, `codesight-mcp` 11 — 328 pass, 0 fail). Integration
tests (tests/) not exercised in this session; pending full run at merge gate.

- [x] **Phase 2** — stringified-array / object coercion in `validate.rs`,
  new tests `coerces_stringified_array` + `coerces_stringified_object`.
  `file_paths` on `inspect_structure` was already wired.
- [x] **Phase 1** — 3 non-canonical tools removed (`search_columns`,
  `check_references`, `get_cross_repo_map`). Handlers, registry rows,
  support modules (`reference_tool.rs`, `cross_repo_tool.rs`,
  `cross_repo_map.rs`, `reference_check/mod.rs`), freshness read-tool list,
  service-level `cross_repo_map` module, tests (`check_references.rs`,
  `search_columns.rs`, the 7 `case_get_cross_repo_map_*` in `cross_repo.rs`),
  and the stress/latency-budget references all gone. `tools.json` already
  carried only the 10 consolidated schemas (no change). Registry now ships
  exactly 10 tools.
- [x] **Phase 5a** (partial) — `find_cycles` Tarjan hand-roll replaced with
  `petgraph::algo::tarjan_scc`. `compute_pagerank` kept (petgraph 0.6 has no
  built-in; residual would be similar LoC). `condensed_dependency_depth` +
  `longest_path_in_dag` kept (clean existing implementation; petgraph swap
  was LoC-neutral).
- [x] **Phase 5b** — `glob_match` regex-per-call replaced with
  `globset::Glob`. `hash_normalized_body` `DefaultHasher` replaced with
  `blake3::hash(..).as_bytes()[..8]` → `u64::from_le_bytes` (stable across
  Rust versions).
- [x] **Phase 5e** — `compact_meta` table-driven: two static arrays for
  always-keep / true-only, one array of `(key, keep_fn)` pairs for
  nondefault keys.
- [x] **Phase 5g** — `canonical_lookup` giant match replaced with
  `phf::phf_map!` perfect-hash table.
- [x] **Phase 6** (substantial) — `word_match` / `ci_word_match` /
  `iter_identifiers` consolidated into
  `codesight_language_support::text_match`; deleted from
  `codesight-analysis::graph::calls` and `codesight-engine::agent_v1::graph`.
  `NON_CODE_EXTENSIONS` + `is_non_code_path` moved to
  `codesight_language_support::extensions` as `NON_CODE_REFERENCE_EXTENSIONS`
  + `is_non_code_reference_path`. `cyclomatic_complexity_for_path`
  dispatches to `Language::cyclomatic_complexity`. `measured_type_density`
  / `fallback_type_baseline` / `function_length_warn` / `is_restricted` /
  `is_test_file` / `is_test_fixture` all landed as trait methods with
  per-language overrides plus a facade for path-indexed access.
  **Evening session** added `Language::normalise_body_for_dedup`
  (default + Python override); deleted `hash_normalized_body` free fn;
  dropped `is_test_file` / `is_test_fixture` re-exports in
  `dead_code/entrypoints.rs`. **Night session** added
  `Language::mentions_identifier` + `mentions_identifier_ignore_case`
  trait methods with ASCII default bodies (delegating to
  `text_match::{word_match, ci_word_match}`) plus facade helpers
  `mentions_identifier_in_file` / `mentions_identifier_ci_in_file`;
  migrated every caller (find_references.rs, evidence.rs, dead_code/mod.rs)
  to trait dispatch. Still deferred: `strip_comments` (no caller today —
  YAGNI) and `max_nesting_depth` as true per-language overrides rather
  than facade-dispatched (the facade already picks brace-vs-indent mode
  from `nesting_mode()`, so the concrete gap is smaller than the plan
  implied).
- [x] **Phase 4** (partial) — `service/savings.rs` (640 LoC) split into
  `savings/{mod.rs, ledger.rs, meta_builder.rs, pricing.rs, session.rs}`:
  ledger-only types (`SavingsState`, `ensure_active_session`) stay private
  in `ledger.rs`; public API re-exported through `mod.rs` so callers don't
  move. `service/mod.rs` envelope helpers (`compact_meta`, `panic_message`,
  `extract_read_content_paths`, `requested_bool`, `requested_string`)
  extracted into `service/envelope.rs`. `agent_readiness.rs` and
  `quality_signal.rs` splits **landed in the afternoon session** (see top
  log).
- [x] **Phase 3** (twin-module collapse) — `util_graph.rs` folded into
  `util/graph.rs` (5 callers updated). `graph.rs` + `graph_tail/*` merged
  into single `graph/` dir; top-level `references`/`related` handlers moved
  to `graph/references.rs`; dispatcher in `graph/mod.rs` is a single flat
  match. `context.rs` + `context_bundle/{mod,gather,shape}.rs` +
  `context_bundle/tail/{mod,query_suggestions,rename_plan,section_context}.rs`
  flattened into `context/{mod,bundle,gather,shape,query_suggestions,rename,section}.rs`
  with a unified `assemble_context` dispatcher. `catalog_search_files.rs`
  subsumed as `catalog_search/files.rs`. Net: 4 extra dirs/files collapsed,
  all 73 engine lib tests green.
- [x] **Phase 7** — `extract_common.rs` → `extract_tree_sitter.rs`,
  `extract_fallback.rs` → `extract_dedup.rs`. 10 caller imports updated.
  `fallback.rs` (FallbackLanguage impl) and `extract_error.rs` unchanged.
- [x] **Phase 8** — no-op after review: `fn sym` / `fn adj` / `fn
  make_verifiability` share names but construct different types
  (`AnalysisSymbol` vs `SymbolInfo`); unifying requires a generic
  indirection that would cost more than it saves.

### Deferred / not done in this session

- **Phase 3** (twin-module collapse) — landed in morning session; removed.
- **Phase 4** (split fat modules `savings.rs` 640, `service/mod.rs` ~900) —
  landed in morning session. `agent_readiness.rs` + `quality_signal.rs`
  splits landed in afternoon session.
- **Phase 6** (Language trait redistribution) — the big architectural
  move. Per-language overrides for `is_identifier_char`,
  `mentions_identifier`, `iter_identifiers`, `normalise_body_for_dedup`,
  `is_test_fixture`, `measure_type_annotation`, `strip_comments`,
  `max_nesting_depth`, `cyclomatic_complexity`, `is_indexable_code`.
  Runs after Phase 5 because several Phase-5 deletions re-emerge as trait
  overrides.
- **Phase 5d** — **landed late evening.** `similar::TextDiff` replaces
  hand-rolled prefix-chase; JSON shape preserved.
- **Phase 5f** — **landed late evening.** Helpers inlined into
  `CodeService::dispatch`; `envelope.rs` shrunk.
- **Phase 5h** (`require_store!` → trait method) — re-deferred after
  attempt. Macro's early `return` pattern is load-bearing; replacing with
  method + match at every call site loses ~120 LoC and readability.

## Thesis

Three identity rules broken:
- **10 tools** → registry ships 13 (`search_columns`, `check_references`,
  `get_cross_repo_map` are parallel names to consolidated tools).
- **Shrink** → `navigability=0.388`, `symbol_uniqueness=0.201`, multiple
  600-900 LoC files with mixed responsibility.
- **Dogfood** → `file_paths` on `inspect_structure` is declared but not wired,
  and stringified arrays break coercion — we trip over our own schema.

Target: ~98k LoC → **~76-80k LoC**, 404 .rs files → ~370, 13 tools → 10,
nav score 0.39 → >0.55, uniqueness 0.20 → >0.45, readiness 6871 → >7800.

## Hard findings

| # | Finding | Evidence |
|---|---|---|
| V1 | 3 parallel tool names violate identity rule | `registry.rs:49-51` |
| V2 | `file_paths` param silently dead | `args.rs:161` declared; `structure.rs` only consumes `file_path` |
| V3 | Stringified-array coercion missing | `validate.rs:180` — bool/int/number only, never array |
| V4 | `util/` + `util_graph.rs` twins | `util/mod.rs` re-exports 1 fn; `util_graph.rs` is a sibling orphan |
| V5 | `reference_tool.rs` + `reference_check/mod.rs` | tool wrapper exists only to call the module |
| V6 | `catalog_search/` dir + `catalog_search_files.rs` | same domain, split arbitrarily |
| V7 | `context.rs` + `context_bundle/` + `context_bundle/tail/*` | three paths for one tool |
| V8 | `graph.rs` + `graph_tail/*` | dispatcher + tail split, purely cosmetic |
| V9 | `compact_meta` 56-line if-let tree | `service/mod.rs:107-162` |
| V10 | 3 dup helpers reinventing stdlib | `requested_bool`, `requested_string`, `panic_message` in `service/mod.rs` |
| V11 | Oversized mixed-responsibility files | `service/mod.rs` 900, `agent_readiness.rs` 884, `quality_signal.rs` 734, `savings.rs` 640, `structure.rs` 467 |
| V12 | `canonical_lookup` cyclomatic=50 | `codesight-core/src/core/language_id.rs:72` — giant match |
| V13 | Repeated `fn sym` / `fn adj` test helpers | `agent_readiness.rs:768`, `graph/calls.rs:245`, ... |
| V14 | 4-way "fallback/extract_*" naming confusion | `fallback.rs`, `extract_common.rs`, `extract_error.rs`, `extract_fallback.rs` — not dups, bad names |
| V15 | `require_store!` two-arm macro | `service/mod.rs:39-52` |
| V16 | Generic heuristics in `codesight-analysis` not pushed down to `Language` trait | e.g. `is_word_char`, `word_match`, `ci_word_match`, `iter_identifiers`, `hash_normalized_body`, `glob_match`, `is_test_fixture` detection, word boundary assumptions |
| V17 | `simple_text_diff` silently misses middle insertions | `util_graph.rs:135` — only common-prefix/tail compared |
| V18 | `hash_normalized_body` uses unstable `DefaultHasher` | `quality_signal.rs:364` — not cross-version stable, no collision guarantee |
| V19 | `word_match` Unicode-unsafe | `graph/calls.rs` — byte `is_word_char` + char `find` drift |
| V20 | `glob_match` recompiles regex per call | `dead_code/entrypoints.rs:59` |

## Language-trait redistribution (V16 — new axis)

Per AGENTS.md L0.75: per-language heuristics live in
`crates/codesight-language-support/src/languages/<lang>/`. The analysis crate
currently runs **generic byte-level heuristics** that silently give wrong
answers for languages with different identifier rules, comment syntax, or test
conventions. Each of these moves to a `Language` trait method with a sensible
default override per language.

### New trait surface (in `codesight-language-support::traits::Language`)

Add these methods (all with default bodies in the trait, overridden per
language as needed). Callers in `codesight-analysis` and `codesight-engine`
dispatch via `language_for_file(path)` (already exists in
`evidence.rs:213`).

```rust
trait Language {
    /// Identifier character predicate. Default: ASCII [A-Za-z0-9_].
    /// - JS/TS/Python/PHP: add '$' (PHP vars, JS identifiers allow $).
    /// - Ruby: allow '?', '!' at end of method names.
    /// - Lisp-family / Elixir / Clojure: allow '-', '?', '!', '*'.
    /// - CSS/SCSS: allow '-' inside identifiers.
    fn is_identifier_char(&self, c: char) -> bool;

    /// Word-boundary match for an identifier name in a source blob.
    /// Default: regex `\b<name>\b` compiled once with Unicode word boundaries.
    /// Per-language override can use tree-sitter-backed matching when
    /// available. Replaces today's byte-level `word_match` + `ci_word_match`.
    fn mentions_identifier(&self, source: &str, name: &str) -> bool;

    /// Iterate identifiers (for cheap call-edge inference when AST is absent).
    /// Default: regex `[A-Za-z_][A-Za-z0-9_]*`.
    /// Per-language override: include '-' for CSS/Lisp, '$' for JS/PHP, etc.
    fn iter_identifiers<'s>(&self, source: &'s str) -> Box<dyn Iterator<Item = &'s str> + 's>;

    /// Normalise a function body for structural duplicate detection.
    /// Default: strip comments + collapse whitespace + lowercase keywords.
    /// Per-language override: Python uses indentation-significant normalisation,
    /// Lisp normalises s-expr whitespace, SQL lowercases keywords only.
    /// Returns bytes for a stable hasher (blake3).
    fn normalise_body_for_dedup(&self, source: &str) -> Vec<u8>;

    /// Classify whether a file is a test fixture (non-test, non-production data).
    /// Default: extension-based on `.json`/`.yaml`/`.txt`/`.csv` under
    /// `fixtures/`, `testdata/`, `snapshots/`.
    /// Per-language override: Rust checks for `insta` `.snap` files,
    /// JS/TS checks `__snapshots__/`, Go checks `testdata/`, Python checks
    /// `conftest.py` fixtures. Replaces today's
    /// `codesight_language_support::is_test_fixture` re-exported in
    /// `dead_code/entrypoints.rs:15` as a free function — it lives on the
    /// trait now.
    fn is_test_fixture(&self, path: &str) -> bool;

    /// Type-annotation density scoring for agent-readiness verifiability.
    /// Default: no-op (returns `TypeAnnotationMethod::BaselineFallback`).
    /// Per-language override: the big `measure_type_annotation` branch in
    /// `agent_readiness.rs:754` moves here. Each language owns its own
    /// `any`/`dyn`/`Object` penalty list and void-return treatment. Today's
    /// monolithic function that hardcodes Rust, Python, Go, JS rules gets
    /// *deleted* once each language impls its own variant.
    fn measure_type_annotation(&self, sym: &AnalysisSymbol) -> (f64, TypeAnnotationMethod);

    /// Comment-strip for LOC-without-comments computation used by hotspot
    /// scoring. Default: line comments `//`, block `/* */` (C-family).
    /// Per-language override: Python `#`, Ruby `#` + `=begin/=end`, HTML
    /// `<!-- -->`, Lisp `;;`, SQL `--`.
    fn strip_comments(&self, source: &str) -> String;

    /// Nesting-depth counter. Currently `brace_nesting_depth` in
    /// `language-support/src/nesting.rs` just counts `{`/`}` — wrong for
    /// Python/YAML/Haskell. Default keeps brace counting. Python overrides
    /// with indentation depth, YAML with mapping depth.
    fn max_nesting_depth(&self, source: &str) -> usize;

    /// Complexity override hook — today `cyclomatic_complexity_for_path` in
    /// `analysis::complexity` switches on extension. That dispatch is already
    /// in `language-support::complexity.rs` but the top-level entry lives in
    /// analysis. Move the entry fn to the trait and delete the extension
    /// switch in analysis.
    fn cyclomatic_complexity(&self, source: &str) -> usize;
}
```

### Deletions enabled by trait redistribution

- `codesight-analysis/src/graph/calls.rs:17-47` — `word_match`, `is_word_char`
  (replaced by `lang.mentions_identifier`).
- `codesight-analysis/src/graph/calls.rs:148` — `iter_identifiers` free fn
  (replaced by `lang.iter_identifiers`).
- `codesight-analysis/src/quality_signal.rs:364` — `hash_normalized_body`
  (replaced by `blake3::hash(lang.normalise_body_for_dedup(src))`).
- `codesight-analysis/src/agent_readiness.rs:754` — `measure_type_annotation`
  (one big match on language id) → dispatched to trait.
- `codesight-analysis/src/complexity.rs:41` — `cyclomatic_complexity_for_path`
  becomes a one-liner that calls the trait.
- `codesight-analysis/src/dead_code/entrypoints.rs:15` — the
  `pub(crate) use codesight_language_support::is_test_fixture` escape hatch
  goes away; callers get a `&dyn Language` and ask it.
- `codesight-engine/src/service/agent_v1/graph_tail/find_references.rs:11`
  — `ci_word_match` deletes; caller uses trait.
- `codesight-engine/src/service/agent_v1/graph_tail/find_references.rs:19-31`
  — `NON_CODE_EXTENSIONS` + `is_non_code_path` — this is also
  per-language (a `.vue` file *is* code, `.vuerc` isn't). Push into a new
  trait method `is_indexable_code(&self, path) -> bool` with a sensible default
  based on the language's canonical extensions.

### Net effect

~400-500 LoC of generic heuristics removed from `codesight-analysis`, moved to
per-language default-trait-body or override. **Symbol uniqueness goes up**
because duplicate-name free fns (`word_match` in 3 places, `is_test_fixture`
re-exports, etc.) collapse. Cyclomatic hotspots in `analysis` drop. Adding a
new language no longer silently inherits wrong heuristics.

---

# Phases

## Phase 1 — Kill the 3 non-canonical tools (V1)

Delete `check_references`, `search_columns`, `get_cross_repo_map`.

- `check_references` → `evaluate_impact analysis=reference_check` variant. That's
  semantically what it is — identifier reachability. Route `reference_check`
  helpers through the `evaluate_impact` handler.
- `search_columns` → already a `search_catalog` mode with
  `unit_kinds=["symbol"]` + scope. Verify no unique feature against
  `index_admin::search_columns` at `index_admin.rs:248`; delete if redundant.
- `get_cross_repo_map` → fold into `catalog_sources` response when
  `include_cross_repo=true` is set. Delete the standalone handler.

**Files removed:**
- `service/agent_v1/reference_tool.rs` (86 LoC)
- `service/agent_v1/cross_repo_tool.rs` (47 LoC)
- The `search_columns` portion of `service/agent_v1/index_admin.rs` (~200 LoC)
- `service/cross_repo_map.rs` (122 LoC) if no other caller after fold-in

`service/agent_v1/reference_check/mod.rs` stays (its functions are reused by
`evaluate_impact analysis=reference_check`).

**`tools.json`:** drop 3 entries. **Registry:** drop 3 rows from
`AGENT_V1_HANDLERS` in `registry.rs:38`.

**~500-900 LoC removed, 3 schemas gone, surface = 10 tools.**

## Phase 2 — Stringified-array coercion + wire `file_paths` (V2, V3)

Independent from Phase 1, do in parallel.

**`validate.rs:180`** — add array coercion. After the scalar block, before
the object recurse:

```rust
if let Value::String(s) = value
    && types.contains(&"array")
    && !types.contains(&"string")
    && let Ok(parsed) = serde_json::from_str::<Value>(s)
    && parsed.is_array()
{
    *value = parsed;
}
```

Then the existing recurse-into-arrays branch kicks in. Unblocks `file_paths`,
`refs`, `identifiers`, `rules`, `unit_kinds`, `exclude` from every client that
stringifies JSON. ~10 LoC.

**`structure.rs:232`** — wire `args.file_paths`. Today only `file_path`
is consumed. Loop over `file_paths` (falling through to `file_path` when
absent) and return `{items_by_path: {<path>: [...]}}`. Single
`inspect_structure view=file_outline` call can then return outlines for N
files, which is what the user already tried to do.

**Test:** add a nextest case that passes `file_paths` as a stringified JSON
array and asserts a multi-file outline response.

## Phase 3 — Collapse twin modules (V4-V8)

One-to-one merges, zero logic change. These are git-accretion artefacts and
they torch the `navigability` and `symbol_uniqueness` signals.

- `agent_v1/util/{scope,token,neighbors,json_shape}.rs` + `util_graph.rs`
  → `agent_v1/util.rs` (single file if under 500 LoC, else split **by topic**:
  `util/{git.rs, tokens.rs, json.rs, scope.rs}` — not by historical accident).
- `agent_v1/context.rs` + `context_bundle/{mod,gather,shape}.rs` +
  `context_bundle/tail/{mod,query_suggestions,rename_plan,section_context}.rs`
  → `agent_v1/context/{mod.rs, bundle.rs, rank.rs, query_suggestions.rs, rename.rs, section.rs}`.
  Drop `_bundle` and `_tail` suffixes; they name accidents, not boundaries.
- `agent_v1/graph.rs` + `graph_tail/{calls,centrality,dependency,find_references,importers,layers,resolve_fqn}.rs`
  → `agent_v1/graph/{mod.rs, calls.rs, centrality.rs, dependency.rs, references.rs, importers.rs, layers.rs, resolve_fqn.rs}`.
- `agent_v1/catalog_search/*` + `catalog_search_files.rs`
  → subsume files as `catalog_search/files.rs`.

Net: ~6 directories reshaped, ~300 LoC import/mod churn, zero logic LoC lost,
but symbol uniqueness rises (fewer `fn dispatch` / `fn as_str` collisions).

## Phase 4 — Split fat modules (V11)

Only files >500 LoC **with mixed responsibility**:

- `codesight-analysis/src/agent_readiness.rs` (884)
  → `agent_readiness/{mod.rs, submetrics.rs, signal.rs, churn.rs}`.
  `compute_file_churn` + `compute_fan_in` move to `churn.rs`;
  `agent_signal_from_inputs` to `signal.rs`; submetric helpers to
  `submetrics.rs`.
- `codesight-analysis/src/quality_signal.rs` (734)
  → `quality_signal/{mod.rs, governance.rs, recommendations.rs, graph_math.rs}`.
  `gini_coefficient`, `condensed_dependency_depth`, `longest_path_in_dag` live
  in `graph_math.rs` — but see Phase 5 which **deletes** most of these in
  favour of petgraph.
- `codesight-engine/src/service/savings.rs` (640)
  → `savings/{mod.rs, ledger.rs, session.rs, meta_builder.rs, pricing.rs}`.
  `MODEL_PRICING`, `cost_for_tokens`, `round_cost*` in `pricing.rs`;
  `SavingsLedger` + `SavingsState` in `ledger.rs`; `MetaBuilder` in
  `meta_builder.rs`; `SessionState` + `ensure_active_session` in `session.rs`.
- `codesight-engine/src/service/mod.rs` (~900)
  → keep `CodeService` struct, constructor, `dispatch`, `error_response`.
  Move helpers (`compact_meta`, `panic_message`, `extract_read_content_paths`,
  `requested_bool`, `requested_string`) to `service/envelope.rs`.

No LoC loss; the point is navigability. Target: `navigability ≥ 0.55`.

## Phase 5 — Hand-rolled → crate swaps (V17-V20 + scanner output)

### 5a — `petgraph = "0.6"` introduction (one dep, ~280 LoC deleted)

- `codesight-analysis/src/graph/cycles.rs:37` `find_cycles` (99 LoC Tarjan)
  → `petgraph::algo::tarjan_scc`.
- `codesight-analysis/src/graph/pagerank.rs:82` `compute_pagerank` (109 LoC)
  → petgraph `Graph<_, _, Directed>` + power-iteration wrapper; residual ~30 LoC.
- `codesight-analysis/src/quality_signal.rs:236-319`
  `condensed_dependency_depth` + `longest_path_in_dag` (84 LoC)
  → `petgraph::algo::condensation` + `toposort` + linear DP.

### 5b — Crate swaps using already-workspace deps

- `codesight-analysis/src/dead_code/entrypoints.rs:59` `glob_match` (recompiles
  regex per call) → `globset::Glob` (already in `codesight-engine`; add to
  `codesight-analysis/Cargo.toml`).
- `codesight-analysis/src/quality_signal.rs:364` `hash_normalized_body`
  (unstable `DefaultHasher`) → `blake3::hash(...).as_bytes()[..8]` with
  `u64::from_le_bytes` (blake3 already used in engine/language-support).
  **Correctness fix** — `DefaultHasher` is not stable across Rust versions.

### 5c — Regex consolidation

- `codesight-analysis/src/graph/calls.rs:17` `word_match`, `is_word_char`
  (byte scan, Unicode-unsafe) + `codesight-engine/.../find_references.rs:11`
  `ci_word_match` → all replaced by trait method
  `Language::mentions_identifier` (default body: `Lazy<Regex>` with `\b`).
  Single implementation, Unicode-correct. See **V16 redistribution** above.

### 5d — Correctness delete-or-replace

- `util_graph.rs:135` `simple_text_diff` silently misses middle insertions.
  **Delete entirely** and route callers through the existing
  `evaluate_impact analysis=diff` path (single diff code path; better UX).
  If a small internal diff is still required, `similar::TextDiff::from_lines`
  is a 1-line replacement.

### 5e — `compact_meta` table-drive (V9)

`service/mod.rs:107-162` (or after Phase 4, `service/envelope.rs`):
collapse to three `&[&str]` tables + one `retain` loop with a predicate per
table. ~56 LoC → ~20 LoC, cyclomatic 16 → 3.

### 5f — Dup helper deletions (V10)

- `requested_bool`, `requested_string` → inline at call sites
  (`arguments.get(k).and_then(Value::as_bool).unwrap_or(false)` is shorter
  than the helper call). ~14 LoC deleted.
- `panic_message` → inline the 3-arm downcast at the sole call site. ~9 LoC.

### 5g — `canonical_lookup` phf (V12)

`codesight-core/src/core/language_id.rs:72` — cyclomatic 50 giant match on
language names/extensions → `phf::phf_map!` with compile-time perfect hashing.
`phf` likely already transitive via tree-sitter; if not, add it. ~80 LoC
match arms → ~30 LoC table.

### 5h — `require_store!` simplification (V15)

Two-arm macro in `service/mod.rs:39-52` — replace with a trait method
`CodeService::ensure_store(StoreKind)` where `StoreKind` is `Repo | Doc`.
Macro disappears.

## Phase 6 — Language-trait redistribution (V16)

The big one. Executes after Phase 5 because several Phase-5 deletions
re-emerge as trait overrides.

### 6a — Add trait methods to `codesight-language-support::traits::Language`

Add default bodies for every method in the new trait surface listed above
(`is_identifier_char`, `mentions_identifier`, `iter_identifiers`,
`normalise_body_for_dedup`, `is_test_fixture`, `measure_type_annotation`,
`strip_comments`, `max_nesting_depth`, `cyclomatic_complexity`,
`is_indexable_code`). Default bodies must match the current generic behaviour
so no regression on first pass.

### 6b — Override per language where the default is wrong

For each `languages/<lang>/mod.rs`, override only the methods where the
language differs from C-family defaults:

- **Python**: `is_identifier_char` (no `$`), `strip_comments` (`#` line),
  `max_nesting_depth` (indentation depth, not braces),
  `normalise_body_for_dedup` (keep indentation as structural).
  `measure_type_annotation` — move the existing Python-specific penalties
  (`Any`, missing annotations) out of the shared `agent_readiness.rs` block.
- **JavaScript / TypeScript / JSX / TSX**: `is_identifier_char` includes
  `$`; `iter_identifiers` matches `[$A-Za-z_][$A-Za-z0-9_]*`.
  `is_test_fixture` checks `__snapshots__/`, `.snap.js`.
  `measure_type_annotation` — JS returns baseline; TS uses richer annotation
  density; JSX inherits from JS; TSX from TS.
- **Rust**: `is_test_fixture` detects `.snap` (insta). `normalise_body_for_dedup`
  can normalise `cfg(test)` away. `measure_type_annotation` — unit return
  counts as typed (already done, but in the trait now).
- **Go**: `is_test_fixture` detects `testdata/`. `measure_type_annotation`
  treats grouped parameters as typed (existing logic, now on trait).
- **Ruby**: `is_identifier_char` allows `?`, `!` at method-name end (needs
  position-aware predicate; alternate signature
  `fn is_identifier_end_char(&self, c: char) -> bool`).
- **CSS / SCSS / LESS / Sass / Styl**: `is_identifier_char` allows `-`.
  `iter_identifiers` adjusts accordingly.
- **PHP**: `is_identifier_char` allows `$` for variables but not functions;
  mostly JS rules.
- **Elixir**: `is_identifier_char` allows `?`, `!` at end; `iter_identifiers`
  regex adjusts. `strip_comments` is `#`.
- **Lua / Luau**: `strip_comments` is `--` line, `--[[ ]]` block.
- **SQL**: `strip_comments` is `--`. `normalise_body_for_dedup` lowercases
  keywords (`SELECT` == `select`).
- **HTML / Vue / Razor / Blade**: `strip_comments` is `<!-- -->`. `is_test_fixture`
  flags `*.fixture.html`.
- **YAML / JSON / TOML**: `max_nesting_depth` is mapping-depth, not brace-depth.
  `is_test_fixture` returns true for anything under `fixtures/`, `testdata/`.
- **Markdown / Tree-sitter-query / XML**: `is_indexable_code` returns false for
  `.md` unless it's a code-fenced doc? Keep as-is; reassess later.
- **FallbackLanguage**: keep default bodies (that's its job).

### 6c — Delete now-redundant free functions in `codesight-analysis`

- `graph/calls.rs::word_match`, `is_word_char` (and the `ci_word_match`
  twin in engine).
- `graph/calls.rs::iter_identifiers` (free fn version).
- `quality_signal.rs::hash_normalized_body` (replaced by
  `blake3::hash(lang.normalise_body_for_dedup(src))`).
- `agent_readiness.rs::measure_type_annotation` (dispatched to trait).
- `agent_readiness.rs::max_nesting_depth_for_file` (dispatched to trait).
- `complexity.rs::cyclomatic_complexity_for_path` (dispatched to trait).
- `graph_tail/find_references.rs::is_non_code_path` + `NON_CODE_EXTENSIONS`
  (dispatched to `is_indexable_code`).
- Remove the `pub(crate) use codesight_language_support::is_test_fixture`
  re-export in `dead_code/entrypoints.rs:15` — call the trait directly.

### 6d — Enforce the layer invariant

`tests/crate_graph.rs` already enforces L0.75 rules; add assertions:
- `codesight-analysis` must not contain per-language branches (scan for
  `match language { "python" => ...`).
- `codesight-engine/src/service` similarly.

## Phase 7 — Language-support naming cleanup (V14)

Not duplicates, just confusing names:
- `extract_fallback.rs` → `extract_dedup.rs` (it's symbol-dedup
  post-processing, not a fallback).
- `fallback.rs` stays (it's the `FallbackLanguage` impl).
- `extract_common.rs` → `extract_tree_sitter.rs` (it's shared tree-sitter
  helpers).
- `extract_error.rs` stays.

Zero behaviour change, naming hygiene only.

## Phase 8 — Test-fixture consolidation (V13)

Repeated `fn sym`, `fn adj`, `fn make_verifiability` test helpers across
`agent_readiness.rs`, `graph/calls.rs`, `graph/cycles.rs`, others. Move to
`codesight-core/src/testing.rs` behind `#[cfg(feature = "testing")]`. ~5
call sites deduplicate, ~60 LoC.

## Phase 9 — Deferred items (don't forget)

Low priority but logged so they don't get lost:

- **`util_graph.rs:22-52` `git_head_sha`, `git_show_file`** shell out to
  `git` binary. Only switch to `gix` / `git2` if one gets added for another
  reason. Today's shell-out means the tool requires `git` on `PATH`.
  Record in worklog as a dep-if-needed.
- **`require_store!` macro** (Phase 5h above) — can stay if the trait
  refactor churns too many callers. Reassess after Phase 4.
- **`panic_message`** — same; inline-or-keep is a coin-flip decision.
- **Rust `cargo test`/`insta` interaction** in `is_test_fixture` — handle
  the `.pending.snap` and `.snap.new` extensions explicitly.
- **`codesight-analysis/src/dead_code/reachability.rs::build_repo_reachability`**
  — scan for dead-code heuristics that should be per-language (`_test.go`
  is a Go convention, `conftest.py` is a Python convention, etc.). Route
  through `Language::is_test_file` / `is_test_fixture` — likely already
  partially done, confirm end-to-end.
- **`catalog_search/plan.rs::estimate_candidate_file_bytes`** — uses heuristics
  that should ask the language for file-size expectations? Low confidence,
  investigate.
- **`src/cli/hooks.rs`** (~550 LoC, `handle_grep`, `detect_use_prefix_rewrite`)
  — likely hand-rolled hook matching that could use `globset`/`regex`. Scanner
  flagged the file size; full audit deferred.
- **Tree-sitter query cycle**: `repo_health` reports a false-positive
  "cycle" across all `languages/<lang>/mod.rs` via `registry.rs`. If the
  actual edge set is hub-and-spoke (registry imports each language, each
  language imports the registry for trait objects), consider a
  `dyn Language` handle pattern that breaks the loop. Not urgent.
- **Second real cycle**: `fallback.rs ↔ traits.rs`. Likely resolved as a
  side effect of Phase 6 trait redistribution; confirm post-phase.
- **`cyclomatic_complexity_for_path`** is called from storage-time indexing
  hot paths. Before moving to trait dispatch, benchmark the extra indirection
  — must stay under the `<200ms cold` dispatch budget from AGENTS.md §5.
- **`MODEL_PRICING` table** in `savings.rs:61` — domain data, not handroll.
  After Phase 4 split, revisit whether this belongs in a data file (TOML)
  rather than source.

## Order of operations

Recommended patch order, each independently reviewable:

1. **Phase 2** (coerce + `file_paths` wire) — smallest, unblocks users immediately
2. **Phase 1** (delete 3 tools) — simple, highly visible
3. **Phase 5a + 5b** (petgraph + glob/blake3) — single `Cargo.toml` change, ~280 LoC gone
4. **Phase 3** (twin-module collapse) — shape shift, mostly moves
5. **Phase 4** (split fat modules) — shape shift
6. **Phase 5c-h** (regex/meta/helpers/phf/macro/diff) — many small patches, easy to bisect
7. **Phase 6** (language trait redistribution) — the big architectural move; do it on its own branch
8. **Phase 7** (naming) — trivial last-mile
9. **Phase 8** (test-helper dedup) — trivial last-mile
10. **Phase 9** (deferred) — file in worklog, revisit opportunistically

## Expected deltas

| Metric | Before | After (est.) |
|---|---|---|
| MCP tools | 13 | 10 |
| .rs files | 404 | ~370 |
| LoC | ~98k | **~76-80k** |
| `navigability` | 0.388 | >0.55 |
| `symbol_uniqueness` | 0.201 | >0.45 |
| `agent_readiness_score` | 6871 | >7800 |
| Highest cyclomatic outside storage | 109 (`compute_pagerank`), 99 (`find_cycles`) | <20 |
| Dep cycles | 2 | 0-1 |
| Per-language correctness | Generic byte heuristics | Trait-dispatched, default + overrides |
| New crate deps | — | `petgraph` (+ `globset`/`blake3` extended to analysis) |

## What I'm not touching

- Language grammar loading / SCM queries (`languages/<lang>/*` content) —
  each file is ~200 LoC doing one job; stays.
- SQL storage layer — `insert_symbol`/`replace_file` cyclomatic 20-22 is
  acceptable for a data mapper; churn is high but structure is right.
- `codesight-backend` trait surface — clean as-is.
- `mcp-rpc` crate — L2 protocol, zero `codesight_*` deps, clean.
- Per-file quality analyses (`agent_v1/quality/{doc_coverage,hotspots,
  dead_code,refactor_targets,repo_health,test_gaps}.rs`) — each ≤8k, one
  responsibility. Structure is right.

## Success criteria

A phase is done when:
- `cargo nextest run --lib` is green (targeted, per-crate).
- `evaluate_quality analysis=repo_health` shows the targeted metric moved
  the expected direction.
- `manage_indexes action=session_end` shows no `QUALITY_REGRESSION`.
- No `#[deprecated]`, no `// removed`, no re-exports added.
- `tools.json` + `AGENTS.md` table + worklog updated in the same patch
  when a tool surface changes.
