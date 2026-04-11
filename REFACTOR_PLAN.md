# REFACTOR_PLAN.md

Active program for `codesight-mcp`. Living document. Work is organized around four
non-negotiable principles that every patch is measured against:

1. **Code reduction and dedup.** Shrink the product by applying `rust-modularity`,
   `rust-design-patterns`, and the `simplify` skill. One shared helper beats three
   copies; one trait beats a `match` tree; reach for a crate before hand-rolling.
2. **Crate-level modularity.** The workspace split (`codesight-core` →
   `codesight-extractors` / `codesight-storage` → `codesight-mcp`) is the backbone.
   Dependencies point inward; no catch-all `types`/`utils` crate; product logic
   (MCP handlers, CLI, framework detection) stays in the root crate.
3. **Real-use onboarding.** A new operator must be able to install the binary and
   wire it into Claude Code in one page. This is currently the largest gap to
   shipping — there is **no `README.md`** and no documented MCP client config.
4. **Real-use reliability and prior-art parity.** The server must work on
   *any* codebase (not just this repo), must be performant and correct under
   concurrent dispatch, and must reach feature parity with the patterns that
   make `../prior-art-mcp` operationally solid (response envelope, session
   awareness, batch parameters, watch mode, context providers).

Contract constraints that override everything else:

- `agent-v1` and `src/tools.json` are the only public MCP surface. Extensibility
  happens through `view` / `analysis` enums, not parallel tool names.
- No internal backwards compatibility. When behavior changes, tests and
  `tools.json` move in the same patch.
- Structural parsers (tree-sitter, `.scm` queries, injections) before string
  scanning. Heuristics live at the edges, never in shared paths.
- Targeted tests by default; full-suite runs are reserved for explicit merge
  gates (see AGENTS.md `Day-to-day commands`).

---

## Snapshot of the current state (2026-04-10)

| Area | State |
|---|---|
| Workspace crates | 4 (`codesight-core` 182 LoC, `codesight-extractors` 9.0k, `codesight-storage` 2.4k, root `codesight-mcp` 25.4k). Dependency direction clean. |
| Top-level product modules | 12 (`service`, `framework`, `indexing`, `analysis`, `retrieval`, `storage`, `server_support`, `config`, `cli`, `security`, `test_support`, `agent_v1`). |
| MCP tools (agent-v1) | 11 coarse tools (`ingest_sources`, `catalog_sources`, `search_catalog`, `read_content`, `inspect_structure`, `inspect_graph`, `evaluate_impact`, `evaluate_quality`, `assemble_context`, `manage_indexes`, cross-repo). |
| Languages | 30 via `runtime/queries/*/tags.scm` + generic AST fallback. |
| Tests | 87 integration binaries under `tests/`, crate-level unit tests where they pay off. |
| CLI subcommands | `serve` (default), `check`, `init` — all landed. No `watch`, no packaged install script. |
| Quality gates | `cargo clippy --all-targets -- -D warnings` clean; full suite gates on `./scripts/nextest-cache`. |
| **Onboarding docs** | **Missing.** No root `README.md`, no Claude Code MCP config example, no release artifacts. `ARCHITECTURE.md` + `CONFIGURATION.md` exist but assume the reader already has the binary. |
| Response envelope | Handlers shape JSON directly; no `_meta` with `timing_ms`, `tokens_saved`, `truncated`. |
| Session / caching | No per-repo journal, no negative-evidence short-circuit, no search-result cache. |
| Batch params | Only `read_content` accepts arrays; other high-call tools are single-target. |
| Watch mode | Absent. |
| Context providers | Framework detection is monolithic under `src/framework/*`; no `ContextProvider` trait. |

---

## Phase 1 — historical (compressed)

WS1–WS17 (structural refactor) and P1–P9 (production hardening) landed between
2026-03 and 2026-04-10. What they produced, in one paragraph per area:

- **Module shape.** `agent-v1` dispatch split into registry + per-family handlers;
  one structural import subsystem; tree-sitter-first extraction with tiered query
  packs; framework providers decomposed; SCM engine phased
  (`load`/`capture`/`scope`/`extract`/`injections`); residual Rust extractors
  isolated under `extractors/residual/*`; workspace crate split with stable
  inward dependency direction.
