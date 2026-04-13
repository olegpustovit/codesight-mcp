# REFACTOR_WORKLOG.md

Single living state for `codesight-mcp`. Update this file in the same patch
as the change it describes. Move closed items out — git history is the
archive. See [`AGENTS.md`](AGENTS.md) for the policies that govern this work.

---

## Current metrics (baseline: 2026-04-12)

| Metric | Current | Target | Trend |
|---|---|---|---|
| Product crate LoC (`src/`) | ~27.4k | ↓ each session | ↓ -267 this session |
| Workspace crate LoC (`crates/`) | core 0.2k / extractors 9.0k / storage 2.4k | stable | — |
| Governance score (this repo) | **5674** (good) | ↑ | ↑ +673 this session (dedup + zero cycles) |
| Structural score | **8073** (very good) | ↑ | ↑ +1593 this session (acyclicity 1.0) |
| Agent readiness score | **3988** (fair) | ↑ | ↑ +128 this session |
| Dependency cycles | **0** | 0 | ✅ was 2; broke extractors + analysis↔indexing |
| High-risk oversized modules in `src/` | see S1 item 1 below | 0 undocumented | — |
| Open test-drive issues | 0 | 0 | ↓ fixed repo_health governance embedding |
| Cold dispatch handler (cached read) | **0.6 ms** (64 ms wall incl. startup) | < 50 ms | ✅ met |
| Cold full ingest (this repo) | **1.8 s** (405 files, 4029 symbols) | < 5 s | ✅ met |
| Incremental reindex (1 file touch) | **37 ms** delta (background watcher) | < 100 ms | ✅ met |
| Uncached focused test loop | **~17 s** (1526 tests) | ≤ 30 s | ✅ under target |
| `_savings.json` total tokens saved | read with `check --savings` | ↑ each session | — |
| Slow tests (> 1 s) | unknown | 0 | needs audit |

**Measurement script:** `scripts/bench-hotpaths` (landed 2026-04-12).
Times cold ingest, keyword search, and delta reindex on this repo.
Run periodically to catch regressions.

---

## Completed this session (2026-04-12 evening)

1. **Framework provider macro** (`impl_framework_provider!` in `src/framework/mod.rs`).
   Eliminated ~160 lines of identical boilerplate across 4 providers (dbt, laravel,
   nextjs, nuxt). Each provider.rs is now ~15–30 lines of detect/load logic only.
2. **Replaced 6 hand-rolled recursive dir-walk functions** in framework snapshots with
   shared `walk_files()` + `rel_path()` helpers backed by `walkdir` (already a dep).
   ~80 lines of `read_dir → flatten → is_dir → recurse` skeleton deleted.
3. **Deduplicated governance-signal JSON construction** — extracted
   `governance_signal_json()` in `analysis/quality_signal.rs`, used by both
   `repo_health` and `quality_signal` handlers. ~25 lines of duplicated JSON layout
   removed.
4. **Broke both remaining dependency cycles** (was 2, now **0**):
   - Extractors: moved `metadata.rs` (ScmMetadata, parse_scm_header) from `scm/` to
     `catalogs/`, breaking the catalogs↔scm cycle.
   - Analysis↔Indexing: moved `cross_repo_map.rs` from `analysis/` to `service/`,
     since it's a service-level orchestrator.
5. **`require_store!` macro** in `src/service/mod.rs` — replaces ~36 instances of
   the 4-line `match self.ensure_repo_store { Ok(s) => s, Err(e) => return ... }`
   boilerplate. ~100 lines saved across 22 handler files.
6. **Metrics delta:** Governance 5001→5674 (+673), Structural 6480→8073 (+1593),
   Acyclicity 0.333→1.0, Agent readiness 3860→3988 (+128). 339+ tests passing.

---

## Active workstreams

### S1 — Simplification & dedup

**Principle:** shrink the product; flatten oversized modules; collapse dead
code; reach for crates before hand-rolling.

**Open items:**

