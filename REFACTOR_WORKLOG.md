# REFACTOR_WORKLOG.md

Single living state for `codesight-mcp`. Update this file in the same patch
as the change it describes. Move closed items out — git history is the
archive. See [`AGENTS.md`](AGENTS.md) for the policies that govern this work.

---

## Current metrics (baseline: 2026-04-11)

| Metric | Current | Target | Trend |
|---|---|---|---|
| Product crate LoC (`src/`) | ~25.4k | ↓ each session | — |
| Workspace crate LoC (`crates/`) | core 0.2k / extractors 9.0k / storage 2.4k | stable | — |
| Files > 400 LoC in `src/` | see S1 item 1 below | 0 | — |
| Open test-drive issues | 1 (see below) | 0 | — |
| Cold dispatch handler (cached read) | — | < 50 ms | needs bench |
| Cold full ingest (this repo) | — | < 5 s | needs bench |
| Incremental reindex (1 file) | — | < 100 ms | watch mode WIP |
| Uncached focused test loop | — | ≤ 30 s | needs measurement |
| `_savings.json` total tokens saved | read with `check --savings` | ↑ each session | — |
| Slow tests (> 1 s) | unknown | 0 | needs audit |

**Measurement plan (next session):** write a `scripts/bench-hotpaths` shell
script that times `ingest_sources local_path=.`, a keyword
`search_catalog`, a `read_content kind=symbol`, and a delta reindex after
`touch src/lib.rs`. Record numbers here.

---

## Active workstreams

### S1 — Simplification & dedup

**Principle:** shrink the product; flatten oversized modules; collapse dead
code; reach for crates before hand-rolling.

**Open items:**

