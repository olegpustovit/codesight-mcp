# AGENTS.md

`codesight-mcp` — local MCP server for structural, token-efficient code
retrieval. Rust workspace at the repo root. Onboarding: [`README.md`](README.md).

## Identity

Three rules are non-negotiable:

- **Consolidated tools.** 10 tools, dispatched by `view` / `analysis` / `graph`
  enums. Never split a consolidated tool into parallel names.
- **Stateless per-request.** The agent owns session state. Any session
  awareness we add is opt-in, never a mandatory protocol.
- **No LLM calls from inside the server.** The agent *is* the LLM; summaries
  are its job.

---

## Non-negotiable policies

### 1. Shrink the code. Always.

- LoC and per-module cyclomatic complexity only go down or stay flat. 
- Defaults come from `crates/codesight-analysis/src/thresholds.rs`, surfaced
  via `evaluate_quality ... _meta_verbose=true`. Override only through
  `CODESIGHT_THRESHOLD_*` env vars.

  | Signal | Default | Action when missed |
  |---|---|---|
  | Cyclomatic complexity (per fn) | `<= 20` | Split and extract helpers. |
  | File LoC | `<= 500` soft | Split only on mixed responsibility. |
  | `navigability` | `>= 0.70` | Add re-exports, flatten module edges. |
  | `comprehensibility` | `>= 0.70` | Rename for clarity, cut nesting. |
  | `verifiability` | `>= 0.70` | Add targeted tests or type annotations. |
  | `change_safety` | `>= 0.70` | Reduce fan-in, tighten visibility. |
  | `agent_readiness_score` | `>= 0.70` | Fix the weakest dimension first. |
  | Doc coverage (public API) | `>= 60%` | Document public surfaces. |
  | Untested entry points (`test_gaps`) | `0` | Add a nextest case or drop `pub`. |
  | `structural_score` | `>= 0.60` | Follow `quality_signal.root_causes`. |
  | Per-file blast radius | `<= 10 files` | Extract code, narrow visibility. |

- Reach for `simplify`, `rust-modularity`, `rust-design-patterns` **before**
  writing a new helper. Shared helper > 3 copies; trait + registry > growing
  `match`; maintained crate > hand-rolled util.
- Split oversized/confused modules when complexity > 20, responsibilities
  mix, or fan-in is high. LoC alone is a weak signal — don't split on it.
- Features and behavior stay; regressions don't. Delete placeholders,
  aliases, and useless surfaces aggressively — update `src/tools.json` +
  tests in the same patch.

### 2. Dogfood `codesight-mcp` for every lookup.

If we don't use it on ourselves we won't notice when it breaks.

**Default → codesight mapping.** Every row is a *reach*: if you catch
yourself typing the left column, stop and use the right column.

