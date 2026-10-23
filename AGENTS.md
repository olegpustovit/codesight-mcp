# AGENTS.md

`symslice-mcp` — local MCP server for structural, token-efficient code
retrieval. Rust workspace at the repo root. Onboarding: [`README.md`](README.md).

## Identity

Three rules are non-negotiable:

- **Consolidated tools.** Six tools (`search`, `outline`, `read`, `references`,
  `impact`, `health`), dispatched by `mode` / `view` / `kind` / `analysis`
  enums. Never split a consolidated tool into parallel names. Authoritative
  schema: [`crates/symslice-engine/src/tools.json`](crates/symslice-engine/src/tools.json).
- **Stateless per-request.** The agent owns session state. Any session
  awareness we add is opt-in, never a mandatory protocol.
- **No LLM calls from inside the server.** The agent *is* the LLM; summaries
  are its job.

---

## Non-negotiable policies

### 1. Shrink the code. Always.

- LoC and per-module cyclomatic complexity only go down or stay flat. 
- Defaults come from `crates/symslice-analysis/src/thresholds.rs` — plain
  constants in `pub const THRESHOLDS: Thresholds = Thresholds { ... }`,
  surfaced by `health analysis=quality_signal`. Edit the constants
  directly if a threshold has to move.

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

### 2. Dogfood `symslice-mcp` for every lookup.

If we don't use it on ourselves we won't notice when it breaks.

**Default → symslice mapping.** Every row is a *reach*: if you catch
yourself typing the left column, stop and use the right column. The 6-tool
surface is authoritative — see [`tools.json`](crates/symslice-engine/src/tools.json).

| Default (don't) | Symslice (do) | Args |
|---|---|---|
| `Grep` a symbol/fn/type name | `search` | `mode=keyword`, opt. `scope=` / `fuzzy=true` |
| `Grep` substring across files | `search` | `mode=keyword`, opt. `scope=*.md` to narrow |
| `Grep -E` regex over files | `search` | `mode=regex`, opt. `per_file_limit=N` |
| Natural-language question | `search` | `mode=nl` |
| Unsure of the exact name | `search` | `fuzzy=true` |
| `Glob` / `find` / `ls -R` | `outline` | `view=repo` |
| "What's in this repo" | `outline` | `view=repo` |
| File outline before reading | `outline` | `view=file path=…` (batch via `paths`) |
| `Read` a known file range | `read` | `refs=["path:N-M"]` |
| `Read` a function/class body | `read` | `refs=["path::Name#kind"]` or `["id:<sid>"]` |
| Batch read N files/symbols | `read` | one call with N refs — never N calls |
| `Read` a markdown section | `read` | `refs=["section:path::Heading"]` |
| File's outgoing imports | `references` | `kind=imports path=…` |
| Incoming imports | `references` | `kind=importers path=…` |
| Where is X referenced | `references` | `kind=symbol ref="path::Name#kind"` |
| Pre-refactor: who breaks? | `impact` | `analysis=blast_radius ref=…` |
| Rename safety | `impact` | `analysis=rename_safety ref=… new_name=…` |
| Hotspots / complexity | `health` | `analysis=hotspots top_n=N` |
| Dead code | `health` | `analysis=dead_code top_n=N` |
| Per-slice agent readiness | `health` | `analysis=quality_signal scope=…` |
| Repo health summary | `health` | `analysis=repo_health` |
| Untested entry points | `health` | `analysis=test_gaps top_n=N` |

#### Effective use

- **Outline first.** `outline view=file path=<path>` before reading any
  whole file. Then `read` the specific symbols you need.
- **Batch reads.** `read` takes N refs — one call, not N. Failures are
  per-ref (`data.errors[]`), not atomic — successes still land in
  `data.items[]` with a top-level `PARTIAL_BATCH` warning.
- **Pick `mode`**: `keyword` for identifiers (fastest), `regex` for
  patterns (file-text scan), `nl` for question-shaped queries.
- **Narrow with `scope`**: glob narrowing path *and* unit kind in one knob
  (`*.md` → docs only; `crates/foo/**` → that crate). There is no separate
  `unit_kinds` / `file_pattern`.
- **`limit` is a global cap** on items returned. For regex with many
  matches per file, also pass `per_file_limit=N` if you want ripgrep
  semantics; otherwise the global cap may drop later files'
  matches and you'll see a `REGEX_OVERFLOW` warning.
- **`references` graph over mental graph walking.** "Let me grep every
  usage of X" = `references kind=symbol`. "Who imports this file" =
  `references kind=importers`.
- **Read the warnings.** `confidence: "partial"` always carries a code:
  `NO_EXACT_MATCH`, `AMBIGUOUS_IDENTIFIER`, `OVERBROAD_QUERY`,
  `NL_DEGRADED`, `TRUNCATED`, etc. Full taxonomy in
  [`warnings.rs`](crates/symslice-engine/src/service/agent_v1/warnings.rs).
- **`_meta.tokens_saved` is the scoreboard for `mode=keyword|nl`.** For
  `mode=regex`, the meta carries `bytes_scanned` + `files_scanned` instead
  — regex inherently can't beat raw grep on tokens, so the metric is
  mode-conditional.
- **Quality loop on risky edits**: `health analysis=quality_signal
  scope="…"` before; check `agent_readiness_score` and `weakest_dimension`.
- **Fallback only when symslice can't answer** (missing grammar, tool
  under repair). Native `Grep`/`Read` remain correct for writing,
  running tests, and editing — not for read/search.

