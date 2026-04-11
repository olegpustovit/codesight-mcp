# AGENTS.md

This repository is the `codesight-mcp` Rust crate at repo root.

## Layout

| Path | Purpose |
|------|---------|
| [`Cargo.toml`](Cargo.toml) | Product crate manifest. |
| [`agentpack.toml`](agentpack.toml) | Agentpack manifest pointing at this crate. |
| [`src/`](src/) | Runtime, indexing, retrieval, MCP handlers, operator CLI. |
| [`src/cli/`](src/cli/) | Operator subcommands that do not speak MCP (e.g. `check`). |
| [`src/config/`](src/config/) | Layered `AppConfig`: env vars, global TOML, defaults, trust policy. |
| [`src/service/agent_v1/validate.rs`](src/service/agent_v1/validate.rs) | Coerce-then-validate MCP args against `tools.json`. |
| [`crates/codesight-core`](crates/codesight-core) | Shared primitives (symbols, FactQuality). |
| [`crates/codesight-extractors`](crates/codesight-extractors) | SCM engine + curated query packs. |
| [`crates/codesight-storage`](crates/codesight-storage) | WAL-configured SQLite + forward-only migrations (code index *and* doc index DDL via `storage::doc_schema`). |
| [`runtime/queries/`](runtime/queries/) | Tree-sitter `.scm` symbol queries. |
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | Crate split, data flow, FactQuality taxonomy. |
| [`CONFIGURATION.md`](CONFIGURATION.md) | Env vars, precedence chain, `check` subcommand. |
| Workspace config | Optional repo-root `.codesight.toml` (TOML) for per-workspace adaptive languages, extra extensions, query-pack roots. |
| [`tests/`](tests/) | Integration tests. Checked-in fixtures: [`tests/fixtures/doc_reference/`](tests/fixtures/doc_reference/), [`tests/fixtures/code_reference/`](tests/fixtures/code_reference/), [`tests/fixtures/lang_samples/`](tests/fixtures/lang_samples/). |

## Dogfooding (non-negotiable)

**Drive test-drive and self-fix work through `codesight-mcp` itself.** We are
building a retrieval engine — if we don't use it on ourselves we won't notice
when it's broken. Exploration must go through the MCP tools before reaching
for default filesystem tools.

### Tool replacement map

| Default tool | Use instead | Notes |
|--------------|-------------|-------|
| `Grep` / `rg` for symbols, imports, references | `mcp__codesight__search_catalog` (`query_mode=keyword`) | symbol-aware scoring, cross-file |
| `Grep` / `rg` for raw regex over file text | `mcp__codesight__search_catalog` (`query_mode=regex`) | routes to file-text search automatically |
| `Glob` / `find` for tree walks | `mcp__codesight__inspect_structure` (`view=file_tree` or `repo_outline`) | respects index, skips `.codesight-index/**` |
| `Read` of a known file/range | `mcp__codesight__read_content` (`kind=file` + `start_line`/`end_line`) | carries `language`, clamp warnings, `_meta` token savings |
| `Read` of a symbol body | `mcp__codesight__read_content` (`kind=symbol`) | hashes, context_lines, drift detection |
| manual import chasing | `mcp__codesight__inspect_graph` (`graph=importers` / `references` / `calls`) | real graph edges, cross-repo-aware |
| manual impact review before a rename/refactor | `mcp__codesight__evaluate_impact` (`analysis=blast_radius` / `rename_safety` / `impact_preview`) | structured findings |
| ad-hoc hotspot / dead-code spotting | `mcp__codesight__evaluate_quality` (`analysis=repo_health` / `dead_code` / `hotspots` / `doc_coverage`) | operator-facing metric schema |
| digest/packing work for a ticket | `mcp__codesight__assemble_context` (`mode=bundle` / `context_bundle` / `ranked` / `query_suggestions`) | token-budgeted output |
| checking whether an index is fresh | `mcp__codesight__manage_indexes` (`action=status`) → `mcp__codesight__ingest_sources` | never assume staleness by hand |

### Fallback rules

- Default filesystem tools (`Grep`, `Glob`, `Read`, `Edit`, `Write`, `Bash`)
  remain the right choice for **writing** changes, running tests, and
  editing files — you can't commit through an MCP tool.
- Fall back to `Grep` / `Read` for retrieval **only when**:
  1. codesight can't answer the question (e.g. the file isn't indexed, the
     grammar is missing, the tool itself is the thing under repair); or
  2. you've already tried the codesight path once and captured the gap.
- Every retrieval-path fallback must be logged as a test-drive issue in
  `CODESIGHT_TESTDRIVE_ISSUES_v*.md` — that's how gaps convert into
  product fixes.
- Issues found during a test-drive land in the same file and get fixed in
  the same working session wherever practical — don't let it rot into a
  backlog.

## Tests (non-negotiable)