| Default (don't) | Codesight (do) | Args |
|---|---|---|
| `Grep` a symbol/fn/type name | `search_catalog` | `query_mode=keyword`, `unit_kinds=["symbol"]`, opt. `kind=` / `fuzzy=true` |
| `Grep` substring across files | `search_catalog` | `query_mode=keyword`, `unit_kinds=["file"]`, opt. `file_pattern` / `scope` |
| `Grep -E` regex over files | `search_catalog` | `query_mode=regex`, `context_lines=N` |
| `Grep … \| wc -l` / count scope | `search_catalog` | `output_mode=count` (or `files_with_matches`) |
| Natural-language question | `search_catalog` | `query_mode=natural_language`, `domain=hybrid` |
| `Glob` / `find` / `ls -R` | `inspect_structure` | `view=file_tree` (opt. `path=`) |
| "What's in this repo" | `inspect_structure` | `view=repo_outline` / `repository_rollup` |
| File outline before reading | `inspect_structure` | `view=file_outline`, `file_path=…` (batch via `file_paths`) |
| `Read` a known file range | `read_content` | `refs=["path:start-end"]` or object form |
| `Read` a function/class body | `read_content` | `refs=[{kind:"symbol", path, start_line}]` or `stable_id` |
| Batch read N files/symbols | `read_content` | one call with N refs — never N calls |
| File's outgoing imports | `inspect_graph` | `graph=imports`, `file_path=…` |
| Incoming imports | `inspect_graph` | `graph=importers`, `file_path=…` |
| Symbol references | `inspect_graph` | `graph=references` or `find_references identifier=…` |
| Function call graph | `inspect_graph` | `graph=calls`, `symbol_id=…` |
| Cross-module deps / cycles | `inspect_graph` | `graph=dependencies` / `dependency_cycles` / `layer_violations` |
| Pre-refactor: who breaks? | `evaluate_impact` | `analysis=blast_radius`, `refs=[…]` |
| Rename safety | `evaluate_impact` | `analysis=rename_safety`, `refs[0].new_name="…"` |
| Symbol diff | `evaluate_impact` | `analysis=diff`, 2 refs in same file |
| Hotspots / complexity | `evaluate_quality` | `analysis=hotspots` |
| Dead code | `evaluate_quality` | `analysis=dead_code` |
| Repo health / doc coverage | `evaluate_quality` | `analysis=repo_health` / `doc_coverage` |
| Ticket kickoff bundle | `assemble_context` | `mode=context_bundle`, `refs=[…]`, `max_tokens=N` |
| Section summary | `assemble_context` | `mode=section_context` |
| Opening-move suggestions | `assemble_context` | `mode=ranked` / `query_suggestions` |
| Freshness / list sources | `catalog_sources`, `manage_indexes action=status`, `ingest_sources incremental=true` | — |

#### Effective use

- **Start every session with `catalog_sources`** — you need a `source_id`.
  If stale: `ingest_sources path=. incremental=true`.
- **Pick `query_mode`**: `keyword` for identifiers (fastest), `regex` for
  patterns (auto-routes to file text, *ignores* `unit_kinds`),
  `natural_language` for questions.
- **Narrow aggressively** with `unit_kinds`, `scope="src/service/"`,
  `file_pattern` — cheap token savers.
- **Batch reads.** `read_content` takes N refs; never N calls.
- **Prefer `inspect_graph` over mental graph walking.** "Let me grep every
  usage of X" = `graph=references`.
- **Budget tokens**: `max_tokens=4000` for browsing, `8000` for deep reads.
- **Quality loop on risky edits**: `evaluate_quality analysis=quality_signal
  scope="…"` before; `manage_indexes action=session_end` before sign-off
  (check `quality_delta`, watch for `QUALITY_REGRESSION`).
- **`_meta.tokens_saved` is the scoreboard.** Zero or negative = wrong tool
  or too-broad query. Re-shape.
- **Fallback only when codesight can't answer** (missing grammar, tool
  under repair). Native `Grep`/`Read` remain correct for writing,
  running tests, and editing — not for read/search.

### 4. Proactive improvement every session.

After the test-drive: *"what's my feature request, what should improve?"*
Add to the worklog backlog. Key metric: **tokens saved per query**
(`$storage_dir/_savings.json` via `check --savings`). Big simplifying
changes are welcome; bloat is the enemy.

### 5. Performance — local SSD SQLite, no excuses.

- Dispatch → handler: **< 50 ms** cached, **< 200 ms** cold.
- Incremental reindex after single-file edit: **< 100 ms**.
- Cold full ingest of this repo: **< 5 s**.

Touching a hot path requires a targeted bench or timing print. Regressions
roll back the same session.

### 6. Tests are instant and targeted, or they're debt.

- **Targeted by default**: smallest scope that proves the change via
  `cargo nextest run`.
- **Never** run the full suite unless asked, or gating a merge.
- Test > 1 s → log in worklog's slow-test list, plan a fix.
- Never `#[ignore]` or delete a test to silence failure. Root-cause it.
- Pre-existing failures / clippy errors get fixed in the same session.

---

## Layout

