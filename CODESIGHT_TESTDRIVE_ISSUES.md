# Codesight MCP Test-Drive — Issues & Improvement Plan

Test-drive performed via direct JSON-RPC against `codesight-mcp serve` (the same
binary Claude Code launches) using this repo (and
`tests/fixtures/code_reference`) as the corpus. All 10 agent-v1 tools were
exercised. Findings are grouped by severity.

## P0 — Server-crashing / data-loss bugs

### 1. UTF-8 boundary panic in `make_summary` kills the MCP server
**Symptom.** First attempt to `ingest_sources` on this repo crashed the entire
server process (MCP client saw `Connection closed`):

```
thread 'tokio-rt-worker' panicked at src/retrieval/sections/builder.rs:15:33:
byte index 200 is not a char boundary; it is inside '…' (bytes 198..201)
fatal runtime error: failed to initiate panic, error 5, aborting
```

Trigger: any doc section whose first 200 bytes end mid-codepoint. `REFACTOR_PLAN.md`
contains `…` around byte 198, which is enough to kill the process.

**Root cause.** [`src/retrieval/sections/builder.rs:15`](src/retrieval/sections/builder.rs)
does `&trimmed[..200]` without honoring char boundaries.

**Fix.** Use `trimmed.char_indices().take_while(|(i, _)| *i < 200)` (or
`floor_char_boundary` on nightly) and fall back to the largest valid prefix.
Add a unit test with a multi-byte codepoint straddling the budget. Audit every
other `&s[..N]` / `s.split_at(N)` site for the same class of bug (I only
inspected the crashing one).

### 2. Panic path is `abort`, not graceful tool error
The panic above aborts the whole process, losing all in-flight dispatches and
dropping the MCP transport. Tokio worker panics should be caught at the
dispatch boundary and turned into a `ToolError::Internal` envelope so one bad
file never takes down the server. Add a `catch_unwind` or a per-dispatch task
boundary in `src/service/mod.rs` around tool handler invocation.

## P1 — Contract drift: `tools.json` schemas vs. handler requirements

Many tools return `INVALID_REQUEST` for arguments the schema advertises as
optional, or reference parameters the schema doesn't declare at all. A caller
that trusts `tools/list` will hit avoidable errors on first call.

| Tool | `analysis`/`graph`/`view` | Schema says | Handler actually requires |
|---|---|---|---|
| `inspect_graph` | `calls` | `ref` (object) optional | `symbol_id` (not in schema) |
| `inspect_graph` | `importers` | `ref` optional | `file_path` (not in schema) |
| `evaluate_impact` | `changed_units` | `refs` optional | `since_sha` (not in schema) |
| `evaluate_impact` | `blast_radius` | `refs` optional | `refs[0]` |
| `evaluate_impact` | `rename_safety` | `refs` optional | `refs[0]` |
| `inspect_structure` | `class_hierarchy` | `path` documented as "file_tree subtree prefix" | `path` repurposed as class name |
| `assemble_context` | `context_bundle` | `refs` optional | `refs` with a symbol ref |

**Fix plan.**
1. For each mode, split the argument structs so `tools.json` documents the
   required parameters per mode (either via `oneOf` in JSON schema, or by
   exposing per-mode input structs via `ValidatedArgs<T>`).
2. Add `since_sha`, `symbol_id`, `file_path` as first-class schema fields.
3. Rename `inspect_structure.path` in `class_hierarchy` to `class_path` or
   similar — don't repurpose an existing field.
4. Add contract tests that enumerate every enum value for every tool and
   assert the schema describes the actually-required parameters.

## P1 — Unimplemented / partial analyses masquerade as "ok"

Several analyses return `ok: true` with placeholder data and only a warning:
- `evaluate_quality analysis=repo_health` → `PARTIAL_REPO_HEALTH` — `dead_code_pct`,
  `dead_count`, `cycle_count`, `cycles_sample`, `top_hotspots` are all placeholders.
- `assemble_context mode=ranked` → `PARTIAL_RANKED_CONTEXT` — `centrality_score`
  is always `0.0`.
- `inspect_graph graph=cross_source` → `NOT_IMPLEMENTED` but still returns `ok=true` with empty edges.
- `evaluate_quality analysis=coverage` → `NOT_IMPLEMENTED` warning, `ok=true`, empty findings.

This violates the usual "either do the thing or return a structured error"
contract and makes it impossible for an automated agent to detect missing
capability without string-matching warning codes.

**Fix plan.** Decide per analysis: either (a) fully implement and drop the
warning, or (b) return `ok=false, category=not_implemented` with a stable
error code, and remove the mode from the schema enum until it ships.
`AGENTS.md` already mandates "no parallel public MCP surfaces" — shipping
placeholder fields is a form of surface fragmentation.

## P2 — Ingest output semantics