- **Never leave a broken test or failing lint in place.** If you discover a pre-existing failing test, clippy error, or broken implementation while working on something else, **fix it in the same session** — even if it's not part of your original task. Do not:
  - Skip it with a comment like "pre-existing, not my regression."
  - Mark it `#[ignore]` to get CI green.
  - Quietly scope your runs to avoid hitting it.
  - Defer it to "a later cleanup pass."
  Root-cause it and restore the contract the test was enforcing, or (if the test is genuinely stale) update it to the new contract in the same patch. Noticing a broken thing and walking past it is how this codebase rots.
- **Do not lose tests.** Refactors and simplifications must keep the suite meaningful: no deleting tests to silence failures, no broad `#[ignore]` to hide breakage, no shrinking fixtures or assertions without an intentional contract change.
- When behavior changes on purpose, update tests and docs together so they still encode the product contract.
- New work adds or extends tests when it adds behavior; “cleanup” is not an excuse to drop coverage.
- Treat test wall-clock time as a product constraint. Default new coverage to the smallest layer that proves the behavior; end-to-end `agent-v1` and full-ingest tests should guard contracts, not absorb every internal case.
- **Targeted tests by default.** Run the smallest scope that proves your change; do not run the full suite unless the user asks for it or you are preparing an explicit merge/release gate. See `Day-to-day commands` below.

## Strategic focus (refactor and direction)

Every patch is measured against four non-negotiable principles. These are the
load-bearing axes of [`REFACTOR_PLAN.md`](REFACTOR_PLAN.md); keep both in sync.

1. **Code reduction and dedup.** Shrink the product. Reach for `rust-modularity`,
   `rust-design-patterns`, and the `simplify` skill before hand-rolling utilities
   or growing central `match` trees. One shared helper beats three copies; a
   well-maintained crate beats a bespoke implementation; a trait with a small
   registry beats a growing `match`. Oversized modules (> 400 LoC) are split,
   not paved over.
2. **Crate-level modularity.** The workspace split `codesight-core` →
   (`codesight-extractors` | `codesight-storage`) → root `codesight-mcp` is the
   backbone. Dependencies point inward, never sideways. No catch-all
   `types`/`utils` crate. Product-only logic (MCP handlers, framework detection,
   CLI subcommands) stays in the root crate; don't smuggle it into the middle
   crates. The crate graph is frozen at four members until shared fact types
   stabilize.
3. **Real-use onboarding.** An operator who has never seen this repo must be
   able to install `codesight-mcp`, wire it into Claude Code, and start saving
   tokens in five minutes, following only the repo root documentation. This is
   currently the largest gap to shipping — treat `README.md`, the
   Claude Code MCP snippet, and the self-host smoke script as first-class
   deliverables. See workstream **S3** in [`REFACTOR_PLAN.md`](REFACTOR_PLAN.md).
4. **Real-use reliability and parity with `../prior-art-mcp`.** The server
   must work on *any* codebase (not just fixture repos or this one), must be
   performant and correct under concurrent dispatch, and must reach feature
   parity with the prior-art patterns that matter in practice: rich `_meta`
   response envelope with `timing_ms` / `tokens_saved` / `truncated`, session
   awareness (journal + negative evidence + search cache), batch array
   parameters on high-call tools, watch-mode incremental reindex, pluggable
   context providers, security hardening follow-through. See workstream
   **S4** in [`REFACTOR_PLAN.md`](REFACTOR_PLAN.md).

Contract-level constraints that override everything else:

- The active structural refactor program is tracked in
  [`REFACTOR_PLAN.md`](REFACTOR_PLAN.md). Treat it as the current working
  document for modularity, heuristic removal, and sequencing. If a change
  materially affects module boundaries, fallback heuristics, or sequencing,
  update it in the same patch.
- Prefer **general mechanisms** (clear extension points, composable pipelines,
  structural parsing) over **edge-case coupling** in core paths. Special cases
  belong at the edges (language-specific extractors, query packs), not
  scattered through shared logic.
- Keep hot paths **allocation-conscious and predictable**; avoid accidental
  O(n²) or repeated I/O when designing abstractions.
- Keep the test loop practical. Prefer layered unit/slice tests for domain
  behavior and reserve full service/MCP indexing tests for public-contract
  coverage.
- **`agent-v1` and [`src/tools.json`](src/tools.json) remain the contract:**
  extensibility must not fragment the public MCP surface. Widen `view` /
  `analysis` enums; never add parallel tool names.

## Working rules

