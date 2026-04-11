# Codesight MCP Test-Drive — Issues & Status

Originally found by driving `codesight-mcp serve` via direct JSON-RPC against
this repo and `tests/fixtures/code_reference`. Every tool in `src/tools.json`
was called; graph/impact modes were probed for every enum value.

All items below are **resolved**; this file is now the historical log plus
the closing commit reference.

---

## ✅ P0 — Server-crashing bug

### 1. UTF-8 boundary panic in `make_summary` killed the MCP server
Ingesting this repo hit `src/retrieval/sections/builder.rs:15` on a section
summary where the 200-byte cut landed inside a multi-byte codepoint (the `…`
ellipsis in `REFACTOR_PLAN.md`):

```
thread 'tokio-rt-worker' panicked at src/retrieval/sections/builder.rs:15:33:
byte index 200 is not a char boundary; it is inside '…'
fatal runtime error: failed to initiate panic, error 5, aborting
```

**Fix.** Walk back to the nearest char boundary before slicing. Added a unit
test with a multi-byte codepoint straddling the 200-byte budget.

### 2. Panic path was `abort`, not graceful tool error
The panic above took down the whole process. Fixed by:
- Wrapping `agent_v1::dispatch` in `catch_unwind(AssertUnwindSafe(...))` in
  `src/service/mod.rs` — panics become a categorized `ToolError::Internal`
  with code `HANDLER_PANIC` instead of aborting.
- Switching release profile from `panic = "abort"` to `panic = "unwind"` in
  `Cargo.toml` so the dispatch-boundary catch actually works in shipping
  binaries.

---

## ✅ P1 — Contract and placeholder hygiene

### 3. `tools.json` schema drift
Seven tool modes required arguments that `tools/list` never advertised. Fixed
by adding per-mode fields to the schema (with descriptions naming the mode
that requires them):

| Tool / mode | Added schema field |
|---|---|
| `inspect_graph.calls` | `symbol_id` |
| `inspect_graph.importers` | `file_path` |
| `inspect_graph.dependencies` | `cross_repo` |
| `inspect_structure.class_hierarchy` | `class_path` (was repurposing `path`) |
| `inspect_structure.file_tree` / `repo_outline` | `max_tokens` |
| `evaluate_impact.changed_units` | `refs` description now names commit-ref shape |
| `evaluate_quality` | `analysis` enum tightened; added `granularity`, `min_confidence`, `include_tests`, `entry_point_patterns` |
| `assemble_context.context_bundle` | `refs` description now names symbol-ref requirement |
| `search_catalog.query_mode` | described that `regex` auto-routes to file-text |

Also wired `class_path` through the handler in
`src/service/agent_v1/structure.rs` and updated the error message.

### 4. Placeholder data masqueraded as success
- `evaluate_quality analysis=repo_health` used to emit a `PARTIAL_REPO_HEALTH`
  warning with zeroed placeholder fields for `dead_code_pct`, `dead_count`,
  `cycle_count`, `cycles_sample`, `top_hotspots`. **These analyses were
  already implemented** (`dead_code_payload`, `find_cycles`,
  `hotspots_payload`); `repo_health` just wasn't calling them. Fixed by
  wiring `repo_health` to call all three and populate real values. Live
  drive on the code_reference fixture now reports `dead_count: 20`,
  `dead_code_pct: 100.0`, `cycle_count: 0`.
- `assemble_context mode=ranked` emitted a `PARTIAL_RANKED_CONTEXT` warning
  and a `centrality_score: 0.0` placeholder on every item. Dropped the
  field and the warning.
- `inspect_graph graph=cross_source` was an `ok=true` stub. The same
  functionality is already in `graph=dependencies` with `cross_repo=true`,
  so `cross_source` was removed from the enum and the handler.
- `evaluate_quality` unknown-analysis fallthrough returned `ok=true` with
  `NOT_IMPLEMENTED`. Now returns a proper `INVALID_ARGUMENT` error listing
  the valid enum values (surfaced at schema-validation time now that the
  enum is tightened).

---

## ✅ P2 — Ingest output and token accounting

### 5. `catalog_sources` races against in-flight ingest
`catalog_sources` ran concurrently with ingest and returned empty `sources`
while ingest was still committing. Fixed by:
- Adding `ReindexStateManager::in_flight_repos()`.
- Gating `catalog_sources` in `src/service/agent_v1/freshness.rs`:
  `await_all_if_strict` blocks in strict mode, `annotate_catalog_with_in_flight`
  attaches `data.indexing: [...]` + `STALE_DATA_WARNING` in relaxed mode.