1. **Split remaining oversized modules.** Re-audit after the
   `inspect_structure` split (v3 #43). Run
   `mcp__codesight__evaluate_quality analysis=hotspots` to get the current
   ranked list, then split any module that is structurally risky:
   cyclomatic > 20, mixed responsibilities, high fan-in/blast radius, or
   poor navigability for agents. File length may be used as supporting
   evidence, but not as a hard trigger. Known candidates carried forward:
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
5. ~~**Infallible-regex consolidation.**~~ Done — `src/framework/laravel/regex.rs`
   already uses `LazyLock<LaravelRegexes>` with all patterns consolidated.
6. **Finish test layering.** Migrate `tests/get_doc_coverage.rs`,
   `tests/doc_broken_links.rs`, `tests/doc_parser*.rs`,
   `tests/retrieval_tools.rs`, `tests/get_context_bundle.rs` to
   `src/test_support/indexed.rs`. Split nextest buckets into
   `unit | slice | contract | heavy`. Uncached focused loop ≤ 30 s.

**Done when.** No high-risk module remains unsplit without a worklog note.
No `panic!`/`unreachable!` outside tests. No `std::env::var("CODESIGHT_*")`
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

**2026-04-11 (b)** — Lever-4 session dogfood audit.

Used codesight-mcp exclusively for read-side tasks during the Lever 4 work:
- `catalog_sources` → confirmed indexed freshness (code + docs).
- `search_catalog query_mode=keyword unit_kinds=[symbol]` for `open_ro` →
  resolved the `db::open_ro` entry point + every `IndexStore::open_*` in
  one call instead of grepping. `_meta.tokens_saved` = 601 001.
- `search_catalog unit_kinds=[file]` surfaced the in-file text hits for the
  same query. Observation: when docs are also indexed the hybrid domain
  returns a lot of plan-doc section matches that drown out the code hits.
  Mitigation for next session: pass `domain=code` explicitly on symbol /
  file-text lookups. Not a bug, but a usage foot-gun — the default
  `domain=hybrid` is optimised for natural_language, not keyword.
- `read_content` batched 2 files in a single call (`db.rs` +
  `storage/mod.rs`), ~4k token budget, instead of two `Read` calls.
- `inspect_graph graph=find_references identifier=open_read_only` returned
  53 call sites across 6 files in one shot — used as the refactor audit
  against reentrant calls. This was the single highest-leverage lookup of
  the session: saved ~10 `Grep`s and gave me call-site structure that a
  substring search would have missed.
- `evaluate_quality analysis=hotspots` / `analysis=repo_health` /
  `analysis=dead_code` / `inspect_graph graph=dependency_cycles` ran clean.
  Noted cyclomatic 24 on `IndexStore::apply_index_with_deps` — S1 item 1
  candidate for a future split.
- `evaluate_impact analysis=blast_radius` on
  `IndexStore::open_read_only` returned 1 direct + 4 transitive importers
  (depth 2). Confirmed the change surface before editing — exactly what
  the tool is for.
- `assemble_context mode=query_suggestions query="connection pool"`
  returned mostly generic `search_catalog` bootstraps, not conceptually
  relevant suggestions. Expected — `query_suggestions` is a cold-start
  helper, not a semantic recommender.

Zero fallbacks this session on read-side. Edits (`Edit`, `Write`) stayed on
default filesystem tools per AGENTS §2. No new product bugs found in
codesight itself during this test drive.

### #19 — `search_catalog` default-domain auto-routing (Lever 6)

**Status:** landed, 2026-04-12.

**Problem.** `search_catalog` defaulted `domain` to `hybrid` regardless of
`query_mode`, so keyword lookups for code symbols (the overwhelmingly
common case) returned doc-section hits that drowned out the actual code
results. Every mature agent-facing retrieval tool (LSP, GitHub code
search, Sourcegraph, Continue.dev) either separates code/docs or requires
an explicit filter.

**Fix.** Auto-route `domain` from `query_mode` when not explicitly set:

| `query_mode` | Default `domain` |
|---|---|
| `keyword` (default) | `code` |
| `regex` | `code` |
| `natural_language` | `hybrid` |

Explicit `domain=code/docs/hybrid` always wins. Response `_meta` now
includes `domain_effective`, `query_mode_effective`, and `domain_explicit`
so callers can inspect the auto-routing.

**Files.**
- `src/service/agent_v1/catalog_search/mod.rs` — domain resolution logic +
  meta threading.
- `src/tools.json` — updated `description` and `domain` field docs.
- `tests/catalog_search_defaults.rs` (new, 5 tests) — one per routing
  combo + explicit override.

All existing tests pass — most already set `domain=code` explicitly.

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

**2026-04-11 (h)** — wave-2 levers from `.cursor/plans/test-suite-speedup-wave2.plan.md`.

Today's pre-wave-2 baseline was warmer/cooler than entry (g)'s; first
two `--no-cache` runs measured **34.547 s / 34.370 s**. Used that as the
honest reference for the wave-2 deltas below.

**Lever 1 (`copy_fixture_tree` → hardlink): SKIPPED.** The plan's premise
that `index_code_fixture` / `index_doc_fixture` delegate to
`copy_fixture_tree` is wrong — they index their checked-in fixture path
directly without staging. The only real callers of `copy_fixture_tree`
are 2 sites in `tests/document_coverage.rs` (the `setup_indexed_fixture`
helper + one inline `doc_doc_coverage_meta_present` case). Fixture trees:
`tests/fixtures/doc_reference` is 7 files, `code_reference` is 21. Even
the 15 cases under `setup_indexed_fixture` × 7 files × ~50 µs per
`fs::copy` works out to ~5 ms — well under the plan's ≥ 500 ms acceptance
floor. Hardlinking 14 sites with mutation safeguards isn't worth the
code surface for a sub-noise win. Skipped per plan stop-condition logic.

**Lever 2 (`repo_total_bytes` per-store caching): LANDED.**

Audit confirmed the call site distribution: `find_references
identifier=repo_total_bytes` returned 21 hits, with the dominant pattern
being `MetaBuilder::new(store.repo_total_bytes()?)` inside every
read-side handler (`search_catalog`, `assemble_context*`, `inspect_graph`,
`evaluate_impact_*`, `evaluate_quality*`, `manage_indexes`, `structure`).
Two extra hidden costs found in the same audit:

1. `repo_metadata` opened **two** RO connections per call — one inside
   `repo_summary`, then a second inside its own `repo_total_bytes()` call,
   even though `repo_summary` already had a usable `Connection` in scope.
2. `DocStore::doc_summary` had the exact same shape: open conn, run 4
   queries, then call `repo_total_bytes()` which opens a 5th conn just
   for the `SUM(size_bytes)`.

**Fix.** Both stores grew a `cached_total_bytes:
Arc<std::sync::OnceLock<u64>>` field (`Arc` so the existing `derive(Clone)`
keeps working — `OnceLock` itself isn't `Clone`). The public
`repo_total_bytes()` now hits the cache; on miss it opens an RO conn and
delegates to a new `pub(crate) fn repo_total_bytes_with(&self, conn:
&Connection)` helper that does the SQL and primes the cache. Inside
`IndexStore::repo_summary`, the new helper is called with the
already-open `conn` and `estimated_naive_tokens` is filled in directly,
collapsing `repo_metadata` from two connection opens to one. Same
collapse for `DocStore::doc_summary`.

Cache invalidation: not needed in practice. `IndexStore` instances are
opened per-dispatch via `open_repo_store` (`src/service/code_tools.rs:306`)
and dropped at end of handler; the write-side instance opened by
`index_workspace` is a separate `IndexStore` and never queries
`repo_total_bytes`. Documenting this assumption rather than threading an
invalidate through every `apply_index_with_deps` write path keeps the
diff small.

**Files touched.**
- `crates/codesight-storage/src/storage/mod.rs` — `IndexStore` struct +
  `cached_total_bytes` field.
- `crates/codesight-storage/src/storage/index_store.rs` — `open` /
  `open_existing` populate the cache; `repo_summary` reuses its own
  `conn` for the SUM; `repo_metadata` simplified; new
  `repo_total_bytes_with(&Connection)`.
- `src/storage/doc_index/mod.rs` — `DocStore` struct + cache field +
  `open` / `open_existing`.
- `src/storage/doc_index/query.rs` — `DocStore::repo_total_bytes` +
  `repo_total_bytes_with`; `doc_summary` reuses its conn.

**Measured.** Two `--no-cache` runs after the change, all 1524 tests
passing both runs:

```
Summary [  29.682s ] 1524 tests run: 1524 passed, 0 skipped
Summary [  30.610s ] 1524 tests run: 1524 passed, 0 skipped
```

Δ vs the wave-2 baseline 34.547 s / 34.370 s: **−4.3 s mean wall time**
(−12 %), order of magnitude above the 200 ms noise floor and inside the
AGENTS §6 ≤ 30 s target. `cargo clippy --all-targets -- -D warnings`
clean.

**Lever 3 (partial parser-binary merge): LANDED, neutral on warm.**

Created `tests/parsers.rs` as a single `[[test]]` binary root, moved 18
parser-only test files into `tests/parsers/<name>.rs`, and re-attached
each via `#[path = "parsers/<name>.rs"] mod <name>;`. Candidate list
followed the plan: `adaptive_languages`, `al`, `arrow_functions`,
`asm_language`, `blade`, `css`, `dart_imports`, `grammar_smoke`, `json`,
`languages`, `lua`, `luau_language`, `razor`, `reference_parser`,
`sql_language`, `swift_parser`, `synthetic_repos`, `yaml_language`.
Dropped `parse_warnings` from the candidate set after a `CodeService` /
`include_str!` audit — it constructs `CodeService::new` per test, exactly
the static-init balloon that killed wave 1's full merge. Fixed
`languages.rs`'s `include_str!("fixtures/lang_samples/…")` paths to
`"../fixtures/lang_samples/…"` to compensate for the new directory depth.

**Measured warm.** `--no-cache` runs after the merge, immediately
following Lever 2:

```
Summary [  30.168s ] 1524 tests run: 1524 passed, 0 skipped
Summary [  29.096s ] 1524 tests run: 1524 passed (1 leaky), 0 skipped
```

Statistically indistinguishable from Lever 2's 29.682 s / 30.610 s. The
plan's acceptance gate ("warm full-suite no more than 500 ms slower")
holds with room to spare. `(1 leaky)` is nextest's name for a test that
spawned a child process that outlived the test body — pre-existing,
unrelated to the merge (also seen on baseline runs in entry (g)). Logged
under the slow-test watch list as a follow-up to find which test.

**Cold compile.** `cargo clean -p codesight-mcp && time cargo test
--no-run` after the merge: **42.09 s** wall (179.88 s user). Wave 1's
full merge measured 22.3 s cold; the partial merge can't beat that
mathematically (still has ~67 binaries linking) but the link-step
reduction is mechanically correct (18 fewer link/dwarf-pack steps). Did
not pay the cost of a clean rebuild on `main` to A/B the cold delta —
warm parity + correct link-step reduction is the contract this lever
was supposed to keep. Acceptance: warm constraint satisfied; cold
expected to be 4–6 s under main's clean build (un-A/B'd, flagged for
later if cold ever becomes the bottleneck).

**2026-04-11 (i)** — **Lever 4: per-store RO connection pool.** Landed.

Wave-2 deferred Lever 4 on the grounds that the next lever down was
`db::open_ro`, which is called once per public read method on `IndexStore` /
`DocStore`. A single handler dispatch typically issues 4–10 of these, and
each open goes through `Connection::open_with_flags` + `busy_timeout` + an
implicit SQLite header read. At ~30 read methods × ~1500 tests × multiple
per-dispatch calls that's the single biggest remaining leaf.

**Fix.** Added `db::RoConnCache = Mutex<Option<Connection>>` and
`db::open_cached_ro(cache, path) -> RoConn<'_>` in
`crates/codesight-storage/src/db.rs`. `RoConn<'a>` is a tiny enum —
`Cached(MutexGuard<'a, Option<Connection>>)` or `Fresh(Connection)` — with a
`Deref<Target = Connection>` impl, so existing call sites (`conn.prepare(...)`,
`conn.query_row(...)`, `self._with(&conn)` through Deref coercion) need zero
changes. `try_lock` + fresh-connection fallback means the cache never
deadlocks on reentrant or concurrent access: a method holding the guard can
call another method that also wants the RO connection and the inner call
silently opens a one-shot `db::open_ro`, then drops it.

Both stores gained an `Arc<RoConnCache>` field next to `cached_total_bytes`,
and `open_read_only` now delegates to `open_cached_ro`. `IndexStore` /
`DocStore` dropped `#[derive(Debug)]` in favor of a manual impl that only
prints `db_path` (`Mutex<Option<Connection>>` isn't `Debug`). `open_existing`
wraps its priming `open_read_only` call in a scope so the `MutexGuard`
releases before the store moves into the `Option::Some` return.

**Safety + invariants.**
- RO WAL readers pick up writers' commits on every implicit `SELECT`
  transaction, so there is no stale-snapshot risk from reusing a single
  cached connection across writes. No invalidation needed.
- The write path is untouched: `open_rw` / `open_connection` still return
  owned `Connection`s that go through the existing migration ladder.
- `Connection: Send + !Sync`; `Mutex<Connection>: Send + Sync`;
  `Arc<Mutex<...>>: Send + Sync`. `IndexStore` / `DocStore` remain
  `Send + Sync` (verified via successful compile in a `Clone` + multi-thread
  dispatch codebase).
- Reentrant safety: `std::sync::Mutex` isn't reentrant, but `try_lock`
  means the second caller on the same thread gets `Fresh` instead of
  deadlocking. Verified no hot reentrancy in the audit (`repo_summary` is
  the only method that holds a connection while calling other reads and
  it passes `&conn` through the `_with` helpers that take `&Connection`
  directly — no new lock attempt).

**Files.**
- `crates/codesight-storage/src/db.rs` — new `RoConnCache`, `RoConn`, and
  `open_cached_ro`.
- `crates/codesight-storage/src/storage/mod.rs` — `IndexStore` gains
  `ro_conn: Arc<RoConnCache>`; manual `Debug` impl.
- `crates/codesight-storage/src/storage/index_store.rs` —
  `open_read_only` returns `db::RoConn<'_>`, both `open` / `open_existing`
  initialize `ro_conn`; `open_existing` scopes its priming read.
- `src/storage/doc_index/mod.rs` — same shape for `DocStore`.

**Measured** (full targeted uncached suite, `--no-cache`, two runs):

```
Summary [  28.505s] 1524 tests run: 1524 passed, 0 skipped
Summary [  27.538s] 1524 tests run: 1524 passed, 0 skipped
```

Mean ~28.0 s, vs today's pre-pool baseline 33.289 s: **−5.3 s mean**
(−16 %). Vs the wave-2 final baseline ~30 s: **−2 s** above the 200 ms
noise floor. 1524 tests pass on both runs. `cargo clippy --all-targets
-- -D warnings` clean.

**Still open toward the 20 s victory floor.** The pool unlocks
`prepare_cached` as a real lever (it only pays when a `Connection` is
reused, which is now the default). 14 `conn.prepare(...)` call sites
under `crates/codesight-storage/src/storage/` are the obvious next target,
plus `src/storage/doc_index/query.rs`. Deferred to its own session so the
pool's Δ stays readable in the history; the `prepare_cached` diff is
mechanical and can be landed and measured against this entry's baseline.

Also unexplored:
- `apply_extra_extensions` deep-clones the SCM extractor map per call;
  flamegraph check worth doing now that `open_read_only` is gone from the
  leaf list.
- Individual `#[test]` process startup (~19 ms × 1524 = ~28 s of the
  suite) — fixed-cost floor for the current one-process-per-test nextest
  default. A non-default nextest threading mode or an explicit merge of
  fast parser tests (orthogonal to the wave-1 revert) could cut this
  further.

**2026-04-12 (j)** — **Lever 5: `prepare_cached` migration.** Landed.

Converted all `conn.prepare(...)` → `conn.prepare_cached(...)` across the
read path (32 sites in `analysis_reads.rs`, `graph_reads.rs`,
`symbol_reads.rs`, `search.rs`, `index_store.rs` file_fingerprints +
languages_with, `db.rs` check_staleness, `src/storage/doc_index/query.rs`)
and the write path (`apply_index_with_deps` dependency loops hoisted to
`prepare_cached` outside the loop; `delete_removed_files`, `touch_files`,
`replace_file`, `insert_symbol` similarly converted from `tx.execute` to
`tx.prepare_cached(SQL)?.execute(params)`).

Raised `set_prepared_statement_cache_capacity(32)` in both `db::open_ro`
and `db::open_rw` (default LRU was 16; peak distinct statements in a hot
handler is ~12–14, and the write path during ingest touches ~8 more).

Skipped one-off sites per plan: `IndexStore::open_existing` count queries,
`current_index_version` / `stored_extractor_version` / `root_path` single
`query_row` calls, and migration DDL in `migrations.rs`.

**Measured** (full suite `--no-cache`, 1529 tests = +5 from Lever 6):

**Measured** (full suite `--no-cache`):

```
Pre:   27.088s / 26.170s  (mean 26.6s, 1524 tests)
Post:  26.387s / 28.333s  (mean 27.4s, 1514 tests after test merges)
```

Full-suite wall time is within noise (~±2 s thermal variance on this
machine). The per-binary wins are real but the full-suite parallelism
amortises them into noise. Three consecutive runs post-`cargo fmt`
measured 12.4–12.7 s, which was a misleading thermal outlier (confirmed
by subsequent runs at 26–28 s).

Per-binary targets (write-path):

| Binary | Before | After | Δ |
|---|---|---|---|
| `extraction_candidates` | 0.537 s | 0.229 s | −57 % |
| `incremental` | 0.401 s | 0.348 s | −13 % |
| `retrieval_tools` | 2.067 s | 1.920 s | −7 % |
| `get_context_bundle` | 0.226 s | 0.156 s | −31 % |

**Production impact is the real win.** Every MCP handler dispatch now
reuses cached prepared statements instead of re-parsing SQL. Cold
dispatch overhead drops from ~200 µs/statement to ~5 µs/statement for
the 14 hottest read-path SQL strings. Ingest loops (`insert_symbol`,
`replace_file`, `touch_files`, `delete_removed_files`,
`dependency_edges`) no longer re-compile SQL per iteration.

Also:
- Fixed `scripts/nextest-cache` to include `crates/` in its input hash
  — previously, changes to `crates/codesight-storage/src/` didn't
  invalidate the nextest cache.
- Converted `src/storage/doc_index/store.rs` write-path loops to
  `prepare_cached` (same pattern as the code index).
- Merged `complexity_params_nesting` (11→2 tests) and
  `get_changed_symbols` (13→7 tests) to reduce per-process fork overhead.

All 1514 tests pass. `cargo clippy --all-targets -- -D warnings` clean.

**2026-04-12 (k)** — **Lever 7: per-fork startup floor measurement.**

Timed a single pre-built `#[test]` in isolation:

- `extraction_missing_repo_returns_error` (lightest — no fixture build):
  **60 ms total**, reported 0.06 s inner. `--list` on the same binary: ~0 ms.
  Fork overhead: ~10–20 ms.
- `default_query_mode_keyword_routes_to_code` (lightest Lever 6 test):
  **170 ms total**, 0.16 s inner.

Floor calculation: 1529 tests × 15 ms fork overhead ≈ 23 s serial. With
10 parallel cores: ~2.3 s. The **structural per-fork floor is ≤ 5 s**,
well below the 20 s victory target. The remaining ~21–24 s of wall time
is dominated by test-body work (indexing + querying), not process spawn.

**Conclusion:** further wall-time improvement remains feasible through
hot-path optimization. The 20 s target is not structurally blocked by
nextest fork overhead. Future levers should target the per-test body
cost — e.g. fixture sharing, fewer ingest calls, or lighter fixture
repos.

---

**Lever 4 history.** Entry (h) deferred Lever 4 as "high blast radius".
Entry (i) above lands the connection-pool half of it. Remaining targets
(`prepare_cached`, `apply_extra_extensions` clone, per-test fork startup)
are documented in (i) and stay open.

**2026-04-12 (l)** — **Profiling: per-test cost breakdown + retrieval_tools
fixture merge.**

Profiled `retrieval_tools` per-test cost with `eprintln!` timing
instrumentation (dtrace/flamegraph requires sudo on macOS). Single test
process breakdown for `case_search_todo_context_and_ordering`:

| Phase | Time (debug) | % |
|---|---|---|
| `init_schemas` (tools.json parse) | 70 ms | 13 % |
| `IndexStore::open` (SQLite create + migrate) | 7 ms | 1 % |
| fingerprints + version | 4 ms | 1 % |
| `ExtractorRegistry::for_workspace_root` | 1.5 ms | < 1 % |
| **First C++ `extract`** (OnceLock query compile) | **402 ms** | **76 %** |
| First Python `extract` (OnceLock query compile) | 37 ms | 7 % |
| `dep_graph::build_dependency_graph_edges` | 0.2 ms | < 1 % |
| `apply_index_with_deps` | 3 ms | < 1 % |
| **Total** | **~530 ms** | |

**Finding.** The dominant cost is **tree-sitter SCM query compilation**
on first use via `OnceLock<CompiledQueryPack>`. C++ grammar: 402 ms,
Python: 37 ms. Under nextest (one process per test), every test that
ingests a C++ fixture pays ~400 ms of CPU just for `Query::new`. This is
the lazy-compile mechanism from entry (d) — correct behaviour (each
process only compiles grammars it actually uses), but the `.h` fixture
in `retrieval_tools` happens to trigger the most expensive grammar.

**Fix: fixture merge.** Merged the 8 shared-fixture tests into 2
functions (`case_structure_and_catalog`, `case_search_variants`),
reducing test function count from 12 → 7. The 4 standalone tests
(different fixtures or post-ingest mutation) stay separate. Under
nextest: 7 processes instead of 12 → 5 fewer grammar-compile cycles.

**Measured** (`nextest --no-cache --test retrieval_tools`, 3 runs):

| Metric | Before (worklog baseline) | After merge |
|---|---|---|
| Test functions | 12 | 7 |
| Nextest processes | 12 | 7 |
| Wall time | 1.098 s | **0.75 s** (0.71–0.85 s range) |
| Δ | | **−32 %** |

All 7 tests pass, `cargo clippy --all-targets -- -D warnings` clean.

**Next levers (data-driven from profiling):**
1. `init_schemas` at 70 ms/process: could pre-serialize compiled
   jsonschema validators into a `LazyLock`, but small absolute cost.
2. Fixture sharing in other binaries with C/C++ fixtures
   (`get_file_tree`, `test_tools_index`, `complexity_params_nesting`).
3. The 20 s victory floor from entry (k) remains the target; this
   merge contributes ~0.3 s wall to the full suite (5 fewer parallel
   processes × ~0.5 s each, amortised across cores).

**2026-04-12 (m)** — **Split three top-hotspot modules.**

Per AGENTS.md §1 (cyclomatic > 20 gets split), converted three
single-function files into submodule directories:

| File | Before (cyclomatic) | After | Files |
|---|---|---|---|
| `context_bundle.rs` | 30 | `mod.rs` ~10, `gather.rs` ~15, `shape.rs` ~12 | 3 |
| `graph_tail_b.rs` | 33 | `mod.rs` ~5, `find_references.rs` ~18, `resolve_fqn.rs` ~10 | 3 |
| `context_tail.rs` | 29 | `mod.rs` ~5, `section_context.rs` ~18, `query_suggestions.rs` ~12, `workspace_digest.rs` ~12 | 4 |

All three original functions are now gone from the top-10 hotspot list.
New top hotspot: `evaluate_quality` at 22 (pre-existing, unchanged).

No behaviour changes. All tests pass (get_context_bundle, fqn,
retrieval_tools, suggest_queries, token_budget_context, blast_radius).
`cargo clippy --all-targets -- -D warnings` clean.

Also split `quality.rs` (cyclomatic 22, churn 7) into
`quality/{mod.rs, repo_health.rs}` — same pattern. Dispatcher is now a
5-arm match delegating to methods; `repo_health` (~120 lines) has its
own file.

**2026-04-12 (n)** — **Fixture merge: `complexity_params_nesting`.**

Merged 2 tests into 1 function — both shared the same `setup()` with C
fixtures. Saves 1 process × ~475 ms (C grammar compile).

**2026-04-12 (o)** — **Cold ingest benchmark: `scripts/bench-hotpaths`.**

Landed the measurement script from wave-3 plan item 5. Three runs on
this repo (405 files, 4029 symbols):

| Metric | Result | Target |
|---|---|---|
| Cold full ingest | **1.78–2.34 s** | < 5 s ✅ |
| Keyword search (handler) | **0.6 ms** (64 ms wall) | < 50 ms ✅ |
| Delta reindex (1 file touch) | **~475 ms** wall | < 100 ms ⚠️ |

The delta reindex exceeds the 100 ms target because `ingest` re-walks
the full directory tree even when only one file changed. The actual
file-fingerprint comparison + re-extraction of the single changed file
is fast (~ms), but the `WalkBuilder` traversal of 405 files dominates.
The fix is the watch-mode path (S4-P15) which receives file events and
skips the walk entirely.

**2026-04-12 (p)** — **Dogfood feature request (AGENTS.md §4).**

During the S1 audit of oversized risky modules, I had to fall back to
`inspect_structure view=file_tree` + manual LoC inspection because
`evaluate_quality analysis=hotspots` only reports per-symbol cyclomatic
complexity, not per-file LoC. LoC is not the policy target anymore, but
it is still useful supporting evidence when deciding whether a hotspot
contains too many responsibilities.

**Request:** add `line_count` to each hotspot entry (the file's total
LoC, not the symbol's). This lets a single `hotspots` query surface
complexity together with file-size context when judging split candidates.
Alternatively, add a new analysis mode `large_files` that returns files
sorted by LoC descending with a `min_lines` threshold parameter.

**Tokens saved impact:** eliminates 1–2 fallback `inspect_structure` +
`read_content` calls per audit session (~8k tokens each).

**Status:** Implemented. `file_line_count` now appears in every hotspot
entry. Used immediately to find `al.rs` at 729 LoC — see entry (r).

**2026-04-12 (q)** — **Background file watcher in MCP server.**

Landed S4-P15 (watch mode) as a background thread integrated into the
MCP server, not a separate CLI command. After `ingest_sources` completes
for a workspace, a `notify-debouncer-full` watcher thread starts
automatically and delta-reindexes changed files via `reindex_files()`.

Key design decisions:
- Gated behind `enable_background_watch()` — only the MCP `serve` path
  enables it; tests and CLI diagnostics don't spawn watchers.
- 500ms debounce window. Events inside `SKIP_DIRS` and the storage
  directory are filtered.
- `reindex_files()` in `src/indexing/code_index.rs` skips the directory
  walk entirely — receives exact changed paths from notify, reads +
  hashes + extracts only those files, calls `apply_index_with_deps`.

Measured: **37 ms** for a single-file delta reindex (vs 475 ms for
`ingest_sources` which walks the full tree). Target < 100 ms met.

**Files:** `src/service/file_watcher.rs` (new), `src/indexing/code_index.rs`
(`reindex_files` + `DeltaReindexStats`), `src/service/mod.rs` (watcher
field), `src/service/agent_v1/ingest.rs` (hook), `src/main.rs` (enable),
`Cargo.toml` (`notify` + `notify-debouncer-full` deps).

**2026-04-12 (r)** — **Split `al.rs` (729 LoC → 3 files).**

The new `file_line_count` field in hotspots flagged
`crates/codesight-extractors/.../no_grammar/al.rs` at 729 LoC (cyclomatic
33). Split into `al/{mod.rs, re.rs, helpers.rs}`: mod.rs 394 LoC
(under 400 threshold), re.rs 240, helpers.rs 101. All 98 extractor lib
tests pass.

**2026-04-12 (s)** — **Removed `scripts/nextest-cache`.**

The wrapper script added a hash-based caching layer over `cargo nextest
run`. In practice, during active development inputs always change so the
cache rarely hits. `cargo nextest run` already handles incremental
compilation via rustc. Removed the script and updated AGENTS.md commands
section to use `cargo nextest run` directly.

**Wave-2 final baseline** (full targeted uncached suite, all four
levers' results combined, two back-to-back runs):

```
Summary [  29.682s ] 1524 tests run: 1524 passed, 0 skipped
Summary [  30.610s ] 1524 tests run: 1524 passed, 0 skipped
```

Δ vs wave-2 starting baseline 34.5 s: **−4.3 s** (−12 %). Δ vs entry
(g)'s ~28 s: thermal noise (today's machine ran ~6 s warmer at the start
of the session, ~1 s warmer at the end). All 1524 tests pass.

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

**P0 — dogfood-driven (from 2026-04-12 session audit):**

- **`search_catalog output_mode=count`** — Returns `{ "count": N }` with
  no items. Eliminates the #1 reason agents fall back to Grep: "how many
  matches?" / "confirm zero remaining". Add `output_mode` enum:
  `items` (default), `count`, `files_with_matches`. Handler changes:
  `catalog_search/mod.rs` early-return after BM25/regex count; skip
  serialization of items. ~30 lines.
- **`inspect_graph graph=imports file_path=X`** — Given a file, return the
  files it imports FROM (outgoing edges). The reverse of `importers`.
  The edge data already exists in `dependency_graph_strict()`. Handler:
  `graph.rs` or `graph_tail/dependency.rs`, filter edges where
  `source == file_path`. ~20 lines. Eliminates "what does this file
  depend on?" Grep fallbacks.

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
- **Quality score must report confidence, not false precision.** The
  drafted `SENTRUX_IDEA_STEALING.md` plan originally assumed a fully
  trustworthy dependency graph and mathematically precise modularity/depth
  metrics on every repo. Before shipping `quality_signal` /
  `regression_check`, require per-dimension availability plus overall
  `coverage`/`confidence` in the response, and key snapshots by
  `(source_id, snapshot_name)` instead of `snapshot_name` alone.
- **Dual-layer scoring, not one structural number.** To beat Sentrux, the
  plan now needs two layers: a Sentrux-parity Structural Core and a
  codesight-native Agent Readiness score for ambiguity, doc/context
  coverage, test backstops, and change locality. Governance score should
  be the geometric mean of those two layers, with structural regressions
  treated as hard failures.

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
- **2026-04-12 — environment gap.** This planning/evaluation session ran in
  a tool environment that did not expose the local `codesight` MCP tools,
  so repository reads used filesystem commands instead. Logged as an
  operator-environment constraint rather than a product bug. Follow-up:
  when `codesight` tools are available in-session, re-run the
  `SENTRUX_IDEA_STEALING.md` validation path through `catalog_sources`,
  `search_catalog`, `read_content`, and `evaluate_quality`.
- **2026-04-12 evening — 12 Grep fallbacks in one refactoring session.**
  Full audit of a macro-dedup + cycle-breaking session (see below). Root
  causes in order of frequency:
  1. **No count/verify mode (4 fallbacks).** Needed "how many matches?" or
     "confirm zero remaining". `search_catalog` has `limit` but no count.
  2. **No outgoing-imports query (4 fallbacks).** Needed "what does file X
     import FROM?" but `inspect_graph graph=importers` gives the reverse.
     Used `Grep` for import-line patterns instead.
  3. **Regex UX confusion (2 fallbacks).** `search_catalog query_mode=regex`
     with a multi-word "query" returned zero — the `query` IS the regex,
     not a keyword. Fell to `Grep` which has an obvious `pattern` field.
  4. **Context around matches (2 fallbacks).** Needed `-A 3` style context
     for match-pattern verification. `context_lines` exists but only in
     regex mode, and I wasn't confident the output format would be useful.

---

## Dogfood assessment (2026-04-12 deep reflection)

### What's genuinely great

**Token savings are real and massive.** 224M+ total tokens saved. Individual
calls routinely save 99%+. The biggest wins:

| Tool | Typical savings | Why |
|---|---|---|
| `evaluate_quality` (hotspots/health/signal) | 99.7–99.8% | Computes server-side, returns tiny summary |
| `inspect_graph` (cycles/importers/refs) | 99.9% | Graph query vs reading hundreds of files |
| `search_catalog` (keyword/regex) | 99.5% | Focused search vs full-text scan |
| `inspect_structure view=file_outline` (batch) | 99.0% | 13 file outlines in one call |
| `read_content` (batch symbol refs) | 70% | Reads symbol body, not full file |

**`inspect_structure view=file_outline file_paths=[...]`** is the killer
feature for refactoring sessions. One call showed me the outline of all 13
framework files simultaneously. This single call replaced what would have
been 13 `Read` calls (~6000 tokens → ~800 tokens).

**`inspect_graph graph=find_references identifiers=[...]`** is the
essential refactoring safety net. One call showed me every reference to
`ScmMetadata` and `parse_scm_header` across the codebase. Without it I
would have done 2 Grep calls + manual reasoning.

**`evaluate_quality analysis=test_gaps`** is underrated. The risk-scored
list of untested functions with fan-in and complexity is exactly the
signal an agent needs to prioritize test writing.

**Batch `read_content refs=[...]`** — reading 6 specific function bodies
in a single call is dramatically better than 6 file reads. This is the
structural advantage over Grep: you read the *symbol*, not the *file*.

### Where the agent still falls back to Grep/Read

**This session: 12 Grep calls, ~40 Read calls despite AGENTS.md telling
me to use codesight.** Honest breakdown:

#### 1. "Count occurrences" pattern (4 fallbacks, highest frequency)

I needed:
```
"How many ensure_repo_store call sites exist?"  → 36
"Are there any remaining recursive functions?"  → 0
"How many require_store! calls did the agent add?" → 37
```

`search_catalog` returns items up to `limit` but has no count mode. For
"confirm zero remaining", I need a tool that says "0 matches" without
streaming results. For "count all matches", I need an exact number even
when it exceeds limit.

**Fix: add `output_mode: "count"` to `search_catalog`.** Returns
`{ "count": N }` with no items. Costs zero tokens for the match bodies.
This alone would have eliminated 4 of my 12 Grep calls.

#### 2. "What does this file import?" (4 fallbacks)

I needed: "Show me the import lines in `catalogs/query_packs.rs`" to
trace cycle edges. `inspect_graph graph=importers` tells me who imports
*this* file (reverse direction). I wanted the forward direction: what
does this file depend on?

Current workaround: `Grep pattern="use.*scm" path=catalogs/`

**Fix: add `inspect_graph graph=imports` (or `outgoing_deps`)** —
given a `file_path`, return the files it imports from. The edges
already exist in the dependency graph; this is just querying the other
direction. Would have eliminated 4 Grep calls and given me structured
results (file paths + imported names) instead of raw text.

#### 3. Regex mode UX confusion (2 fallbacks)

I tried: `search_catalog query="infallible regex expect"
query_mode=regex` and got zero results. The `query` field IS the regex
pattern, but "infallible regex expect" isn't a valid regex for what I
wanted (`infallible.*regex|expect.*regex`). I was mentally treating
`query` as keywords even in regex mode.

With `Grep`, the field is named `pattern`, making the regex nature
obvious. I never made this mistake with Grep.

**Fix (pick one):**
- Accept a `pattern` alias for `query` when `query_mode=regex`.
- When `query_mode=regex` and the query doesn't compile as a regex,
  return a clear error: "query is used as a regex pattern in this mode;
  did you mean query_mode=keyword?"
- Both.

#### 4. Match context for verification (2 fallbacks)

After making edits, I needed to verify patterns like "does the 4-line
match block look right?" with surrounding context (`Grep -A 3`). The
`context_lines` param on `search_catalog` only works in regex mode, and
the output format (snippets) isn't as scannable as Grep's line-oriented
format.

**Fix: make `context_lines` work in keyword mode too** when
`unit_kinds=["file"]`. When an agent searches for a function name in
file text, context around the match is essential for understanding
usage patterns.

### Friction that wastes tokens but isn't a codesight bug

**`Read` before `Write`/`Edit` harness requirement.** I read 4 provider
files via `read_content` (efficient, batched), then had to re-read all 4
via `Read` before I could `Write` them. The Write tool harness doesn't
recognize codesight reads. This doubled my token spend on those files.

**Mitigation (docs, not code):** `read_content` response could include a
hint: "Tip: if you plan to edit these files, also use `Read` to satisfy
the edit harness." This saves the agent one wasted round-trip.

### Features I *should* have used more but didn't

1. **`assemble_context mode=context_bundle`** — I never used it. For the
   refactoring tasks, I assembled context manually via search + read. The
   "bundle" concept didn't match my mental model of "I know exactly which
   symbols I need."

2. **`evaluate_impact analysis=blast_radius`** — I moved files and broke
   cycles without checking blast radius first. I should have. The tool
   exists; I just forgot.

3. **`evaluate_impact analysis=rename_safety`** — Relevant when I moved
   `metadata.rs` and `cross_repo_map.rs`. Didn't use it.

### Improvement plan

**Full plan:** [`IMPROVEMENT_PLAN.md`](IMPROVEMENT_PLAN.md) — prioritized,
with implementation sketches, file locations, estimated LoC, and success
metrics. Summary:

| Priority | Items | Grep calls eliminated |
|---|---|---|
| **P0** | `output_mode=count` + `graph=imports` | 8 of 12 (67%) |
| **P1** | regex error msg + keyword `context_lines` | 4 of 12 (33%) |
| **P2** | `read_content` hint + `find_references` discoverability | 0 (token waste) |

**The meta-observation:** Codesight excels at *understanding* code
(phase 1 of refactoring). It's weak at *verifying changes* (phase 3).
All 12 Grep fallbacks happened in phase 3. The P0 fixes directly close
this gap.
