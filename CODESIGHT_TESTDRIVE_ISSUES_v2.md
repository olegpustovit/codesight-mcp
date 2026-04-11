# Codesight MCP Test-Drive v2 — Open Issues

Driven against `OlegHQ/codesight-mcp` (this repo) via MCP, 2026-04-11.
Every tool in `src/tools.json` called at least once.

## P0 — Response-size blowouts

### 1. `evaluate_quality analysis=dead_code` exceeds MCP response budget
- `granularity=file`, defaults: **111 KB** response, rejected by harness.
- `granularity=symbol`, defaults: **1.24 MB** response, rejected by harness.
- Root cause in `src/service/agent_v1/quality.rs:158` and
  `src/analysis/dead_code/mod.rs:88-128`: each dead entry carries a prose
  `evidence_notes` block, and the only cap is `top_n.unwrap_or(50)` —
  50 × ~2 KB ≈ 100 KB, no byte budget.
- **Fix options** (pick all):
  - Lower default `top_n` to ~15 for `dead_code` (keep 50 for `hotspots`).
  - Add `max_tokens` to the `evaluate_quality` schema and wire through a
    byte-budget packer like `pack_catalog_items_to_token_budget`.
  - Drop `evidence_notes` from each entry; keep one top-level
    `evidence_notes` block documenting the methodology.
  - For `granularity=symbol`, only emit symbols from dead files that are
    already in the truncated `dead_files` list (currently it walks
    `all_symbols` and then truncates — builds the full vec first).

### 2. `evaluate_quality analysis=dead_code` reports 94% dead on a live repo
- `repo_health` on this repo returns `dead_code_pct: 94.35`, `dead_count: 384`
  out of 407 files. The crate builds and ships — the detector is wrong.
- Likely cause: reachability seed roots don't include Rust `lib.rs` / `main.rs`
  correctly for workspace members, or `pub` surfaces aren't treated as roots.
- Live code in `src/service/agent_v1/*.rs` (hot dispatcher paths) showing up
  as "unreachable_from_live_roots" is the tell.
- Fix: audit `analysis::dead_code::entrypoints` and
  `build_repo_reachability` against workspace-member Rust targets.

## P1 — Schema drift and contract leaks

### 3. `top_n` / `min_complexity` not advertised on MCP `tools/list`
- `src/tools.json:283,287` declares both fields for `evaluate_quality`.
- The MCP tool schema actually served to clients omits them.
- This session's loaded schema shows only `analysis, entry_point_patterns,
  granularity, include_tests, min_confidence, refs, repo_path, source_id`.
- Suspect: `mcp.rs` `tools/list` returns a different schema than
  `validate::build_schemas` uses. Check the path in `src/mcp.rs:195`.

### 4. `catalog_sources`: code+docs share the same `source_id`
- Both entries have `source_id: "OlegHQ/codesight-mcp"`, distinguished only
  by `domains: ["code"]` vs `["docs"]`.
- `read_content`/`search_catalog` take only `source_id` and have to infer
  domain from the ref. Ambiguous for operators and for tool wrappers.
- Fix: either namespace docs `source_id` (e.g. `…::docs`) or make `domain`
  a required arg on read/search paths that need to disambiguate.

## P2 — Noise and correctness

### 5. `inspect_structure view=repo_outline` surfaces index artifacts
- Top items returned:
  - `.codesight-index/_savings.json::total_tokens_saved#constant`
  - `.codesight-index/_savings.json::total_cost_avoided#constant`
  - `.mcp.json::mcpServers#constant`
- These are codesight's own index-dir artifacts — should never appear in
  a repo outline.
- Fix: exclude `.codesight-index/**` from indexing (or at least from
  outline views); also demote top-level config JSON files in the default
  outline ranking.

### 6. `inspect_graph graph=dependency_cycles` reports a 21-node SCC over extractors
- The entire `crates/codesight-extractors/src/extractors/**` tree is
  reported as one cycle spanning 21 files: `asm.rs`, `ejs.rs`, `luau.rs`,
  `scss.rs`, `verse.rs`, `vue.rs`, `yaml.rs`, multiple `mod.rs`, residual/
  host_semantics/multi_dialect subtrees.
- This is almost certainly an artifact of how `mod` declarations feed
  the file-level dep graph — sibling modules under a shared `mod.rs`
  get lumped into one SCC because the parent re-exports all of them.
- Fix: treat `pub mod x; pub mod y;` as parent→child edges only, not
  child→parent. Or compute SCCs on the symbol reference graph, not the
  mod declaration graph.

### 7. `repo_health` always reports `index_stale: true`
- Re-ingested seconds earlier, `manage_indexes status` shows the index
  updated successfully, yet `repo_health.findings[0].evidence.is_stale:
  true` and `index_stale: true` at the top level.
- Freshness check is misfiring for same-session reads. Investigate
  `src/service/agent_v1/freshness.rs` staleness signal.

### 8. `repo_health` findings block doesn't surface the 94% dead-code number
- `findings[0].summary: "index metadata snapshot"`, severity `info`.
- But the payload has `dead_code_pct: 94.35` — a number that should be
  a `warning`/`error` finding with summary like "94% of files unreachable
  from live roots (detector may be misconfigured)".
- Fix: emit a per-metric finding when `dead_code_pct > 50` or
  `cycle_count > 0`, tied to the numeric evidence.

### 9. `search_catalog query_mode=regex` returns mixed score shapes
- Code/file hits come back with no `score` field; doc hits come back
  with `score: 1.0`. Caller can't sort a single result list by score
  without null-handling.
- Fix: populate `score` for file hits (BM25 or a stable placeholder) or
  drop it from docs for parity.

## P3 — Polish

### 10. `search_catalog` file hits use `stable_id: "<path>#L<line>"`
- E.g. `CODESIGHT_TESTDRIVE_ISSUES.md#L108` is the `stable_id`.
- That format isn't accepted back by `read_content` as a kind=file ref
  without parsing. Either document the grammar or return a structured
  `{path, line}` on file hits instead of a composite stable_id.

### 11. `evaluate_impact blast_radius` returns empty for a clearly-imported symbol
- Called on `coerce_and_validate` (imported by ~12 files via the
  `validate` module). `importer_count: 0`, `direct_dependents_count: 0`.
- `inspect_graph graph=importers` on the same file correctly reported
  12 importers.
- Fix: blast_radius should reuse the same file-importer edge set, then
  refine with symbol-level refs. Right now it seems to query symbol-
  reference edges only, which are sparser.

### 12. `ingest_sources` response metadata volatility
- `catalog_sources` calls within ~30 seconds showed `file_count` flipping
  between 406 and 407 and `symbol_count` between 4059 and 4061.
- Either a background watch-mode reindex is running (fine, but should
  set a `reindexing: true` flag) or the counts are non-deterministic.
  Investigate.

## Closed quick wins — not filing

- `read_content` symbol body return looks correct (flat shape, body,
  ref echoed).
- `assemble_context context_bundle` with `max_tokens=1500` and
  `include_budget_report=true` correctly reports `used_tokens: 383` and
  a populated `included_symbols` list.
- `inspect_structure file_outline` clean output, `extraction_tier:
  curated_query_pack` annotation useful.
- `inspect_graph graph=importers` correct 12-importer list.
- `manage_indexes action=status` clean.
