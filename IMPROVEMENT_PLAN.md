# Codesight-MCP Improvement Plan

**Source:** Deep dogfood audit, 2026-04-12. One full refactoring session
(macro dedup, cycle-breaking, boilerplate elimination) with every codesight
call and every Grep/Read fallback traced and root-caused.

**Goal:** Eliminate agent fallbacks to Grep/Read during refactoring sessions.
Current: 12 Grep fallbacks per session. Target: 0.

---

## Current state — what's genuinely great

224M+ total tokens saved. Individual calls routinely save 99%+.

| Tool | Typical savings | Killer use case |
|---|---|---|
| `evaluate_quality` (hotspots/health/signal) | 99.7% | Server-side analysis, tiny summary returned |
| `inspect_graph` (cycles/importers/find_refs) | 99.9% | Graph query vs reading hundreds of files |
| `search_catalog` (keyword/regex) | 99.5% | Focused search vs full-text scan |
| `inspect_structure view=file_outline` (batch) | 99.0% | 13 file outlines in one call |
| `read_content` (batch symbol refs) | 70% | Read 6 function bodies in one call |

**Best features for refactoring:**
- `file_outline file_paths=[...]` — batch outlines, the #1 time saver
- `find_references identifiers=[...]` — batch refactoring safety net
- `evaluate_quality analysis=test_gaps` — risk-ranked untested functions
- `dependency_cycles` — exact cycle detection for structural work

---

## The gap: verification phase

A refactoring session has three phases:

1. **Understand** — codesight excels (search, outline, graph, quality)
2. **Change** — not codesight's job (Edit, Write, Bash)
3. **Verify** — codesight is weak (count, confirm absence, trace imports)

**All 12 Grep fallbacks happened in phase 3.** The fixes below close this gap.

---

## P0 — Dogfood-critical (eliminates 8 of 12 fallbacks)

### P0-A: `search_catalog output_mode=count`

**Problem.** 4 Grep fallbacks per session for "how many X exist?" and
"confirm zero remaining X". `search_catalog` returns items up to `limit`
but has no way to just count without streaming results.

**Real examples from this session:**
```
Grep pattern="ensure_repo_store" output_mode=count  → 50 occurrences
Grep pattern="fn scan_.*recursive"                  → 0 (verify removal)
Grep pattern="require_store!"  | wc -l              → 37 (verify agent work)
Grep pattern="let store = match" output_mode=count  → 36 (plan scope)
```

**Solution.** Add `output_mode` enum param to `search_catalog`:
- `items` (default) — current behavior, returns full items array
- `count` — returns `{ "count": N, "files": N }` with no items
- `files_with_matches` — returns `[{ "file": "path", "count": N }]`

**Implementation sketch:**
- File: `src/service/agent_v1/catalog_search/mod.rs`
- Add `output_mode` to `SearchCatalogArgs` (default `items`)
- For `count` mode: run BM25/regex as normal, but skip `build_item()`
  serialization. Return only the counts. ~30 lines.
- For `files_with_matches`: group hits by file, return file+count. ~15 lines.
- Update `src/tools.json` schema with the new enum param.
- Tests: one test per mode in `tests/agent_v1_input_validation.rs`.

**Token savings:** ~2k per session (eliminates count Grep + parsing overhead).

**Grep calls eliminated:** 4 per refactoring session.

### P0-B: `inspect_graph graph=imports file_path=X`

**Problem.** 4 Grep fallbacks per session for "what does this file import
FROM?" The existing `graph=importers` gives the reverse direction (who
imports this file). When tracing dependency cycles, you need both directions.

**Real examples from this session:**
```
Grep pattern="use.*catalogs" path=scm/        → trace cycle edge direction
Grep pattern="use.*scm"      path=catalogs/   → trace reverse edge
Grep pattern="use.*metadata"  path=scm/       → verify import cleanup
Grep pattern="cross_repo_map" path=src/        → find all import sites
```

**Solution.** Add `graph=imports` (or `outgoing_deps`) to `inspect_graph`:
- Input: `file_path` (same as `importers`)
- Output: `{ "file_path": "X", "imports": [{ "file": "Y", "names": ["A","B"] }] }`

**Implementation sketch:**
- File: `src/service/agent_v1/graph.rs` or `graph_tail/dependency.rs`
- The dependency graph from `store.dependency_graph_strict()` already has
  directed edges. `importers` filters `target == file_path`. This new
  variant filters `source == file_path`.
- ~20 lines of handler code + graph enum variant.
- Update `src/tools.json` to add the new graph variant.
- Test: one integration test verifying known import edges.

**Token savings:** ~3k per session (structured results vs raw Grep lines).

**Grep calls eliminated:** 4 per refactoring session.

---

## P1 — UX friction (eliminates remaining 4 of 12 fallbacks)

### P1-A: Regex mode error message

**Problem.** 2 Grep fallbacks from confusion: `search_catalog
query="infallible regex expect" query_mode=regex` returned 0 results
silently. The `query` field IS the regex pattern, but the name "query"
doesn't signal this. `Grep` names its field `pattern`, making the regex
nature obvious.

