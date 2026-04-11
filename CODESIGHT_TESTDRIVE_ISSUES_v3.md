# Codesight MCP Test-Drive v3 — Open Issues

Driven against `OlegHQ/codesight-mcp` (this repo) via MCP, 2026-04-11.
Follow-up to `CODESIGHT_TESTDRIVE_ISSUES_v2.md`. Every tool in `src/tools.json`
exercised at least once with defaults and one non-default param.

## Closed in v3 session (follow-up, 2026-04-11)

- **v2 #4** — `catalog_sources` entries now carry `domain` (`"code"`/`"docs"`)
  and `stable_key` (`"<source_id>#<domain>"`) alongside the legacy `domains`
  array. Downstream callers can key off `stable_key` to pair code and docs
  indexes without string-parsing the array. The shared `source_id` remains
  as designed for hybrid tooling.
- **v2 #5** — `is_outline_noise` was dead code in `src/storage/symbol_reads.rs`
  (the live path is `codesight_storage::storage::symbol_reads`). Moved the
  filter into the live crate and extended it to cover `build.rs` at any
  depth plus `.codesight.toml`. `most_central_symbols` now runs its result
  through the same filter so PageRank no longer highlights build scripts.
- **v2 #7** — `crates/codesight-storage/src/db.rs::check_staleness` now
  walks with `ignore::WalkBuilder` (same `git_ignore` + SKIP_DIRS config the
  indexer uses) instead of `walkdir`. Previously, any gitignored file with
  a matching extractor permanently marked the index stale because the
  walker saw it but the indexer had skipped it.
- **#6 (new)** — `manage_indexes` now advertises `source_id` as required in
  `tools.json`. Every action (`invalidate`, `delete`, `rebuild`, `status`)
  needs it, so the schema was lying to agents. Description also spells out
  the `rebuild`-only `root_hint` requirement.
- **#16** — `assemble_context mode=query_suggestions` now calls the new
  language-agnostic `IndexStore::most_imported_files` (in-degree over the
  stored `dependency_edges` table) instead of the Python-only helper, so
  Rust/TS/etc. repos actually populate `most_imported_files`. Example
  queries now skip symbols from noisy files (`build.rs`, `.mcp.json`, …)
  and restrict the sampler to real code kinds (function/method/class/
  struct/trait/interface/enum) so the first suggestions land on the public
  API rather than editor config keys.
- **#17** — `repo_outline.most_central_symbols` drops the misleading
  `symbol_id` field (it was the file path, not a symbol id) and now emits
  `{ kind: "file", path, score }`. `tests/pagerank.rs::outline_most_central_*`
  updated to match.
- **dogfooding gap (new)** — during this session the agent still defaulted
  to `Grep`/`find`-style tools for some lookups (e.g. searching for
  `check_staleness`, `dependency_graph`, `extractor_for_path`) despite
  `search_catalog` being the correct first choice per `AGENTS.md`. Logged
  as a reminder that the dogfooding habit needs stronger reinforcement in
  session priming. No product fix yet; the fallback worked but burned
  context and hid retrieval gaps we would have noticed via codesight.

## Closed in v3 session (2026-04-11)

- **#14** — `doc_coverage` now rejects empty `refs` with
  `InvalidArgument: doc_coverage requires at least one refs[*] entry with kind=symbol`.
  `tools.json` schema description flipped to "required".
- **#15** — `evaluate_impact.refs[*]` sub-schema now carries an `items`
  object including `new_name`, so `tools/list` advertises the field that
  `rename_safety` requires at runtime.
- **#18** — `.scm`, `.toml`, `.md`, `.yaml`/`.yml` now resolve in
  `language_for_path`, and `get_file_content` falls back to the path-based
  detector when the stored `file_language` is missing/unknown.
- **#19** — `read_content` file reads now emit `end_line_clamped: true` and
  append `"end_line_clamped"` to the `warnings` array whenever the requested
  `end_line` exceeded the file length.
- **v2 #2** — `dead_code` detector now promotes Cargo targets to live roots.
  `src/analysis/dead_code/entrypoints::detect_cargo_target_files` walks every
  indexed `Cargo.toml`, auto-discovers `tests/**/*.rs`, `examples/**/*.rs`,
  `benches/**/*.rs`, and reads `[[test]]` / `[[example]]` / `[[bench]]` /
  `[[bin]]` entries for explicit `path` overrides. Evidence note
  `cargo_target` surfaces the attribution.