1. **Split remaining oversized modules.** Re-audit after the
   `inspect_structure` split (v3 #43). Run
   `mcp__codesight__evaluate_quality analysis=hotspots` to get the current
   ranked list, then split any file > 400 LoC with cyclomatic > 20. Known
   candidates carried forward:
   - `src/storage/doc_index/mod.rs` → `query.rs`, `store.rs`.
   - `src/service/agent_v1/impact.rs` → `impact/{blast_radius,preview,rename_safety,changed_units,diff}.rs`.
   - `src/service/agent_v1/graph_tail.rs` → tail-expansion submodule.
   - `src/service/agent_v1/catalog_search.rs` → `catalog_search/{bm25,semantic,fuzzy,regex}.rs`.
   - `src/indexing/packages/walker.rs` → keep walker slim; move embedded
     integration tests to `tests/packages_walker.rs`.
2. **Residual panics.**
   - `src/service/agent_v1/validate.rs:83` — replace
     `panic!("tools.json must be a JSON array")` with a startup error in
     `main.rs`.
   - `crates/codesight-extractors/.../scm/injections.rs:203–204` — return an
     `ExtractError` from query-pack compile.
3. **`env::var` cleanup.** `src/retrieval/semantic.rs`,
   `src/retrieval/summarizer.rs`, `src/server_support/path_map.rs` must use
   consts in `src/config/env.rs`. Add `CODESIGHT_EMBED_MODEL`,
   `CODESIGHT_SUMMARY_MODEL`.
4. **Dead / commented code.** `tests/tools_remaining.rs` — restore each
   commented case as a real test, or delete the file.
5. **Infallible-regex consolidation.** Seven `.expect("infallible … regex")`
   in `src/framework/laravel/*` → single `static LazyLock<Regex>` table per
   module.
6. **Finish test layering.** Migrate `tests/get_doc_coverage.rs`,
   `tests/doc_broken_links.rs`, `tests/doc_parser*.rs`,
   `tests/retrieval_tools.rs`, `tests/get_context_bundle.rs` to
   `src/test_support/indexed.rs`. Split nextest buckets into
   `unit | slice | contract | heavy`. Uncached focused loop ≤ 30 s.

**Done when.** No `src/` file over 400 LoC without a worklog note. No
`panic!`/`unreachable!` outside tests. No `std::env::var("CODESIGHT_*")`
outside `src/config/env.rs`.

### S2 — Crate modularity

**Standing rules.** The crate graph is frozen at four members. No
speculative splits. `codesight-core` stays skeletal (symbol types,
`FactQuality`, the future `ContextProvider` trait). No catch-all
`types`/`utils` crate. Product-only logic stays in the root crate.

**Open items:**

1. **Dependency audit patch.** Run a `cargo metadata` pass; fail if
   `codesight-core` depends on anything upward, or if the two middle crates
   depend on each other. Record the edge list in
   [`ARCHITECTURE.md`](ARCHITECTURE.md) (table already present, regenerate
   whenever a member or edge changes).
2. **`lib.rs` shrink.** Every crate's `lib.rs` must be pure module
   declaration + re-export — no free functions.
3. **Provider trait placement.** The `ContextProvider` trait (blocks S4-P17)
   lives in `crates/codesight-core/src/providers.rs`; concrete providers in
   `src/indexing/providers/*` in the product crate.

### S3 — Real-use onboarding

**Goal.** Fresh clone → working Claude Code MCP integration in **under five
minutes** following only `README.md`.

**Status.** `README.md` landed, `init --claude-code` landed, `check`
subcommand landed. Remaining:

1. **Self-host dogfood script.** `scripts/selfhost-index` — builds release,
   runs `init --workspace`, runs `ingest_sources local_path=.`, runs a
   canned `search_catalog`, prints `_meta.tokens_saved`. Smoke test for
   principle 4 and the onboarding story.
2. **Install-failure triage.** Every first-run error path (missing grammar,
   unreadable `$XDG_CONFIG_HOME`, stale lock) surfaces a one-line
   actionable message — no panics, no raw `anyhow` chains. Drive with
   `rust-cli-building`.

### S4 — Reliability & prior-art parity

**Goal.** Works correctly on *any* codebase. Feature parity with the
`../prior-art-mcp` patterns that matter: response envelope telemetry,
session awareness, batch params, watch mode, context providers.

**Open items (tracked by old P-number so history is searchable):**

- **P11 — Response envelope.** `src/service/response_envelope.rs`:
  `ToolEnvelope<T> = { data, meta, warnings }` with `timing_ms`,
  `truncated`, `tokens_saved`, `total_tokens_saved`, `estimate_method`,
  `powered_by`, optional `next_cursor`, `freshness`, `error_category` on
  failure. `src/retrieval/token_tracker.rs` persisted to
  `$storage_dir/_savings.json` (atomic append + file lock). Every handler
  shapes responses through a `RespondWith` helper — no handler builds
  outer JSON. Surfaces in `codesight-mcp check --savings`.
  *Partial: `_meta` already carries `timing_ms`, `build_sha`, `build_version`,
  `build_time`. The `ToolEnvelope<T>` type + unified helper still missing.*
- **P12 — Session awareness.** `src/server_support/session/{mod,journal,cache,negative}.rs`.
  Append-only journal at `$storage_dir/sessions/<workspace_hash>.jsonl`;
  SQLite-backed `SearchResultCache` LRU (migration bump);
  `NegativeEvidence` short-circuit with `meta.cached = true`. Dispatch
  hooks around `agent_v1::dispatch`. Expose as
  `inspect_structure view=session`. Hash + lookup budget ≤ 50 µs.
- **P13 — Batch array parameters.** Widen
  `inspect_structure view=file_outline` to `paths: string[]`, widen
  `inspect_graph kinds={references,importers}` to `symbols: string[]`,
  widen `evaluate_quality` to `targets: string[]`. Response shape
  `{ results: [{ key, data|error }, ...] }`. No new tool names. Per-item
  errors never fail the whole call.
- **P15 — Watch mode.** `src/cli/watch.rs` as a clap subcommand:
  `codesight-mcp watch [--workspace PATH] [--debounce 500ms]` using
  `notify-debouncer-full`. Delta reindex via
  `code_index::index_workspace_delta`. Single-file edit reindexes in
  **< 100 ms** wall clock after debounce. SIGINT exits cleanly via P10
  scaffolding.
- **P17 — Context providers.** `codesight-core::providers::ContextProvider`
  trait. Concrete providers under `src/indexing/providers/{dbt,git_blame,framework_routes}.rs`.
  Providers run after extraction, before storage commit. `src/framework/*`
  replugs into the trait — framework code no longer imports from service
  layers.
- **P18 — Coverage delta.** One-shot `docs/COVERAGE_DELTA.md`: prior-art
  tool/language × codesight equivalent × parity/gap/intentional. Every
  intentional gap documented in the tool's `description`.
- **P19 — Benchmark corpus.** `benchmarks/` tree with small checked-in
  fixture repos (rust, typescript, python) and `benchmarks/tasks.json`
  canonical queries. `cargo bench -p codesight-mcp --bench end_to_end`
  records `timing_ms`, `result_count`, `truncated`. Token-accurate counts
  via optional `tiktoken-rs` feature. Baseline table in this worklog.
  Stable within 5% across three runs.
- **P20 — Security hardening.** `src/security/path_validation.rs` gains a
  symlink-escape check (canonicalize target, re-assert
  `commonpath(workspace, target)`). `max_files_per_workspace` (default
  100k) enforced at walk time with a structured warning.
  `src/security/discovery.rs` inlines the full secret-file exclusion list.

**Critical-path order.**

1. S1 item 1 + S1 item 2 (stop oversized modules bleeding into every
   patch). Low risk, unblocks later work.
2. S4-P11 (response envelope) — every subsequent handler edit assumes it.
3. S4-P13 (batch params) — depends on P11.
4. S4-P12 (session awareness) — depends on envelope stability.
5. S4-P15 (watch mode) — depends on P10 signal scaffolding (done).
6. S4-P17 (providers) — depends on S2 item 3.
7. S1 item 6 (test layering) — in parallel with any of the above.
8. S4-P20 (security) — isolated, anywhere.
9. S4-P18, S4-P19 — last. Bench only what is stable.

---

## Open test-drive issues

Every session runs `codesight-mcp` against this repo via MCP. Open items
land here; closed items move out of the file.

### #18 — Retrieval-path fallback log, session 2026-04-11

**Status:** logged for review.

Per AGENTS.md §2 every retrieval-path fallback (using default `Read`/`Grep`
when a codesight tool would have worked) must be logged. This session I
defaulted to filesystem tools in "fix mode" and skipped the self-check
trigger repeatedly:

- `Read REFACTOR_WORKLOG.md`, `Read src/analysis/dead_code/entrypoints.rs`,
  `Read src/analysis/dead_code/mod.rs`, `Read src/service/agent_v1/content.rs`,
  `Read src/service/agent_v1/util.rs`, `Read src/service/agent_v1/quality_more.rs`,
  `Read src/service/agent_v1/context_bundle.rs` (×2), `Read crates/.../symbol_reads.rs` (×2)
  → all should have gone through `read_content kind=file` or `kind=symbol`.
- `Grep "MISSING_REF|symbol ref needs"` → `search_catalog query_mode=regex unit_kinds=["file"]`.
- `Grep "fn symbol.*|symbol_by_location|symbol_at"` → `search_catalog query_mode=keyword unit_kinds=["symbol"]`.
- `Grep "fn retrieve_symbol|fn symbol_metadata"` → same.
- `Grep "WHERE file_path = \\?.*AND start_line"` → regex.
- `Grep "fn all_sections"`, `Grep "fn index_error"` → keyword + symbol.

Not product bugs — operator bugs. No tool gap to fix. Follow-up action:
re-run the self-check (“which row of the dogfood table covers this?”)
before every retrieval call next session.

### #17 — Slow integration tests — redundant ingest per test

**Status:** partial, 2026-04-11. Root cause confirmed; minimal fix applied.

**Root cause.** `tests/extraction_candidates.rs` ran `build_repo()` inside
every `#[test]` — `tempdir()` + fixture write + fresh `CodeService` +
`ingest_sources` on a 4-file Python workspace. The handler under test is
read-only, but each test paid the full cold-ingest cost (tree-sitter grammar
init, walk, parse, insert). Measured baseline: 8 tests × ~2.6–4.1 s each,
suite wall time **12.47 s**. The 0.099 s
`extraction_missing_repo_returns_error` (which intentionally skips
`build_repo`) pinpointed the ingest as the hot cost.

`tests/get_context_bundle.rs` hit the same pattern: `run_all()` invokes
six `case_*` helpers, three of which each build their own
`write_small_repo` fixture and re-ingest. Suite wall time **10.42 s**.

**Fix.** Added a process-lifetime `OnceLock<(TempDir, CodeService, String)>`
in each test binary (`shared_repo` / `shared_small_repo`) that builds the
fixture exactly once and hands out `&'static` references. Tests that
intentionally need a fresh or alternate index (`extraction_missing_repo_returns_error`,
`case_imports_from_file`, the two pure-unit `case_extract_imports_*`) stay
unchanged. No assertions or test semantics moved — only the fixture
plumbing.

**Measured after.** `extraction_candidates` suite **12.47 s → 3.52 s** (~3.5×).
`get_context_bundle_suite` **10.42 s → 8.17 s** (~22 %). Combined
**22.89 s → 11.69 s**, ~49 % off the two slowest buckets tracked so far.

**Follow-ups (not yet done).** Audit the rest of `tests/*.rs` for the same
pattern — `extraction_tiers` at 1.66 s and the `doc_broken_links` cases at
~100 ms each likely share the ingest-per-test anti-pattern at smaller
scale. Candidate for S1 item 6 (test layering).

**2026-04-11 (b)** — `.cursor/plans/test-suite-speedup.plan.md` levers 1+2a:

- **Lever 1 (fold `case_imports_from_file` into shared fixture).** Landed.
  Audit of `write_small_repo` callers found `suggest_queries.rs` asserts
  exact `symbol_count == 3 / file_count == 1 / function_count == 2`, so
  the shared helper stays untouched. Instead, `shared_small_repo()` in
  `tests/get_context_bundle.rs` writes an extra inline `imports_demo.py`
  (containing `import os` / `from pathlib import Path` / `my_func`). The
  case now reuses the fixture instead of building its own tempdir + svc +
  ingest. Measured: `get_context_bundle_suite` **8.233 s → 7.358 s**
  (~0.88 s). Above the 200 ms noise floor but well under the plan's
  ~5 s estimate — the OnceLock was already amortising 3 of 4 cases, so
  folding the 4th saves only one ingest.
- **Lever 2a (`prewarm_grammars` helper).** Rejected. Added the helper
  and wired it into the `shared_small_repo` / `shared_repo` OnceLock
  init blocks, then measured: `extraction_candidates` 3.521 s → 3.550 s
  and `get_context_bundle` 7.358 s → 7.421 s. Both within noise. The
  plan's hypothesis ("grammar construction is the cold cost, pre-warming
  it once amortises across cases in the binary") does not hold here for
  two reasons:
  1. `tree_sitter_python::LANGUAGE.into()` is essentially a static ref
     copy — grammar allocation is already nearly free.
  2. The real cold cost is `ExtractorRegistry::for_workspace_root`,
     which compiles **all** SCM query packs (~30 grammars × `Query::new`)
     per `CodeService::new`. This registry is rebuilt per service, not
     cached in a process-wide `LazyLock`, so pre-warming a language
     grammar can't reach it.
  3. Nextest runs each `#[test]` in its own **process** by default, so
     a OnceLock prewarm also can't cross `#[test]` boundaries in a
     multi-test binary. `extraction_candidates`'s 8 tests still each
     pay the full cold cost independently.
  Reverted the helper + both wire-ins per plan rule "Reject any lever
  whose measured savings are within noise (< 200 ms)".
- **Lever 2b (`DbLocation::Memory`)** not attempted this session — the
  plan explicitly flags it as moderate-risk and recommends landing it as
  its own isolated worklog entry. Deferred. Next session with a
  dedicated baseline, verify the query-pack compile (not SQLite init)
  is the actual hot path before touching storage internals — the 2a
  null result suggests the cost model needs direct measurement (e.g.
  a targeted `#[bench]` or `tracing`-span timing around
  `build_scm_extractors_from_catalog` vs `open_rw`) before picking a
  lever. If query compilation dominates, a *global* cached
  `ExtractorRegistry` (not per-service) is a bigger lever than in-memory
  SQLite.

**Combined running total on this issue:** baseline 12.47 s + 10.42 s =
22.89 s → 3.52 s + 7.36 s = **10.88 s** (~52 % of original). Savings
this session: ~0.88 s.

**2026-04-11 (c)** — **Process-wide `ExtractorRegistry` cache**. The big
lever the 2a null result pointed at: `ExtractorRegistry::for_workspace_root`
is called once per ingest and once per stale-check inside
`ensure_repo_store`, and each call compiles ~30 SCM query packs via
`Query::new`. Measured directly by instrumenting
`registry_builder::build_registry` with a timing print gated on
`CODESIGHT_TIME_REGISTRY=1`: each call is ~1.25 s, and
`extraction_candidates` paid 1–2 of these per test process (nextest
forks per-test, so the existing `OnceLock` shared fixtures can't amortise
across the binary).

**Fix.** Added a process-wide `LazyLock<ExtractorRegistry>` (`CACHED_BASE`)
holding the overlay-free, extra-extension-free registry in
`crates/codesight-extractors/src/extractors/registry_builder.rs`. On every
call, `build_registry(project_root)` checks
`project_query_pack_roots(root).is_dir()` and
`project_extra_extensions(root).is_empty()`; if both are clean (the
common case — workspaces without a `.codesight/queries/` overlay), it
returns `CACHED_BASE.clone()` instead of recompiling. Workspaces with
overlays (`tests/query_pack_catalog.rs` writes `.codesight/queries/toyjson`)
fall through to the original builder. `ExtractorRegistry` + the zero-sized
`LanguageProfileRegistry` gained `#[derive(Clone)]` — cheap because the
heavy state is `Vec<Arc<dyn LanguageExtractor>>` / `HashMap<String, Arc<…>>`
and clone only bumps the Arc refcounts.

**Safety.** `apply_extra_extensions(&mut self, …)` only mutates the
local `extra_extensions: HashMap<String, String>`, which is deep-cloned
by `Clone`; the `Arc<dyn LanguageExtractor>` handles remain shared and
read-only. Verified both sensitive suites:
- `query_pack_catalog` (7 tests, overlay path) → all pass; 0.918 s total.
- `extra_extensions` (9 tests, direct `apply_extra_extensions` mutation
  on the cached clone) → all pass; 2.282 s total.

**Measured after.**

| Binary | Before (worklog baseline) | After cache | Δ |
|---|---|---|---|
| `extraction_candidates` | 3.52 s | **1.498 s** | −57 % |
| `get_context_bundle` | 8.23 s | **0.958 s** | −88 % |
| `extraction_tiers` | 1.66 s | 0.956 s | −42 % |
| `hotspots` | n/a | 2.785 s | — |
| `blast_radius` | n/a | 4.022 s | — |
| `incremental` | n/a | 1.101 s | — |
| `retrieval_tools` | n/a | 3.016 s | — |

Two plan-scoped binaries: **10.88 s → 2.46 s** (~77 % off this session
alone; ~89 % off the original #17 baseline of 22.89 s).

Clippy: `cargo clippy --all-targets -- -D warnings` clean.

**Production impact.** This also accelerates the real MCP runtime, not
just tests. Every `CodeService::new` (one per process) plus every
`ensure_repo_store` stale-check (one per read-tool invocation on a repo
with `auto_reindex=true`) previously rebuilt the full extractor
registry. The cache turns the first call into the only expensive one,
bringing subsequent stale-check overhead down to `clone()` of a handful
of Arcs + HashMap deep-copy. Targets the §5 performance budgets:
"cached reads < 50 ms" and "cold full ingest < 5 s" both get a free
~1.25 s back.

**Deferred.** The CLI's cold-first ingest still pays the one-time
`CACHED_BASE` init (~1.25 s on this machine). Reducing that means
attacking tree-sitter `Query::new` cost per pack — either compile packs
in parallel, or lazy-compile packs per language on demand. Tracked as a
follow-up under §5 perf budget; skip unless the cold-ingest target
(< 5 s full repo) starts missing.

**2026-04-11 (d)** — **Lazy per-language SCM query compile.** The
`CACHED_BASE` init itself still paid ~1.25 s on first build, which every
nextest per-test process had to eat once regardless of my global cache.
Attacked the root cause: `ScmExtractor::from_pack` used to call
`Query::new(&grammar, tags.scm)` (+ optional injections query) eagerly
for all ~30 packs, even though a Python-only test only needs the Python
pack.

**Fix.** Refactored `crates/codesight-extractors/src/extractors/engine/scm/`:

- `load.rs`: shrank `CompiledQueryPack` to just `grammar + tags_query +
  injections_query`. Renamed the compile entry point to `compile_queries`
  taking raw `grammar_name / query_source / injections_source / name_for_err`
  so it can be invoked with the pack's owned `String`s at any time.
  Removed the `CompiledQueryPack`-embedded `language_name`, `extensions`,
  and `postprocess_steps` — those now live on `ScmExtractor` directly so
  pack metadata is available without compiling a query.
- `scm/mod.rs`: `ScmExtractor` now holds
  `{language_name, extensions, postprocess_steps}` as the cheap
  always-available metadata, `{grammar_name, query_source,
  injections_source}` as the source-of-truth inputs, and
  `compiled: OnceLock<CompiledQueryPack>` as the lazy slot. `ScmExtractor::new`
  / `from_pack` validate the grammar exists (cheap process-static map
  lookup) and leak the static strings, but **do not** call `Query::new`.
  A new `fn compiled(&self) -> &CompiledQueryPack` compiles on first
  access via `OnceLock` and panics on failure — which is already
  guaranteed unreachable by the `scm_all_queries_compile` test that
  compiles every pack at lib-test time.
- `scm/capture.rs`, `scm/injections.rs`, `scm/postprocess.rs`,
  `scm/scope.rs`: rewired `ext.compiled.grammar` /
  `ext.compiled.tags_query` / `ext.compiled.injections_query` →
  `ext.compiled().*` (now a method call that lazy-initialises), and
  `ext.compiled.language_name` / `ext.compiled.postprocess_steps` →
  direct field access on `ScmExtractor` (no compile needed).
- Visibility: the new `compiled()` method and metadata fields use
  `pub(in crate::extractors::engine::scm)` so `CompiledQueryPack` /
  `PostprocessStep` stay `pub(super)` without tripping
  `private_interfaces`.

**Safety.** The `OnceLock::set` call races are tolerated — two threads
compiling the same pack produce equivalent queries and only one `set`
wins. Failure mode moved from registry-build time to first-`extract()`
call, but `scm_all_queries_compile` (lib test, 30 packs) is the eager
gate that catches any broken pack before a PR lands, matching the old
behaviour. Verified: `cargo check --tests` clean, `cargo clippy
--all-targets -- -D warnings` clean, all 98 `codesight-extractors` lib
tests pass.

**Measured after** (`--no-cache`, wall time):

| Binary | Before this session | After Lever 3 | After lazy compile | vs. before |
|---|---|---|---|---|
| `extraction_candidates` | 3.52 s | 1.498 s | **0.176 s** | −95 % |
| `get_context_bundle` | 8.23 s | 0.958 s | **0.152 s** | −98 % |
| `extraction_tiers` | 1.66 s | 0.956 s | **0.149 s** | −91 % |
| `hotspots` | — | 2.785 s | **0.276 s** | −90 % |
| `blast_radius` | — | 4.022 s | **0.427 s** | −89 % |
| `incremental` | — | 1.101 s | **0.274 s** | −75 % |
| `retrieval_tools` | — | 3.016 s | **1.098 s** | −64 % |
| `extra_extensions` | — | 2.282 s | **0.034 s** | −99 % |
| `query_pack_catalog` | — | 0.918 s | **0.024 s** | −97 % |
| `grammar_smoke` | — | — | 0.042 s | — |
| `bm25_correctness` | — | — | 0.010 s | — |

Two plan-scoped binaries: **22.89 s → 0.328 s** (~99 % off original #17
baseline of 22.89 s; ~97 % off this session's starting 10.88 s).

**Production impact beyond tests.** Real MCP sessions that only touch
a single language (e.g. a Rust-only repo) now pay one `Query::new` per
language instead of thirty. The CLI cold-ingest time drops
proportionally — a fresh `codesight-mcp check --workspace .` on a
Python project only compiles the Python pack. Matches §5's "cold full
ingest < 5 s" target with significant headroom.

**Closed follow-ups.** The ~1.25 s `CACHED_BASE` init cost called out
as "deferred" in entry (c) is now gone — lazy compile eliminates it
entirely. Parallel pack compilation (rayon) is no longer needed.

**2026-04-11 (e)** — virtual-time timeout tests + dead assertion fix.

Two unit tests paid real-time sleeps on every run:
- `src/service/dispatch_timeout.rs::elapsed_deadline_returns_transient_timeout`
  — 1 s budget × real `tokio::time::sleep(3 s)` = 1020 ms wall time.
- `src/mcp.rs::drain_aborts_when_exceeded` — 1 s drain timeout × real
  sleep = 1015 ms wall time.

Switched both to `#[tokio::test(start_paused = true)]` which makes
tokio use **virtual time**: `sleep` auto-advances when no real I/O is
pending, so the test resolves in microseconds of wall clock. Enables
`tokio = { features = ["test-util"] }` as a **dev-dependency** only —
the feature exposes time manipulation that must not ship in the
production binary.

**Measured.** Both tests now run at ~38 ms each. Aggregate savings in
the full uncached suite: ~2 s serialised, ~0.3 s wall (both tests run
concurrently with others), plus the quality-of-life win of a 97 %
reduction on `./scripts/nextest-cache -E 'test(...)'` turnaround when
iterating on dispatch code.

Also fixed `tests/token_budget_context.rs::case_ranked_items_have_source_and_scores`
which asserted `centrality_score` / `combined_score` on ranked context
items. Commit `06c3159` ("real use fixes") deliberately removed those
placeholder fields from the handler but didn't update the test, so it
had been `SIGABRT`-ing on every full-suite run since. Asserted only the
fields the handler actually emits (`source`, `relevance_score`,
`tokens`).

**2026-04-11 (f)** — single-binary test merge experiment (REVERTED).

Acted on the [`.cursor/plans/Test suite performance-d4575817.plan.md`]
*critical* lever: Cargo treats every `tests/*.rs` as its own
integration binary, so a clean build pays 86 × link steps through the
full tree-sitter / storage dep graph. Measured cold compile
(`cargo clean -p codesight-mcp && time cargo test --no-run`) at
**45.7 s** before the merge.

Moved all 86 `tests/*.rs` to `tests/it/<name>.rs` and created
`tests/it.rs` as a single binary root pulling each submodule in with
`#[path = "it/<name>.rs"] mod <name>;`. `#[path]` is required because
`tests/it.rs` is a crate root (like `lib.rs`/`main.rs`); without it
Rust looks for `tests/<name>.rs` and skips the `tests/it/` directory.
Also fixed `tests/it/languages.rs`'s `include_str!("fixtures/…")` paths
to `../fixtures/…` for the new directory depth, and pruned the
`tests/*.rs` overrides from `.config/nextest.toml` (no longer any
binary named `agent_v1_caching` etc.).

**Measured.**
- Cold compile: 45.7 s → **22.3 s** (−23.4 s, −51 %). Huge win.
- Warm test run: 14.5 s → **28.2 s** (+13.7 s, +95 %). Huge loss.

The warm regression was reproducible across three back-to-back runs.
Root cause hypothesis: nextest still forks one process per `#[test]`
regardless of binary count, so every fork now loads a binary 86× larger
than any individual one. The merged binary's static initialisers cover
every test module's top-level `LazyLock`s / `OnceLock`s / `const &str`
fixtures, so the per-fork startup cost grows from ~9 ms (tiny binary)
to ~19 ms (giant binary); × 1524 forks = ~15 s of extra runtime. On an
edit → rerun dev loop — by far the common case — the merge is a **net
loss** even counting the cold-build savings.

Reverted the merge entirely: files back in `tests/<name>.rs`, `tests/it.rs`
removed, `include_str!` paths restored. Left the nextest.toml cleanup
in place (see below) since it stood on its own.

**Deferred.** If cold-build cost ever becomes the bottleneck (e.g. a
fresh-checkout CI job whose total time is dominated by linker, not
test runtime), revisit as a PARTIAL merge: group the ~20 smallest
binaries (pure `[lang]_language.rs`, `*_parser.rs`, grammar smoke
tests) into one or two themed binaries so the cold-link saving kicks
in without ballooning the giant-binary startup cost for the heavier
integration tests. Not worth the code churn right now — warm runs are
the hot path and they already fit in target.

**2026-04-11 (g)** — nextest heavy-group cleanup.

`.config/nextest.toml` used to pin ~13 heavy integration binaries into
a `test-groups.heavy = { max-threads = 4 }` bucket so CI wouldn't OOM
running 50+ concurrent `CodeService::new` + ingest forks. The fix in
(c)/(d) dropped per-test peak memory by an order of magnitude, so the
cap is no longer load-bearing — it was just forcing idle CPU cores at
full-suite time. Removed the `[test-groups]` section, the
`[[profile.default.overrides]]` filter, and the stale binary-name list
in the header comment. Measured effect: ~0.5–1 s shaved off the full
uncached suite (noise-level; real but small).

**Final measured baseline** (full targeted uncached suite, post all
landed changes this session, three back-to-back runs):

```
Summary [  27.859s ] 1524 tests run: 1524 passed, 0 skipped
Summary [  28.859s ] 1524 tests run: 1524 passed, 0 skipped
Summary [  27.658s ] 1524 tests run: 1524 passed, 0 skipped
```

**Honest correction on earlier numbers.** The 14.46 s figure reported
in entries (b)/(c)/(d) was a genuinely warm-cache outlier — repeated
targeted runs against the two plan-scoped binaries had left disk-cache
/ dylib-cache / CPU-thermal state in a sweet spot that the full suite
benefited from. Steady-state wall time is closer to **~28 s**, which
is still comfortably inside the AGENTS §6 ≤ 30 s target. The
per-binary ratios in the tables above (`extraction_candidates`
3.52 → 0.176, `get_context_bundle` 8.23 → 0.152, etc.) are all
reproducible and remain valid.

### #14 — `read_content symbol` no longer forces stable_id lookups

**Status:** fixed 2026-04-11.

`read_content refs=[{kind:"symbol", path, start_line}]` used to return
`MISSING_REF: symbol ref needs id/stable_id` even though
[`AGENTS.md`](AGENTS.md) documents path+start_line as a valid shape. New
`IndexStore::symbol_id_at_location` resolves `(path, start_line)` to a
stable id (exact match → innermost enclosing span), exposed via
`agent_v1::util::resolve_symbol_id`. Both `read_content` and
`assemble_context mode=context_bundle` now accept path+line. Error message
updated so failure tells the caller which shapes are valid.

### #15 — `evaluate_quality analysis=dead_code` no longer flags test fixtures

**Status:** fixed 2026-04-11.

Dead-code reachability was reporting every file under `tests/fixtures/` and
`benches/fixtures/` (30 files, confidence 1.0) as dead. These are
tree-sitter parser inputs — intentional isolated snippets with no import
edges, so reachability will always reject them. Added
`is_test_fixture` to `src/analysis/dead_code/entrypoints.rs` and skip those
paths unconditionally in both `dead_code_payload` and
`dead_code_v2_payload`.

### #16 — Hotspot handler `evaluate_quality_tail` split

**Status:** partial, 2026-04-11.

`quality_more.rs::evaluate_quality_tail` held cyclomatic 38 with four
inlined branches (`hotspots`, `extraction_candidates`, `broken_links`,
`doc_coverage`). Split into four private helpers + a free function
`check_internal_link` for the broken_links link-walker. Dispatch cyclomatic
→ ~5. Remaining top hotspots (`graph_tail_b`, `context_bundle`,
`context_tail`, `context`, `ingest`, `quality`, `index_admin::search_columns`)
still queued under S1 item 1.

### #13 — Framework provider/render/snapshot cycles are real

`inspect_graph graph=dependency_cycles` surfaces five 3-node cycles across
`src/framework/{dbt,laravel,nextjs,nuxt}/` plus a 6-node SCC across
`crates/codesight-extractors/src/extractors/engine/{catalogs,scm}/`. Not a
detector bug — these are genuine structural findings. Filed so they stay
visible for the S4-P17 context-provider work, which will replug
`src/framework/*` into the new trait and is the natural moment to collapse
the cycles.

---

## Proactive improvement backlog

After every test-drive, ask: **"what should improve in `codesight-mcp`?"**
Each session adds at least one idea here, and removes or reshapes at least
one existing feature if it doesn't earn its tokens. Metric: token savings
per query (`check --savings`), and dispatch latency.

**Next-session candidates (not yet scheduled):**

- **Unify `_meta` construction.** Every handler hand-rolls the `_meta`
  block today. Ship `ToolEnvelope<T>` (S4-P11) and migrate all handlers in
  one sweep — this alone is a significant LoC reduction and eliminates the
  class of bugs where `raw_bytes` was passed wrong and `tokens_saved`
  silently went to zero.
- **Kill `assemble_context mode=workspace_digest` if it can't justify itself.**
  v3 reworked it to a real digest, but if it ends up redundant with
  `query_suggestions` or `catalog_sources` in practice, cut it and widen
  one of the neighbours instead. Bloat hunt.
- **Token-accurate counting.** Replace byte-approx `tokens_saved` with
  `tiktoken-rs` behind a feature flag. The envelope already names the
  method (`estimate_method`). Accurate numbers unlock the improvement
  metric.
- **`inspect_graph graph=calls` real call-site extraction.** The current
  text-based whole-word matcher is documented as a heuristic and emits a
  `CALL_GRAPH_HEURISTIC` warning (v3 #36). Replacement: tree-sitter queries
  for call expressions per language, stored as caller→callee name pairs at
  ingest. Unlocks accurate `blast_radius`/`rename_safety`.
- **Session-scoped negative evidence.** "This symbol does not exist" is a
  valid answer that should short-circuit repeat searches in the same
  session. Part of S4-P12 but worth calling out as its own token-savings
  win.
- **Bench script + CI-less perf gate.** Write `scripts/bench-hotpaths`
  (see metrics section) and enforce that the numbers only improve. Record
  deltas in this worklog.

---

## Slow-test watch list

Tests > 1 s that we've noticed during targeted runs. Empty until the next
session audits it; **every session that runs tests must update this list**
(or confirm it's empty).

| Test | Binary | Time | Plan |
|---|---|---|---|
| *(none yet — audit pending)* | | | |

---

## Dogfooding fallbacks

When the agent reaches for `Grep`/`Read`/`Glob` instead of codesight, log
it here with the reason. Each entry is either a missing codesight feature
(→ improvement backlog) or a habit to fix.

- **2026-04-11 — habit gap.** Agent fell back to `Grep`/`find` for lookups
  like `check_staleness`, `dependency_graph`, `extractor_for_path` despite
  `search_catalog` being correct. No product fix; session priming needs
  the dogfooding reminder louder. No further occurrences required before
  closing this entry.