- **Error discipline.** `StorageError`, `ExtractError` (`thiserror`) inside
  crates; `ToolError` + `ToolErrorCategory` at the MCP boundary; `CodeService::dispatch`
  downcasts leaked `anyhow::Error` into categorized responses.
- **SQLite.** Single `open_rw`/`open_ro` helper applies WAL, `busy_timeout`, mmap,
  cache, foreign keys. Forward-only `user_version` migrations for code and doc
  indexes, covered by unit tests.
- **Tracing.** `tracing` + `tracing-subscriber` on stderr only, with
  `EnvFilter`, `--log-format`, `CODESIGHT_LOG_FORMAT`. No bare `eprintln!` in
  libraries.
- **MCP arg validation.** `src/service/agent_v1/validate.rs` compiles every
  `tools.json` `inputSchema` into a `OnceLock` map; handlers consume
  `ValidatedArgs<T>`; typed-deser failures become `ToolError::InvalidArgument`.
- **Config layering.** `AppConfig::load` resolves env → global TOML → compiled
  default; per-workspace scoping via `.codesight.toml`; every `CODESIGHT_*` name
  is a const in `src/config/env.rs`.
- **`check` + strict freshness.** `codesight-mcp check [--workspace PATH]`
  validates storage, migrations, query packs, workspace config.
  `ReindexStateManager` is owned by `CodeService`; `freshness.rs` gates
  read tools in strict mode and annotates `STALE_DATA_WARNING` in relaxed mode.
- **Operational resilience (P10, done 2026-04-10).** WAL passive checkpoint on
  `ReindexGuard::drop` and on shutdown; `current_thread` tokio runtime on
  `src/mcp.rs`; dispatch timeout (`CODESIGHT_HANDLER_TIMEOUT_SECS`, default 60s)
  via `src/service/dispatch_timeout.rs`; `SIGINT`/`SIGTERM` observed through
  `tokio::signal`; concurrent dispatch under a bounded semaphore
  (`CODESIGHT_DISPATCH_CONCURRENCY`, default 4) with a serialized stdout mutex;
  per-repo ingest gate (`ReindexStateManager::try_mark_reindex_start` +
  `ToolError::Transient{REINDEX_BUSY}`); ingest write paths wrap delete-then-insert
  in a single SQLite transaction so orphan rows cannot survive a kill.
- **Init subcommand (P16, done).** `codesight-mcp init [--global|--workspace]`
  writes a commented config template.
- **Benches (P9).** Criterion benches for SCM extract and FTS5 search;
  flamegraph passes and baselines still deferred (reclaimed under S4-perf below).

This means module shape, error discipline, SQLite config, tracing, validation,
config layering, and core operational resilience are **not** the gap anymore.
The gap is onboarding, response-envelope parity, session awareness, and
simplification of the modules that outgrew WS17.

---

## Phase 2 — workstream status (compressed)

| ID | Title | Status | Folded into |
|---|---|---|---|
| P10 | Operational resilience | **done** 2026-04-10 | — |
| P11 | Response envelope & metadata parity | planned | **S4** |
| P12 | Session awareness (journal + negative evidence + cache) | planned | **S4** |
| P13 | Batch array parameters on high-call tools | planned | **S4** |
| P14 | Simplification sweep (splits, panics, env consolidation, dead code) | planned | **S1** |
| P15 | Watch mode + incremental reindex | planned | **S4** |
| P16 | `init` subcommand + config template | **done** | — |
| P17 | Context providers (plugin-style enrichment) | planned | **S4** |
| P18 | Language & tool coverage audit (`docs/COVERAGE_DELTA.md`) | planned | **S4** |
| P19 | Benchmark corpus + token-accurate counter | planned | **S4** |
| P20 | Security hardening follow-through (symlink escape, file caps) | planned | **S4** |
| P21 | Finish test layering (carries P8 forward) | planned | **S1** |

Details for each P-number are preserved in git history
(`git log --follow REFACTOR_PLAN.md`) and are not duplicated here. The active
work is now expressed as four principle-aligned streams.

---

## Phase 3 — four principle-aligned workstreams

Each workstream states the principle it serves, the concrete items it owns, the
Rust skills / sub-agents that apply, and what "done" means.

### S1 — Simplification & dedup (principle 1)

**Goal.** Shrink the product crate; remove hand-rolled utilities that a crate
already solves; flatten modules that grew past 400 LoC; collapse dead code.