- **v2 #8** — `repo_health` now emits a `repo_health:dead_code` finding at
  `warning` ≥20% and `error` ≥50%, with a hint to check Cargo target roots
  if the detector is misconfigured. Finding id renamed from the old
  `repo_health:dead_code_high` (single-threshold) to `repo_health:dead_code`.

## Pre-existing regressions uncovered during session

- `tests/meta_disclosure.rs:340` dispatches `evaluate_impact analysis="churn_rate"`,
  which is not in the served enum. Test was already failing on `main` before
  this session (verified via `git stash` baseline). Fix: either add
  `churn_rate` back to the impact enum or update the test to a valid analysis.
- `cargo clippy --all-targets -- -D warnings` fails on
  `src/retrieval/sections/builder.rs:27` (items-after-test-module) and an
  `if let` in the same area. Also pre-existing on `main`.

## Closed since v2 (no longer filing)

- **v2 #1** — `evaluate_quality dead_code` response blowout. `max_tokens` is
  now advertised on the schema and trims the payload. Defaults return ~700
  response tokens for `granularity=file`.
- **v2 #3** — `top_n` / `min_complexity` schema drift. Both now appear in the
  served `tools/list` schema for `evaluate_quality`.
- **v2 #6** — 21-node extractor SCC. `dependency_cycles` now reports 6 small
  cycles (the framework provider/render/snapshot trios are real; see #13).
- **v2 #9** — regex search score shapes. File hits now ship `score: 1.0`
  alongside doc hits — parity.
- **v2 #10** — `search_catalog` file hit `stable_id` no longer embeds
  `#L<line>`. It now returns the plain path, with `line`/`start_line`
  carried as structured fields.
- **v2 #11** — `evaluate_impact blast_radius` now reports 13 direct importers
  for `coerce_and_validate`, matching `inspect_graph graph=importers`.

## Still open from v2

### v2 #2 (downgraded) — dead-code detector flags Cargo binary targets
- `dead_code_pct` is down to **32.27%** (from 94%), but the top of the list is
  still dominated by real Cargo targets:
  - `tests/agent_v1_envelope.rs`, `tests/agent_v1_caching.rs`,
    `tests/adaptive_languages.rs`, etc. — integration test binaries.
  - `examples/dump_ast.rs`, `examples/test_scm_new.rs` — `[[example]]` targets.
  - `crates/*/benches/*.rs` — `[[bench]]` targets.
- All are `signal: unreachable_from_live_roots`, `confidence: 1.0`.
- Fix: teach `analysis::dead_code::entrypoints` to parse `Cargo.toml` workspace
  members and treat every `[[test]]`, `[[example]]`, `[[bench]]`, and
  auto-discovered target file as a live root. Today it apparently only
  considers `src/lib.rs` / `src/main.rs`.

### v2 #4 — `catalog_sources`: code+docs share the same `source_id`
Still both entries return `source_id: "OlegHQ/codesight-mcp"`, distinguished
only by `domains`. `read_content` / `search_catalog` still infer domain from
the ref. No schema-level disambiguation.

### v2 #5 (partial) — `inspect_structure repo_outline` surfaces config artifacts
- `.codesight-index/**` is now excluded (good — `file_tree` on that path
  returns empty).
- But `.mcp.json::mcpServers#constant` and several `build.rs` constants
  (`QueryPackBuildInput`, `macos_link_sdk`, `main`) still head the outline.
- Fix: demote top-level config JSON and `build.rs` in default outline
  ranking; they're not what an operator wants to see first.

### v2 #7 — `repo_health` always reports `index_stale: true`
Re-ingested within the same session (<1 min old), `manage_indexes status`
clean, and `repo_health.findings[*]` still includes both
`repo_health:index_stale (warning)` and `is_stale: true` in the evidence.
Freshness check is still misfiring for same-session reads.

### v2 #8 — `repo_health` findings don't surface `dead_code_pct`
`findings` now emits `repo_health:cycles` when `cycle_count > 0`, which is
good. But `dead_code_pct: 32.27` still does not produce a finding. Add a
`repo_health:dead_code` finding when the ratio crosses a threshold
(>20% `warning`, >50% `error` plus "detector may be misconfigured" hint).

## New in v3

### 13. Framework provider/render/snapshot cycles are real
Not a bug in the detector — but `dependency_cycles` correctly surfaces five
3-node cycles across `src/framework/{dbt,laravel,nextjs,nuxt}/` and a 6-node
SCC across `crates/codesight-extractors/src/extractors/engine/{catalogs,scm}/`.
These are genuine structural findings; filing so they stay visible for the
S4 workstream.

### 14. `evaluate_quality analysis=doc_coverage` silently returns 0%
- Called with no `refs`: `{coverage_pct: 0.0, documented_count: 0,
  undocumented_count: 0, symbols_checked: 0}`.
- The `0.0%` looks like a real measurement — it isn't; nothing ran.
- Fix: either default to top-N public symbols (mirrors `hotspots`) or
  reject with `InvalidArgument: refs required`. Silent zero is the worst
  of both worlds.

### 15. `evaluate_impact analysis=rename_safety` has undocumented `new_name` field
- Calling with a valid symbol ref errors:
  `"rename_safety requires new_name on the ref."`
- But the served `tools/list` schema for `refs[*]` lists only
  `{end_line, id, kind, path, source_id, stable_id, start_line}` — no
  `new_name`. Operators can't satisfy the contract without reading the
  error message or source.
- Fix: add `new_name` to the refs sub-schema when `analysis=rename_safety`,
  or move it to a top-level arg on `evaluate_impact`. Schema must match
  runtime validation (`src/service/agent_v1/impact/...`).

### 16. `assemble_context mode=query_suggestions` has empty `most_imported_files`
- Returns `most_imported_files: []` on a repo where `inspect_graph importers`
  correctly reports 13 importers for `validate.rs` and dozens of cross-module
  edges exist.
- The first five `example_queries` are `mcpServers`, `QueryPackBuildInput`,
  `generate_builtin_query_packs`, `macos_link_sdk`, `main` — all from
  `.mcp.json` and `build.rs`. Useless suggestions for an operator onboarding
  on this repo.
- Fix: (a) actually populate `most_imported_files` from the import graph;
  (b) weight suggestion candidates toward high-centrality public API symbols
  in `src/` / `crates/*/src/`, demote build scripts and editor config.

### 17. `repo_outline.most_central_symbols` mixes file and symbol identity
- Entries look like
  `{kind: "file", symbol_id: "src/config/mod.rs", file: "src/config/mod.rs"}`.
- `symbol_id` is a file path, not a stable symbol id. Clients that try to
  round-trip these into `read_content refs[{kind: "symbol", id: …}]` will
  fail.
- Fix: either change `kind` to `"module"` / `"file"` consistently and drop
  `symbol_id`, or compute centrality on real symbols and return proper
  `symbol_id` values.

### 18. `read_content` reports `language: "unknown"` for tree-sitter query files
- `runtime/queries/swift/tags.scm` (a first-party file in this repo) comes
  back with `language: "unknown"`.
- Minor, but we own the extractor pipeline — `.scm` should resolve to
  `tree-sitter-query` or at least `scheme`.

### 19. `read_content` out-of-range `end_line` silently clamps
- Requested `start_line=38, end_line=42` on a 39-line file; response returns
  `end_line: 39` with no warning. Clamping is fine, but the ingest-style
  `warnings: []` array should carry `end_line_clamped: true` for tools that
  depend on it (diff tools, patch generators).

## Re-run summary

```
catalog_sources             ok (2 sources, source_id collision)
ingest_sources              (not called — manage_indexes status used instead)
manage_indexes status       ok
search_catalog keyword      ok, score 0.87
search_catalog regex        ok, score parity restored
inspect_structure file_tree ok (.codesight-index excluded)
inspect_structure outline   ok but leaks .mcp.json / build.rs
inspect_graph importers     ok, 13 importers
inspect_graph dep_cycles    ok, 6 real cycles (#13)
evaluate_quality dead_code  ok, budgeted (v2 #1 closed), detector still
                            flags Cargo [[test]]/[[example]]/[[bench]]
evaluate_quality repo_health
                            ok, cycles finding present, dead_code_pct
                            finding still missing (v2 #8)
evaluate_quality hotspots   ok, top 3 look right
evaluate_quality doc_coverage
                            silent 0% (#14)
evaluate_impact blast_radius
                            fixed (v2 #11 closed)
evaluate_impact rename_safety
                            schema drift (#15)
read_content symbol         ok
read_content file+range     ok, clamp undocumented (#19)
assemble_context bundle     ok
assemble_context suggestions
                            low-quality suggestions, missing importers (#16)
```