| Path | Purpose |
|---|---|
| [`src/`](src/) | Binary crate: CLI subcommands + stdio MCP entry point. Thin wiring only — all engine logic lives in `crates/codesight-engine`. |
| [`src/cli/`](src/cli/) | Operator subcommands (`check`, `init`, hooks). |
| [`crates/codesight-engine/`](crates/codesight-engine) | **L1 engine.** MCP tool dispatch, indexing orchestration, `AppConfig`, security, server support, `tools.json` contract. Backend-agnostic. |
| [`crates/codesight-engine/src/tools.json`](crates/codesight-engine/src/tools.json) | **Frozen MCP contract.** Schema + handler ship together. |
| [`crates/codesight-backend/`](crates/codesight-backend) | **L0.5.** `CodeBackend` + `BackendRouter`; tree-sitter + LSP impls are Cargo features (`treesitter`, `lsp`) in the same crate. Engine uses `default-features = false`. |
| [`crates/codesight-core/`](crates/codesight-core) | L0 primitives: `Symbol`, `FactQuality`, pipeline types. |
| [`crates/codesight-analysis/`](crates/codesight-analysis) | Quality, impact, graph analysis. |
| [`crates/codesight-extractors/`](crates/codesight-extractors) | Tree-sitter SCM engine + query packs. Implementation detail of the tree-sitter backend. |
| [`crates/codesight-framework/`](crates/codesight-framework) | dbt / laravel / nextjs / nuxt detection providers. |
| [`crates/codesight-imports/`](crates/codesight-imports) | Multilingual import extraction. |
| [`crates/codesight-retrieval/`](crates/codesight-retrieval) | BM25, fuzzy, embedder, section builders. |
| [`crates/codesight-storage/`](crates/codesight-storage) | WAL SQLite + forward-only migrations. |
| [`crates/mcp-rpc/`](crates/mcp-rpc) | L2: stdio JSON-RPC framing. `Dispatcher` trait (server) + `McpClient` (client). No `codesight_*` deps. |
| [`runtime/queries/`](runtime/queries) | Tree-sitter `.scm` queries (30 langs). |
| [`tests/`](tests) | Integration tests. `tests/crate_graph.rs` enforces layering. |
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | Crate split, data flow, FactQuality, SQLite config. |

Layer invariants (enforced by `tests/crate_graph.rs`):

- **L0** (primitives): `codesight-core` ← `-analysis` / `-extractors` /
  `-retrieval` / `-storage` / `-imports` / `-framework`. No upward deps.
- **L0.5** (backend): `codesight-backend` holds the trait and optional impls.
  Concrete backend code never depends on the engine.
- **L1** (engine): `codesight-engine` depends on the trait + L0. **Does not
  depend on any concrete backend** (Phase C gate — `cargo check -p
  codesight-engine` pulls no tree-sitter / LSP types through its lib).
- **L2** (protocol): `mcp-rpc` has zero `codesight_*` deps.

## Working rules

1. `agent-v1` + `crates/codesight-engine/src/tools.json` is the only public
   MCP surface. Extensibility via `view`/`analysis` enums — never parallel
   tool names.
2. **No backwards compatibility. Zero.** No shims, aliases, dual paths,
   re-exports, wrapper fns, `#[deprecated]`, "// removed" comments.
   Renames, moves, field/arg changes: the old identifier disappears in the
   same patch, every caller renames. `tools.json` + tests + worklog move
   with it.
3. Structural parsers (tree-sitter, `.scm`, injections) before string
   scanning. Heuristics live at the edges.
4. New languages land as `runtime/queries/<lang>/tags.scm`. Rust extractor
   override only when grammar or semantics require it.
5. Keep `lib.rs` skeletal — module declarations and re-exports.
6. All SQLite through `codesight_storage::db::open_rw`/`open_ro`.
7. All logs through `tracing`. No bare `eprintln!`/`println!` in libraries.
   Subscriber → stderr; stdout is reserved for JSON-RPC.
8. Typed errors inside crates (`thiserror`); `ToolError` at the MCP
   boundary. Handlers consume
   `ValidatedArgs<T>::from_validated(v)?.into_inner()`.

## Commands

Day-to-day (targeted, always):

```bash
cargo nextest run --lib                          # crate lib tests
cargo nextest run --test <name>                  # one integration binary
cargo nextest run -p codesight-storage --lib
cargo nextest run -E 'test(my_case)'
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

`cargo nextest run` handles incremental compilation — rustc only rebuilds
changed crates.

Full suite (merge gate or sanity check):

```bash
cargo nextest run
```