**Skills.** `simplify` (primary driver), `rust-modularity` (size-driven splits),
`rust-design-patterns` (traits, `LazyLock<Regex>` tables, `Cow` where profiled),
`rust-perf-analyzer` (don't let a cleanup regress a hot path).

**Items (concrete, each its own patch).**

1. **Split oversized modules** (all absorbed from P14):
   - `src/storage/doc_index/mod.rs` (509) → extract `query.rs`, `store.rs`.
   - `src/service/agent_v1/impact.rs` (494) → `impact/{blast_radius,preview,rename_safety,changed_units,diff}.rs`.
   - `src/service/agent_v1/graph_tail.rs` (488) → tail-expansion submodule.
   - `src/service/agent_v1/catalog_search.rs` (411) → `catalog_search/{bm25,semantic,fuzzy,regex}.rs`.
   - `src/indexing/packages/walker.rs` (475) → move the embedded integration
     tests to `tests/packages_walker.rs`, keep the walker slim.
2. **Residual panics from P1.**
   - `src/service/agent_v1/validate.rs:83` — replace `panic!("tools.json must be a JSON array")` with a startup error routed through `main.rs`.
   - `crates/codesight-extractors/src/extractors/engine/scm/injections.rs:203–204` — return an `ExtractError` from query-pack compile.
3. **`env::var` cleanup.** `src/retrieval/semantic.rs`, `src/retrieval/summarizer.rs`,
   `src/server_support/path_map.rs` must use consts in `src/config/env.rs`. Add
   `CODESIGHT_EMBED_MODEL`, `CODESIGHT_SUMMARY_MODEL`, existing path-map name.
4. **Dead / commented code.** Either restore each commented case in
   `tests/tools_remaining.rs` as a real test, or delete the file.
5. **Infallible-regex consolidation.** Seven `.expect("infallible: … regex")`
   in `src/framework/laravel/*` → single `static LazyLock<Regex>` table per
   framework module.
6. **Dedup scan** (ongoing). Before adding a helper, run the `simplify` skill
   on the target module. Before hand-rolling, check `crates.io` for a
   well-maintained option already in the dep tree.
7. **Finish test layering (was P21/P8).** Migrate
   `tests/get_doc_coverage.rs`, `tests/doc_broken_links.rs`,
   `tests/doc_parser*.rs`, `tests/retrieval_tools.rs`,
   `tests/get_context_bundle.rs` to `src/test_support/indexed.rs`. Split
   nextest buckets into `unit | slice | contract | heavy`. Uncached focused
   loop ≤ 30s, uncached full suite ≤ 3 min.

**Done when.** No production file over 400 LoC without a note in this plan.
No `panic!` / `unreachable!` outside tests or documented infallible init.
No `std::env::var("CODESIGHT_*")` outside `src/config/env.rs`.
`tests/tools_remaining.rs` gone or fully restored. Uncached focused loop ≤ 30s.

### S2 — Crate-level modularity (principle 2)

**Goal.** Keep the crate graph clean, small, and inward-pointing. No drift
back into the product crate; no catch-all shared crate; no new crate until a
boundary is already stable in practice.

**Skills.** `rust-modularity` (architectural pattern selection,
dependency-direction enforcement), `rust-design-patterns` (trait-based
boundaries).

**Standing rules.**

- The crate graph is frozen at four members until shared fact types stabilize:
  `codesight-core`, `codesight-extractors`, `codesight-storage`, root
  `codesight-mcp`. No speculative `codesight-indexing` /
  `codesight-analysis` splits.
- Anything that product layers import belongs in a named crate module, not a
  utility bag. No re-exports that hide the source crate.
- `codesight-core` stays skeletal: symbol types, `FactQuality` taxonomy, and
  the soon-to-land `ContextProvider` trait (S4-P17). Nothing that depends on
  `tokio`, `rusqlite`, or `tree_sitter` belongs in core.
- Product-only logic (MCP dispatch, framework detection, CLI subcommands,
  retrieval scoring) stays in `codesight-mcp`. Do not smuggle product knobs
  into `-extractors` or `-storage` just because they are "shared crates".

**Items.**

1. **Dependency audit** (one patch). Run `cargo depgraph --workspace-only`
   (or a manual `cargo metadata` pass) and record the edge list in
   `ARCHITECTURE.md`. Fail the audit if anything in `-core` depends on
   `-storage`/`-extractors`, or if the two middle crates depend on each other.
2. **`lib.rs` shrink.** Every crate's `lib.rs` must be pure re-export and
   module declaration; no free functions, no `pub use` that hides a submodule
   boundary. Start with `codesight-mcp/src/lib.rs` (currently 26 LoC — verify
   it is still skeletal after S1 splits).
3. **Provider trait placement** (blocks S4-P17). The `ContextProvider` trait
   lives in `crates/codesight-core/src/providers.rs`; concrete providers live
   in `src/indexing/providers/*` in the product crate. Do not open a new
   crate for providers.
4. **Freeze guard.** Any PR that adds a new `crates/` member or a new
   `pub use` across a crate boundary must update this workstream with a
   justification paragraph before merging.

**Done when.** Dependency graph is recorded in `ARCHITECTURE.md`, every crate
`lib.rs` is skeletal, the `ContextProvider` trait lands in `-core` when S4-P17
ships, and no PR has added a crate member without a plan update.

### S3 — Real-use onboarding (principle 3) — **CRITICAL PATH**

**Goal.** An operator who has never seen this repo can install `codesight-mcp`,
point Claude Code at it, index a workspace, and observe token savings in **under
five minutes**. This is the single largest gap to actually using this MCP in
this project to save tokens.

**Skills.** `rust-cli-building` (install story, first-run UX, error clarity),
`update-config` (document the Claude Code settings.json / `.mcp.json`
wiring), `simplify` (trim the resulting docs to the minimum useful).

**Items.**

1. **Root `README.md`** (new file). Single page. Must contain, in order:
   - One-paragraph pitch: what codesight-mcp does, what problem it solves,
     explicit parity claim with `prior-art-mcp`.
   - "Install" section: `cargo install --path .` and a pre-built binary
     path for now; pre-built release artifacts tracked as a follow-up.
   - "Wire into Claude Code" section: exact JSON snippet for both
     `claude mcp add codesight /absolute/path/to/codesight-mcp` (CLI) and
     a manual `.mcp.json` / `settings.json` entry. Include the `env` block
     for `CODESIGHT_INDEX_DIR` and the `CODESIGHT_LOG_FORMAT` hint.
   - "First run" section: `codesight-mcp init --workspace`, then
     `codesight-mcp check --workspace .`, then the first `ingest_sources`
     call through Claude Code, then a `search_catalog` example.
   - "Where state lives" link to `CONFIGURATION.md`.
   - "When to reach for it vs Read/Grep" paragraph that sets the agent's
     expectation — this is the single most effective way to get the token
     savings to actually land.
2. **Claude Code usage hint (`CLAUDE.md` snippet).** A copy-pasteable block
   the operator can drop into `~/.claude/CLAUDE.md` (or a project
   `CLAUDE.md`) that instructs the agent to prefer `codesight-mcp` tools
   over `Read`/`Grep`/`Glob` for code exploration once the index exists.
   prior-art proves this is load-bearing: without the instruction, agents
   do not use the tool. Keep the snippet under 10 lines.
3. **`codesight-mcp init --claude-code`** (new `init` mode). Writes a minimal
   `.mcp.json` entry in the current workspace **or** prints the exact CLI
   command to register globally. Refuses to overwrite without `--force`.
   This is the automation layer that removes the "copy this JSON by hand"
   step from the README.
4. **Self-host dogfood script.** `scripts/selfhost-index` — a shell script
   that: builds release, runs `init --workspace`, runs
   `ingest_sources local_path=.`, runs a canned `search_catalog` query,
   prints the `_meta.tokens_saved` field once S4-envelope lands. Until then
   it prints raw `timing_ms`. This is the proof that the server works on
   *this* codebase (principle 4) as well as the onboarding smoke test.
5. **Install-failure triage.** Any error path the first-run script hits
   (missing tree-sitter grammar, unreadable `$XDG_CONFIG_HOME`, stale lock
   file) must surface a one-line actionable message from `main.rs` or
   `check.rs`, not a panic or a raw `anyhow` chain. Drive this with the
   `rust-cli-building` skill.
6. **README link discipline.** `ARCHITECTURE.md`, `CONFIGURATION.md`, and
   this file must be linked from the README's "Further reading" section.
   The README is the entry point; everything else is the manual.

**Done when.** A fresh clone of this repo can be turned into a working
Claude Code MCP integration in five minutes following *only* the README,
on a machine with `cargo` and Claude Code already installed. The
`scripts/selfhost-index` smoke test passes. The project's own
`CLAUDE.md` points at `codesight-mcp` so that subsequent sessions in this
repo save tokens through it.

### S4 — Real-use reliability & prior-art parity (principle 4)

**Goal.** The server must be correct, performant, and universally useful on any
codebase — not just fixture repos. Feature set should match the prior-art
patterns that matter in practice: response envelope with telemetry, session
awareness, batch parameters, watch mode, context providers, coverage audit,
benchmark corpus, security hardening follow-through.

**Skills.** `rust-design-patterns` (one envelope type, middleware dispatch,
provider trait), `rust-perf-analyzer` and `rust-perf-bench` (hot-path budget
enforcement, end-to-end benchmarks), `rust-cli-building` (watch subcommand,
operator flags).

**Items (tracked by old P-number so history is searchable).**

- **P11 — Response envelope.** New `src/service/response_envelope.rs`:
  `ToolEnvelope<T> = { data, meta, warnings }` with `timing_ms`, `truncated`,
  `tokens_saved`, `total_tokens_saved`, `estimate_method`, `powered_by`,
  optional `next_cursor`, `freshness`, and `error_category` on failure.
  New `src/retrieval/token_tracker.rs` persisted to
  `$storage_dir/_savings.json` (atomic append + file lock). Every handler
  builds through a `RespondWith` helper; no handler shapes outer JSON.
  Snapshot test pins the envelope shape per tool family. Surfaces in
  `codesight-mcp check --savings`.
- **P12 — Session awareness.** `src/server_support/session/{mod,journal,cache,negative}.rs`.
  Append-only `SessionJournal` under `$storage_dir/sessions/<workspace_hash>.jsonl`;
  `SearchResultCache` LRU (SQLite-backed new `search_cache` table, migration
  bump); `NegativeEvidence` short-circuit with `meta.cached = true`.
  Dispatch hooks around `agent_v1::dispatch`. Expose as
  `inspect_structure view=session`. Hash+lookup budget ≤ 50 µs.
- **P13 — Batch array parameters.** Widen `inspect_structure view=file_outline`
  to `paths: string[]`, `inspect_graph kinds={references,importers}` to
  `symbols: string[]`, `evaluate_quality` to `targets: string[]`. Response
  shape `{ results: [{ key, data|error }, ...] }` in batch mode. No new
  tool names. Per-item errors must not fail the whole call.
- **P15 — Watch mode.** `src/cli/watch.rs` wired as a clap subcommand:
  `codesight-mcp watch [--workspace PATH] [--debounce 500ms]`. Use
  `notify-debouncer-full`. Reuse / introduce
  `src/indexing/code_index.rs::index_workspace_delta`. Respect trust policy
  and language filters. Single-file edit reindexes in < 100 ms wall-clock
  after debounce. SIGINT exits cleanly via P10 scaffolding.
- **P17 — Context providers.** `codesight-core::providers::ContextProvider`
  trait (see S2 for placement). `src/indexing/providers/{dbt,git_blame,framework_routes}.rs`
  concrete implementations. Providers run after extraction, before storage
  commit. Enrichment rides `FactQuality` as `baseline_structural` unless
  the provider can upgrade it. `src/framework/*` replug into the new trait
  — framework code no longer imports from `service` layers.
- **P18 — Coverage audit.** One-shot `docs/COVERAGE_DELTA.md`: prior-art
  tool/language × codesight equivalent × parity/gap/intentional. Every
  intentional gap documented in the relevant tool's `description`.
  Language backlog for gaps where no curated pack exists.
- **P19 — Benchmark corpus.** `benchmarks/` tree with small checked-in
  fixture repos (rust, typescript, python) and `benchmarks/tasks.json`
  canonical queries. `cargo bench -p codesight-mcp --bench end_to_end`
  runs dispatch → handler → envelope per query, records `timing_ms`,
  `result_count`, `truncated`. Optional `tiktoken-rs` feature for
  token-accurate counts (replaces byte-approx from P11). Baseline table
  in `ARCHITECTURE.md`. Stable within 5% across three runs.
- **P20 — Security hardening.** `src/security/path_validation.rs` gains
  a symlink-escape check (canonicalize target, re-assert
  `commonpath(workspace, target)`). `max_files_per_workspace`
  (default 100k) enforced at walk time with a structured warning.
  `src/security/discovery.rs` inlines the full secret-file exclusion list.

**Done when.** Every handler response carries a `_meta` block with
`timing_ms` and `tokens_saved`. A restarted session short-circuits repeat
searches via cache and negative evidence. At least three high-call tools
accept batch parameters. `codesight-mcp watch` reindexes a single edited
file in < 100 ms after debounce on a fixture repo. A dbt fixture surfaces
`schema.yml` descriptions through `inspect_structure`. The benchmark
table is checked into `ARCHITECTURE.md` with reproducible numbers. A
symlink-escape fixture is refused with a specific error.

---

## Critical-path order (unblocks "use this MCP in this repo to save tokens")

The order below optimises for principle 3 first (so the user can actually use
the tool in this repo today) and then layers reliability.

1. **S1 simplification sweep (items 1–2)** — stop the oversized modules from
   bleeding into every new patch. Low risk, unblocks every later touch.
2. **S3 items 1–4 (README, CLAUDE.md snippet, `init --claude-code`, selfhost
   script)** — this is the blocker for using codesight-mcp in this repo to
   save tokens. Nothing else in the plan matters if the operator can't wire
   it up. Must land before any further feature work.
3. **S1 items 3–6 + S2 item 1 (env cleanup, dead code, dep audit)** — bite-size
   cleanups that stabilize the crate boundaries before S4 lands the envelope.
4. **S4-P11 (response envelope)** — every subsequent handler edit in S4
   assumes this exists. Lands once the dispatch seam is calm after S1.
5. **S4-P13 (batch params)** — depends on P11 envelope for per-item results.
6. **S4-P12 (session awareness)** — depends on the envelope being stable.
7. **S4-P15 (watch mode)** — depends on P10 signal scaffolding (already done).
8. **S4-P17 (context providers)** — depends on S2 item 3 (trait placement).
9. **S1 item 7 (finish test layering)** — can run in parallel with any of
   the above if a second agent is available.
10. **S4-P20 (security hardening)** — isolated, anywhere.
11. **S4-P18 (coverage delta)** — after P13/P17 so the rows are accurate.
12. **S4-P19 (benchmarks)** — last. Bench only what is stable.

---

## Intentionally out of scope

- CI workflows. Not needed for this product right now.
- Vue / Razor / Blade grammar ports — blocked on upstream `tree-sitter` 0.26
  grammars. Residual facades document the exact version that would retire them.
- Semantic search / embeddings rework. Architecture is sound; not a blocker.
- New crate splits beyond the four already in place. Phase-2+ candidates only.
- prior-art's `plan_turn` opening-move router and `tweakcc` system-prompt
  rewrites. Product experiments, not production infrastructure.
- Parity on raw tool name count. prior-art's 50+ surface is not a goal;
  `agent-v1`'s ~11 coarse tools + enum views is the contract.

## Acceptable heuristics

Not every non-query rule is wrong. These remain acceptable until a better
structural source exists:

- path-to-route mapping for frameworks whose semantics are explicitly
  file-system-based (Next.js, Nuxt)
- extension and compound-extension routing when the file extension *is* the
  language contract
- repo-root and config-file detection (`next.config.ts`, `composer.json`, etc.)
- doc format detection by extension (`.md`, `.rst`, `.ipynb`, …)
- filename / extension policy in `security` when the policy itself is
  filename-based (binary extensions, secret suffixes)
- assembly dialect selection by extension or explicit assembler directives;
  broad content sniffing stays a fallback

Everything else trends toward structural extraction: imports, symbol
classification, entrypoint discovery, framework metadata, cross-repo package
references.

## Query-first extractor policy

Adding a new language or embedded-language variant should usually mean adding
query data or a profile, not editing Rust core.

| Tier | Use when | Mechanism | Rust ownership |
|---|---|---|---|
| Tier 1: curated query pack | grammar exists and we care about quality | `tags.scm` + optional `injections.scm` + `pack.toml` | shared engine only |
| Tier 2: generic AST fallback | grammar exists, no curated pack yet | shared language-agnostic walker using node-kind + `name` field conventions | one shared fallback, no per-language extractor |
| Tier 3: container / injection runner | file embeds one or more child languages | grammar-backed injections or declarative segment extraction delegating to child packs | shared container helpers only |
| Tier 4: residual override | no stable grammar, or truly multi-dialect / host-specific | small named Rust extractor under `extractors/residual/*` with explicit exit criteria | explicitly budgeted and temporary |

Rules:
- query packs are the default path whenever a grammar exists
- a new grammar gets Tier 2 before a bespoke Rust extractor
- container languages delegate inner-language extraction to existing packs
- path/product aliases (`openapi`, `ansible`) are profiles over shared packs,
  not distinct extractors
- residual overrides require stating exactly why Tier 1–3 are insufficient

## Target module shape

```text
src/
  core/                      (moved to crates/codesight-core — ContextProvider trait lives here)
  service/
    agent_v1/
      impact/                (S1)
      catalog_search/        (S1)
      ...
    response_envelope.rs     (S4-P11)
    dispatch_timeout.rs      (done, P10)
  indexing/
    code_index/
    imports/
    packages/
    providers/               (S4-P17)
  extractors/                (moved to crates/codesight-extractors)
  analysis/
    graph/
    quality/
    cross_repo/
  framework/                 (plug into providers — S4-P17)
  retrieval/
    search/
    summaries/
    sections/
    token_tracker.rs         (S4-P11)
  storage/                   (moved to crates/codesight-storage)
    code_index/
    doc_index/
      query.rs               (S1)
      store.rs               (S1)
    cache/
  server_support/
    session/                 (S4-P12)
  config/
  cli/
    check.rs                 (done)
    init.rs                  (done; extend with --claude-code in S3)
    watch.rs                 (S4-P15)
```

Rules:
- `service` is the only MCP boundary and response-shaping layer. It does not
  crawl repos or own indexing algorithms.
- `indexing` owns crawl/delta/import/package/provider fact production. It does
  not know about public tool responses.
- `extractors` own symbol extraction and mixed-language helpers, not persistence
  or API DTOs.
- `analysis` consumes indexed facts. It does not scan raw source in handlers.
- `retrieval` owns ranking/summarization/section logic and the token tracker.
- `storage` owns persistence and I/O, not business rules.
- `core` stays small and focused on truly shared primitives (including the
  `ContextProvider` trait).

## Guardrails for every refactor patch

- Do not delete tests to make refactors easier.
- When moving logic, move or add tests in the same patch.
- Prefer replacing duplicate helpers with one shared implementation. Before
  hand-rolling, check the dep tree or crates.io for an existing solution.
- Avoid adding new root-level modules unless they clearly define a new domain.
- Keep `lib.rs` skeletal.
- Do not add a new Rust extractor until query-pack, generic-AST fallback, and
  container/profile options have been considered and rejected explicitly.
- If a new heuristic is unavoidable, note it here and in code with the reason
  the structural path is insufficient.
- **Run targeted tests by default.** Do not trigger the full suite unless the
  user explicitly asks for it or you are preparing an explicit merge-gate
  check. See AGENTS.md `Day-to-day commands`.
- Never silence clippy with a broad `#[allow(...)]`. Fix the underlying issue
  or scope the allow to a single line with a reason.
- No new tool names on the `agent-v1` surface without an explicit plan entry.
  Widen existing `view` / `analysis` enums instead.
- Every handler response goes through the S4-P11 envelope once it lands. No
  hand-shaped JSON in handlers after that point.

## Decision summary for agents

When implementing the remaining work, assume the following unless a later plan
update says otherwise:

- repo-local query packs, if supported, live under `.codesight/queries/`
- confidence is coarse and typed (`FactQuality`), not numeric
- `injections.scm` is the long-term container mechanism; no Rust DSL
- `indexing/packages` owns package facts; `analysis/cross_repo` only consumes
  them
- only genuinely reusable primitives belong in `core`
- crate splits past `core` + `extractors` + `storage` stay frozen until test
  layering proves out
- typed errors inside crates (`thiserror`), `anyhow::Error` at boundaries
- all SQLite connections go through the WAL-configured helper
- all logs go through `tracing`; no bare `eprintln!` / `println!` in libraries
- MCP args are coerced and schema-validated before any handler runs
- every tool response carries the S4-P11 metadata envelope (once it lands)
- every handler runs under a dispatch timeout (done, P10)