### 6. `missing_extractors` was misnamed
The field listed languages that **did** extract symbols — it meant "no
import extractor". Renamed to `languages_without_import_extractor` at every
site (`IndexResult`, `CodeIndexSummary`, both ingest response branches, and
the three integration tests). The parallel `parse_warnings` field, which
was just a prose restatement of the same list, was dropped entirely.

### 7. Token accounting
- `inspect_structure view=repo_outline` was unbounded (10256 response tokens
  on a 20-file fixture). Added `max_tokens` to the schema and wired it into
  `pack_catalog_items_to_token_budget`. Live drive with `max_tokens=500`
  now returns 635 tokens with `truncated: true`.
- `inspect_structure view=file_tree` similarly honors `max_tokens` by
  converting the token budget into an equivalent `max_files` cap.
- Several handlers passed `raw_bytes = 0` (or `raw_bytes = response size`)
  to `MetaBuilder::new`, producing bogus `tokens_saved: 0`. Fixed for
  `query_suggestions`, `workspace_digest`, `assemble_context mode=ranked`,
  and the file-kind branch of `read_content` to use real repo or file
  sizes.

### 8. `read_content` double-nested `content`
Items were shaped `items[i] = {content: {content, line_count, ...}}`, with
the outer wrapper adding nothing. Flattened to
`items[i] = {content, line_count, ..., ref}` across symbol, section, and
file refs. Updated every test that asserted on the old shape.

### 9. Rust import extraction (previously overscoped in the plan)
Original plan listed Rust/Python/TS/JS/Go as missing. After verifying
against `src/indexing/imports/mod.rs`, Python/JS/TS/TSX/JSX/Go/Vue/SQL/Dart
all already had extractors; only **Rust** was actually missing. Added:

- `src/indexing/imports/tree_sitter/rust.rs` — tree-sitter walker over
  `use_declaration` / `extern_crate_declaration`, with recursive flattening
  for `scoped_use_list` / `use_list` / `use_as_clause` / `use_wildcard`.
  8 unit tests.
- `src/indexing/imports/fallback/rust.rs` — line-based fallback for the
  no-parse path, covering `pub use`, `pub(crate) use`, multi-imports,
  wildcards, `as` aliases, inline comments, `extern crate`. 9 unit tests.
- `src/indexing/imports/resolve.rs::resolve_rust_specifier` — maps
  `::`-paths to `a/b/c.rs` or `a/b/c/mod.rs`, longest-match first, and
  strips `crate::`/`self::`/`super::` prefixes. 4 unit tests.
- Wired into `tree_sitter/mod.rs`, `fallback/mod.rs`,
  `IMPORT_EXTRACTION_CAPABILITIES`, and the dispatch match in
  `extract_imports_with_quality`.

Live drive: a fixture with `use crate::models::User` in `src/lib.rs` now
produces a `depends_on` edge `src/lib.rs → src/models.rs` with
`import_fact_quality: baseline_structural`, and the external crates
(`std`, `serde`, `tokio`, `rayon`, `proc_macro`) correctly don't produce
local edges.

---

## ✅ P3 — Minor polish

### 10. Regex search mode silently ignored
`search_catalog query_mode=regex` did nothing for symbol/section searches —
regex was only wired through `search_code_files_text` and the default
`unit_kinds` didn't include `file`. Fixed by auto-routing: when
`query_mode=regex`, the symbol/section branches are skipped and the file
branch is forced regardless of `unit_kinds`. Documented in the schema.

### 11. Diagnostic CLI subcommands
Added `codesight-mcp ingest <path>` and `codesight-mcp search <source_id>
<query>` subcommands that call the same `CodeService::dispatch` handlers as
`serve`, for operators repro'ing issues without a JSON-RPC harness.

---

## How these were found

Raw JSON-RPC sequences run against a fresh `CODESIGHT_INDEX_DIR=/tmp/cs-test/.idx`
against `tests/fixtures/code_reference` (and one crashing run against the repo
root). Every tool in `src/tools.json` was called at least once; graph/impact
modes were probed for every enum value.

## Test coverage added

- `retrieval::sections::builder::tests::*` — 3 new unit tests for the P0
  UTF-8 panic.
- `indexing::imports::tree_sitter::rust::tests::*` — 8 tests.
- `indexing::imports::fallback::rust::tests::*` — 9 tests.
- `indexing::imports::resolve::rust_resolve_tests::*` — 4 tests.
- Existing `parse_warnings`, `doc_tools`, `retrieval_tools`, `incremental`,
  and `get_context_bundle` integration binaries updated for the new
  contract (field rename, flattened `read_content`), all still green.