### 4. Proactive improvement every session.

After the test-drive: *"what's my feature request, what should improve?"*
Add to the worklog backlog. Key metric: **tokens saved per query**
(`$storage_dir/_savings.json` via `check --savings`). Big simplifying
changes are welcome; bloat is the enemy.

### 5. Performance — local SSD SQLite, no excuses.

**Per-tool SLA tiers.** Different tools have different inherent costs;
hold each to its own bar instead of one global threshold.

| Tool / variant | Warm | Cold | Notes |
|---|---|---|---|
| `search mode=keyword` | < 20 ms | < 100 ms | FTS5 + BM25. |
| `search mode=regex` | < 30 ms | < 150 ms | File-text scan, capped by `limit` (and optional `per_file_limit`). |
| `search mode=nl` | < 100 ms | < 400 ms | Dense embedder warmup on cold; falls back to BM25 with `NL_DEGRADED` on timeout. |
| `outline view=file` | < 10 ms | < 50 ms | Single-table SELECT, function-local items filtered. |
| `outline view=repo` | < 50 ms | < 200 ms | Centrality precomputed, body prewarmed at server start. |
| `read` (symbol/section/range) | < 30 ms | < 150 ms | Per-ref ≤ 5 ms warm; per-ref errors land in `data.errors[]`. |
| `references kind=imports` / `importers` / `symbol` | < 20 ms | < 100 ms | Indexed lookups; `limit` is a global cap. |
| `impact analysis=blast_radius` | < 50 ms | < 250 ms | Reverse-BFS, depth ≤ 2. |
| `impact analysis=rename_safety` | < 100 ms | < 500 ms | File-text scan over importers. |
| `health analysis=hotspots` | < 500 ms | < 2 s | Whole-repo cyclomatic walk. |
| `health analysis=dead_code` | < 100 ms (cached) | < 5 s | 30 s in-process memo + persisted `dispatch_roots` cache. CLI/bin/main/hooks paths emit `MANUAL_REVIEW_REQUIRED`. |
| `health analysis=quality_signal` | < 500 ms (scoped) | < 1 s | Per-slice agent-readiness dimensions. |
| `health analysis=repo_health` | < 1 s | < 3 s | Aggregates the others; uses cached dead-code summary, never blocks on a cold dead-code walk. |
| `health analysis=test_gaps` | < 200 ms | < 500 ms | Untested high-risk symbols ranked by churn × cyclomatic × fan_in. |