1. Keep the public MCP surface coherent. The live contract is defined by [`src/tools.json`](src/tools.json) and enforced by the Rust tests.
2. Prefer structural parsers and reusable abstractions over ad-hoc glue.
3. Do not add duplicate tool names, hidden routes, or parallel public MCP surfaces. `agent-v1` is the only supported API.
4. Keep tests self-owned inside this crate. New fixtures belong under [`tests/fixtures/`](tests/fixtures/), not in external oracle trees.
5. **No backwards compatibility.** Do not add shims, aliases, feature flags, or dual code paths to keep obsolete clients working. When behavior changes, update [`src/tools.json`](src/tools.json) and tests to the new contract—never paper over breaks in core or the MCP surface.

## Modularity guidance

- **Prefer crate-focused architecture.** New code that is legitimately shared
  belongs in a workspace crate (`codesight-core`, `codesight-extractors`,
  `codesight-storage`). Product-only logic (MCP handlers, framework detection,
  CLI subcommands) stays in the root `codesight-mcp` crate. Don't mix
  extractor internals or storage plumbing into the product layer.
- Organize by domain at the top level: extractor engine, framework detection, graph/impact analysis, retrieval/service wiring, operator CLI.
- Keep `lib.rs` skeletal and push logic into named modules with **clear dependency direction** (core types inward, adapters at the boundary).
- When a behavior family is open-ended, prefer **traits and small registries** over growing central `match` trees.
- Add new language support through `runtime/queries/{lang}/tags.scm` when possible. Reach for Rust extractor overrides only when the grammar or semantics require it.
- Workspace crates are allowed once a boundary is stable and the split reduces compile/test blast radius without creating a catch-all shared `types` crate.
- **All `CODESIGHT_*` env var names live in `src/config/env.rs`.** Do not
  sprinkle raw `std::env::var("CODESIGHT_*")` calls in new code — look up the
  const and add one if you need a new knob. See [`CONFIGURATION.md`](CONFIGURATION.md).
- **MCP arguments are coerce-then-validated before handlers see them**
  (`src/service/agent_v1/validate.rs`). Handlers consume args via the
  `ValidatedArgs<T>` newtype (`ValidatedArgs::<T>::from_validated(v)?.into_inner()`),
  not raw `serde_json::from_value` — that path is gone, and typed-deser failures
  surface as `ToolError::InvalidArgument` instead of leaking as `Internal`.
  Schema changes land in the same patch as the handler change.
- **Read-path freshness gating** lives in `src/service/agent_v1/freshness.rs`.
  Add new MCP tool names there if they observe indexed state — write/utility
  tools (those that drive reindexing) stay off the list. The dispatcher
  blocks in strict mode and post-annotates `STALE_DATA_WARNING` in relaxed
  mode automatically; handlers do not need to know about freshness.

## Commands

### Quality gates

These gates apply only at merge/release time, or when the user explicitly asks for them. During day-to-day work, prefer targeted runs (see `Day-to-day commands`).

1. **Clippy (warnings denied):** `cargo clippy --all-targets -- -D warnings`
2. **Tests (merge gate only):** `./scripts/nextest-cache` (full suite). For a merge-style check that must not trust a cache skip, run `./scripts/nextest-cache --no-cache` once after meaningful changes (or `./scripts/nextest-cache clean` then `./scripts/nextest-cache`).

Formatting is expected to be clean: `cargo fmt --all -- --check` (or run `cargo fmt --all` before commit).

### Day-to-day commands

**Default: targeted tests only.** Run the smallest scope that proves the behavior you changed — a single test, a single binary, or a crate lib scope. Do **not** run the full suite unless the user explicitly asks for it, you are landing a cross-cutting refactor that genuinely needs the whole thing, or you are preparing an explicit merge/release gate check. The full suite is slow and obscures which change caused a failure; targeted runs tell you faster.

Use **`./scripts/nextest-cache`** for all test runs. It wraps **[cargo-nextest](https://nexte.st/)** (`cargo install cargo-nextest` or your OS package manager) and skips repeated work when inputs and args match a prior successful run — **do not** invoke `cargo nextest run` directly from agents or local loops; it is unnecessarily slow compared to the wrapper.

Targeted runs (the default):

```bash
./scripts/nextest-cache --lib                              # crate lib unit tests
./scripts/nextest-cache --test <name>                      # one integration binary
./scripts/nextest-cache -p codesight-storage --lib         # one workspace crate
./scripts/nextest-cache -p codesight-extractors --lib
./scripts/nextest-cache -E 'test(my_specific_case)'        # filter expression
cargo fmt --all
cargo clippy --all-targets -- -D warnings
```

Full-suite runs are reserved for explicit merge/release checks or when the user asks for them:

```bash
./scripts/nextest-cache                                    # cached full suite (only when asked)
./scripts/nextest-cache --no-cache                         # authoritative full run (only when asked)
./scripts/nextest-cache clean                              # clear wrapper cache
```

Use plain `cargo test` only when nextest is unavailable in an environment, and prefer restoring nextest there.

## Cleanup standard

Delete documents that only narrate obsolete workflows. Keep the repo focused on code, tests, and the minimum docs needed to ship and maintain the product.