**Solution (both, they're cheap):**
1. Accept `pattern` as an alias for `query` when `query_mode=regex`.
   If both are set, error. ~5 lines in `validate.rs` arg coercion.
2. When `query_mode=regex` and the query fails regex compilation,
   return a clear error: `"query is used as a regex pattern in regex
   mode. Your query 'X' failed to compile: <error>. Did you mean
   query_mode=keyword?"` Currently it just returns 0 results.
   ~10 lines in the regex handler.

**Grep calls eliminated:** 2 per session.

### P1-B: `context_lines` in keyword mode

**Problem.** 2 Grep fallbacks for "show me matches with surrounding context".
`context_lines` exists but only works in `query_mode=regex` with
`unit_kinds=["file"]`. In keyword mode with `unit_kinds=["file"]`, it's
silently ignored.

**Real example:**
```
Grep pattern="let store = match self\.ensure_repo_store" -A 3
→ Needed to see the 4-line pattern to plan macro replacement
```

**Solution.** When `unit_kinds` includes `"file"` and `context_lines > 0`,
apply context expansion regardless of query_mode. The file-text search
path already knows the match line; extending the window is trivial.

**Implementation:** In the file-text hit builder (likely in
`catalog_search/mod.rs` or the BM25 file-text path), check
`args.context_lines` and expand the snippet window. ~15 lines.

**Grep calls eliminated:** 2 per session.

---

## P2 — Token waste reduction (no Grep fallbacks, but saves tokens)

### P2-A: `read_content` hint about Write harness

**Problem.** Agent reads files via `read_content` (efficient, batched),
then must re-read via `Read` tool before `Write`/`Edit` because the
Claude Code harness tracks read state separately. This doubled token
spend on 8+ files this session (~4k tokens wasted).

**Solution.** Add a one-line hint to `read_content` response metadata:
`"tip": "Use the Read tool if you plan to edit these files."` No code
change in handlers — just a static string in the response envelope.

### P2-B: Promote `find_references` discoverability

**Problem.** `find_references` is the most useful refactoring tool but it's
buried under `inspect_graph`. An agent looking for "where is X used?" might
try `search_catalog` first (less precise) or `Grep` (no structure).

**Solution options:**
1. Add `find_references` as a top-level tool alias (forwards to
   `inspect_graph graph=find_references`). ~20 lines.
2. Or: add it to the `search_catalog` description: "For usage/reference
   tracking, prefer `inspect_graph graph=find_references`."
3. Or: add a `_meta.suggestion` when `search_catalog` keyword query
   matches an exact symbol name: "Tip: use `inspect_graph
   graph=find_references identifier=X` for complete reference tracking."

### P2-C: Batch `find_references` should report match counts

Currently `find_references identifiers=["A","B"]` returns individual
references but not a top-line count. Adding `"reference_count": N` per
identifier (already present) plus `"total_references": N` at the top
level would let agents skip count Grep calls after using find_references.

---

## P3 — Nice-to-have (future sessions)

### P3-A: `search_catalog output_mode=files_with_matches`

Like Grep's `files_with_matches` mode — returns only file paths, no
snippets. Useful for "which files contain X?" when you don't need the
match content. Saves tokens when hit count is high but you only need
the file list for planning.

**Note:** This could be bundled with P0-A as a third `output_mode` variant.

### P3-B: Diff-aware search

`search_catalog since_commit=<sha>` — search only files that changed
since the given commit. Useful for "did my edit introduce any new
instances of the pattern I was removing?" This is a verification
accelerator but requires ingest-time change tracking.

### P3-C: `evaluate_impact analysis=verify_removal`

A first-class "confirm this pattern is gone" check:
```json
{
  "analysis": "verify_removal",
  "pattern": "fn scan_.*_recursive",
  "scope": "src/framework/"
}
```
Returns `{ "remaining": 0, "verified": true }` or
`{ "remaining": 3, "locations": [...] }`. Combines regex search + count
in one purpose-built call. Could be syntactic sugar over `search_catalog
output_mode=count` but the intent is clearer.

---

## Existing backlog items (from REFACTOR_WORKLOG.md)

These pre-date this audit but remain relevant:

| Item | Priority | Notes |
|---|---|---|
| Unify `_meta` construction (`ToolEnvelope<T>`) | High | Major LoC reduction, eliminates `raw_bytes` bugs |
| Kill `assemble_context mode=workspace_digest` if unjustified | Medium | Bloat hunt — cut if redundant with `query_suggestions` |
| Token-accurate counting (`tiktoken-rs`) | Medium | Accurate numbers unlock the improvement metric |
| `inspect_graph graph=calls` real call-site extraction | Medium | Tree-sitter call queries → accurate blast_radius |
| Session-scoped negative evidence | Low | "Symbol doesn't exist" should be cached |
| Quality score confidence reporting | Done | Already shipped with `confidence` field |
| Dual-layer scoring (structural + agent readiness) | Done | Already shipped as governance signal |

---

## Implementation status — completed 2026-04-12

All items implemented and verified with 21 integration tests (9 new).

**Phase 1 (P0) — DONE:**
1. `search_catalog output_mode=count` + `files_with_matches` — DONE
2. `inspect_graph graph=imports` (outgoing deps) — DONE
3. `tools.json` updated for both — DONE
4. Tests added and passing — DONE

**Phase 2 (P1) — DONE:**
5. `pattern` alias + regex compile error message — DONE
6. `context_lines` in keyword mode — already worked (no change needed)

**Phase 3 (P2) — DONE:**
7. `read_content` tip about Write harness — DONE
8. `_meta.suggestion` for `find_references` discoverability — DONE
9. Batch `find_references` `total_references` count — DONE (P2-C)

**Verification:** 150 lib tests + 21 integration tests all pass. Zero regressions.

---

## Success metric

| Metric | Before | After |
|---|---|---|
| Grep fallbacks per refactoring session | 12 | 0 (all covered by new tools) |
| Read double-reads per session | 8+ files | 0 (tip added) |
| Token waste from fallbacks | ~10k per session | < 1k |
| Agent tools needed for full refactor | codesight + Grep + Read | codesight only (+ Edit/Write/Bash for changes) |