A tool exceeding its tier in a real workload is a regression — log it and
investigate. The tier is the bar; "but it's better than before" is not.

- Legacy headline (kept for compatibility):
  Dispatch → handler: **< 50 ms** cached, **< 200 ms** cold.
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
| [`src/`](src/) | Binary crate: CLI subcommands + stdio MCP entry point. Thin wiring only — all engine logic lives in `crates/symslice-engine`. |
| [`src/cli/`](src/cli/) | Operator subcommands (`check`, `init`, hooks). |
| [`crates/symslice-engine/`](crates/symslice-engine) | **L1 engine.** MCP tool dispatch, indexing orchestration, `AppConfig`, security, server support, `tools.json` contract, stdio JSON-RPC framing (`rpc.rs`). |
| [`crates/symslice-engine/src/tools.json`](crates/symslice-engine/src/tools.json) | **Frozen MCP contract.** Schema + handler ship together. |
| [`crates/symslice-core/`](crates/symslice-core) | L0 primitives: `Symbol`, `FactQuality`, pipeline types, `LanguageId` constants. |
| [`crates/symslice-language-support/`](crates/symslice-language-support) | **L0.75.** One folder per language under `src/languages/<lang>/` — owns identity, grammar + SCM (embedded via `include_str!` from `queries/`), symbol extraction, import extraction + resolution, package discovery, entrypoints, tests, visibility, complexity, and framework providers. Single `Language` trait with default bodies; overrides are opt-in. Host profiles (openapi, ansible) are sibling `Language` impls that share the host grammar. |
| [`crates/symslice-analysis/`](crates/symslice-analysis) | Quality, impact, graph analysis (cycles, coupling, call graph, layer checks). |
| [`crates/symslice-retrieval/`](crates/symslice-retrieval) | BM25, fuzzy, embedder, section builders. |
| [`crates/symslice-storage/`](crates/symslice-storage) | WAL SQLite + forward-only migrations. |
| [`tests/`](tests) | Integration tests. `tests/crate_graph.rs` enforces layering. |
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | Crate split, data flow, FactQuality, SQLite config. |

Layer invariants (enforced by `tests/crate_graph.rs`):

- **L0** (primitives): `symslice-core` ← `-analysis` / `-retrieval` /
  `-storage`. No upward deps.
- **L0.75** (language support): `symslice-language-support` owns every
  per-language heuristic. May depend only on `symslice-core`. Never reaches
  upward into engine, analysis, retrieval, or storage.
- **L1** (engine): `symslice-engine` depends on L0 + language-support +
  analysis + retrieval + storage. Dispatch, indexing, security, and
  stdio JSON-RPC framing all live here.

## Working rules

1. `agent-v1` + `crates/symslice-engine/src/tools.json` is the only public
   MCP surface. Extensibility via `view`/`analysis` enums — never parallel
   tool names.
2. **No backwards compatibility. Zero.** No shims, aliases, dual paths,
   re-exports, wrapper fns, `#[deprecated]`, "// removed" comments.
   Renames, moves, field/arg changes: the old identifier disappears in the
   same patch, every caller renames. `tools.json` + tests + worklog move
   with it.
3. Structural parsers (tree-sitter, `.scm`, injections) before string
   scanning. Heuristics live at the edges.
4. New languages land as `crates/symslice-language-support/src/languages/<lang>/` — one folder with `mod.rs` (struct + `impl Language`), `queries/tags.scm`, and any per-language overrides (imports, extractor, packages, complexity). SCM is `include_str!`-embedded, not read from disk. Sibling profile variants (openapi, ansible) register as additional `Language` impls that share the host grammar.
5. Keep `lib.rs` skeletal — module declarations and re-exports.
6. All SQLite through `symslice_storage::db::open_rw`/`open_ro`.
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
cargo nextest run -p symslice-storage --lib
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