### `missing_extractors` is misnamed
Ingesting `tests/fixtures/code_reference` reports:
```json
"missing_extractors": ["al","blade","cpp","csharp","elixir","java","perl",
                      "php","razor","ruby","rust","xml"],
"parse_warnings":   ["Language 'al' has symbol extraction but no import extractor", ...]
```
Those languages all extracted symbols successfully (199 total). The field
actually means "no import extractor". Rename to `missing_import_extractors`,
or drop it entirely in favor of `parse_warnings`. As-is, a client
reasonably interprets it as "extraction failed for these languages".

### `catalog_sources` races against in-flight ingest
When `ingest_sources` and `catalog_sources` are dispatched back-to-back over
stdio, `catalog_sources` can return `sources: []` while the ingest is still
writing. There is no write barrier or source-visible lock. Either:
- Serialize writers vs. catalog reads inside the dispatcher, or
- Surface in-flight ingests in `catalog_sources` output (with a `status: indexing` flag), or
- Gate `catalog_sources` on the same freshness mechanism described in
  `src/service/agent_v1/freshness.rs`.

## P2 — Token budgeting is inconsistent

`_meta.tokens_saved` is `0` on many legitimate results:
- `read_content` (134 response tokens, saved=0)
- `workspace_digest` (296 tokens, saved=0)
- `query_suggestions` (300 tokens, saved=0)
- `inspect_structure view=repo_outline` (**10256** tokens, saved=0)
- `assemble_context mode=ranked` (313 tokens, saved=0)

`repo_outline` in particular is unbounded — it emitted 10k tokens for a
20-file fixture. Recommend:
1. Add a `max_tokens` / `token_budget` knob to `inspect_structure` (already
   present on `assemble_context` and `search_catalog`) and apply it to
   `repo_outline` and `file_tree`.
2. Audit `MetaBuilder` usage so every handler records
   `naive_tokens`/`response_tokens` consistently — `tokens_saved=0` on a
   handler that clearly filtered 4k → 100 tokens is a reporting bug, not a
   real zero.

## P2 — `read_content` response is double-nested

`read_content` for a file ref returns:
```json
{"items":[{"content":{"content":"const MAX_RETRIES...", ...}}]}
```
The outer `items[].content` wraps an inner `{content, ...}` object. Either
flatten the outer wrapper or rename one of the fields. As-is it's confusing
for clients and wastes tokens.

## P2 — Empty edges on `dependencies` graph

`inspect_graph graph=dependencies` on `code_reference` returns 20 nodes and
**zero edges**. This is consistent with "no import extractors" for most
languages in that fixture — but the graph is still marketed as supported.
Either:
- Implement import extraction for the priority languages (Rust, Python, TS,
  JS, Go — these are in the fixture and all show "no import extractor"), or
- Return a warning when the graph is empty because required extractors are
  missing, naming the languages.

## P3 — Regex search mode does nothing useful

`search_catalog` with `query_mode=regex`, `query=".*"` returns 0 items on a
corpus with 199 symbols. Either regex mode isn't wired into the BM25 path or
it's quietly falling through to literal match. Needs a repro test and a
decision: drop the mode from the enum, or implement it.

## P3 — CLI surface is thin

`codesight-mcp` exposes only `serve`, `check`, `init`. Operators debugging
ingest can't repro an ingest failure without a full MCP stdio harness. Add a
hidden `codesight-mcp ingest --path <p>` (and maybe `search`, `read`)
subcommand that calls the same handler paths as `serve` does, for local
diagnostics and for the self-host smoke script workstream S3.

## Suggested execution order

| # | Issue | Effort | Blocker level |
|---|-------|--------|----|
| 1 | UTF-8 panic (#1) + dispatch-level `catch_unwind` (#2) | S | P0 — ships unsafe |
| 2 | Schema/handler drift table (#3) | M | P1 — breaks first-call UX |
| 3 | Rename/fix `missing_extractors` + add import extractors for Rust/Py/TS/JS/Go (#6, #9) | M | P1 — feature-parity with prior-art |
| 4 | `read_content` double-nesting (#8) | S | P2 — token waste |
| 5 | `tokens_saved` accounting + `repo_outline` budget (#7) | S | P2 — contradicts product pitch |
| 6 | Decide fate of `repo_health`/`ranked.centrality`/`coverage`/`cross_source` placeholders (#4) | M | P1 |
| 7 | Regex search mode (#10) | S | P3 |
| 8 | Ingest/catalog race (#5) | M | P2 |
| 9 | Diagnostic CLI subcommands (#11) | M | P3 |

## How these were found

Raw JSON-RPC sequences run against a fresh `CODESIGHT_INDEX_DIR=/tmp/cs-test/.idx`
against `tests/fixtures/code_reference` (and one crashing run against the repo
root). Every tool in `src/tools.json` was called at least once; graph/impact
modes were probed for every enum value. Logs: `/tmp/cs-err.log`,
`/tmp/cs-out*.jsonl` (transient).
