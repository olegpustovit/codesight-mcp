# AGENTS.md

`codesight-mcp` — a local MCP server that gives coding agents structural,
token-efficient access to any codebase. Rust workspace at the repo root.
For onboarding see [`README.md`](README.md). For active work see
[`REFACTOR_WORKLOG.md`](REFACTOR_WORKLOG.md).

---

## Product identity — how codesight is NOT prior-art

codesight-mcp is an **independent, Rust-native alternative** to prior-art-mcp.
We share the same problem space (structured code retrieval for AI agents) but
our architecture, interface philosophy, and feature depth diverge. Every change
must widen the gap, never narrow it.

### Design principles that differentiate us

| Axis | prior-art (Python) | codesight (Rust) |
|---|---|---|
| **Tool surface** | 30+ flat tools (`get_blast_radius`, `find_dead_code`, …) | **10 consolidated tools** with `view`/`analysis`/`graph` enum dispatch — fewer tool-call decisions for agents |
| **Performance** | Python + SQLite; indexing I/O-bound | Rust + WAL SQLite; sub-millisecond dispatch, <2s cold ingest |
| **Quality intelligence** | Hotspots + dead code (structural) | **Agent-readiness scoring** (navigability, comprehensibility, verifiability, change_safety) + quality regression snapshots + test gap analysis — *understands code quality, not just structure* |
| **Doc-domain awareness** | Code-only (symbol + file text) | **Dual-domain**: code *and* docs (sections, TOC, document outlines) searched together via `domain=hybrid` |
| **Graph depth** | Import graph + text heuristic calls | Import graph + AST calls + **FQN resolution** + **coupling metrics** + **layer violation enforcement** |
| **Context assembly** | `get_context_bundle` (single symbol) | `assemble_context` with **4 modes**: bundle, section_context, ranked, query_suggestions |
| **Regression detection** | None | `evaluate_quality analysis=regression_check` compares quality snapshots across sessions |

### What we must NEVER copy

- **Flat tool proliferation.** prior-art has `get_blast_radius`,
  `get_impact_preview`, `get_changed_symbols`, `get_symbol_diff` as four
  separate tools. We have one: `evaluate_impact` with `analysis=` enum.
  This is a *strength*. Never split a consolidated tool into parallel names.
- **Session-as-state.** prior-art's `plan_turn` / `get_session_context` /
  `register_edit` make the server stateful per-conversation. We stay
  **stateless per-request** — the agent owns its state. If we add session
  awareness, it's an opt-in overlay, not a mandatory protocol.
- **AI-provider coupling.** prior-art bundles Anthropic/Gemini/OpenAI
  summarizer calls inside the MCP server. We never call an LLM from inside
  codesight — the agent *is* the LLM. Summaries are the agent's job.

### What we already have that prior-art doesn't (or does worse)

- **`inspect_structure view=class_hierarchy`** — already supports class
  hierarchy traversal. Extend to trait impls and interface bounds, not rebuild.
- **`ingest_sources input_kind=single_file`** — already handles post-edit
  reindexing of a single file. prior-art's `register_edit` is equivalent.
- **`search_catalog semantic=true`** — local dense retrieval already exists
  in `crates/codesight-retrieval/src/embedder.rs` + `src/service/agent_v1/catalog_search/semantic.rs`.
  prior-art requires external API keys.
- **`run_watcher`** (`src/service/file_watcher.rs`) — built-in debounced file
  watcher with delta reindex. prior-art needs a separate `watch-claude` command.
- **`assemble_context mode=query_suggestions`** — opening-move suggestions
  already exist. Improve it, don't rebuild it.

### Where we must leapfrog (genuinely new features)

1. **`evaluate_quality analysis=plan_turn`** — upgrade `query_suggestions`
   into a full **ranked read-plan** with token estimates, confidence scores,
   and concrete `read_content` refs ready to call. Zero-step-to-code instead
   of hints. Ships as a new `analysis` enum value, not a new tool.

2. **`manage_indexes action=audit_config`** — agent config auditor. Scans
   instruction files for every major coding agent: `AGENTS.md` (open
   standard / Codex / OpenCode), `CLAUDE.md`, `.cursorrules` +
   `.cursor/rules/`, `.github/copilot-instructions.md` +
   `.github/copilot/`, `.windsurfrules` + `.windsurf/rules/`. Finds stale
   symbol refs, dead file paths, token waste. codesight is universal — not
   tied to any one agent. prior-art hardcodes `CLAUDE.md`/`.cursorrules`
   only; ours discovers all conventions.

3. **`evaluate_quality analysis=ownership`** — code ownership via git blame +
   CODEOWNERS. prior-art has nothing here. Critical for large-repo agentic
   workflows ("who do I ask about this module?").

---

## Non-negotiable policies

These override everything else. Every patch is measured against them.

### 1. Shrink the code. Always.

- **Target metric:** total LoC and per-module cyclomatic complexity only go
  down or stay flat. Track the baseline in [`REFACTOR_WORKLOG.md`](REFACTOR_WORKLOG.md);
  update it when you land a reduction.
- Reach for `simplify`, `rust-modularity`, and `rust-design-patterns` **before**
  writing a new helper. One shared helper beats three copies; a trait + small
  registry beats a growing `match`; a maintained crate beats a hand-rolled util.
- Oversized or confused modules get split, not paved over. Primary triggers:
  cyclomatic complexity > 20, multiple responsibilities, repeated internal
  branching, high fan-in/blast radius, or poor navigability for agents. Line
  count is a weak smell only — never split a module solely because it crossed
  an arbitrary LoC threshold.
- Refactor freely — **features and behavior stay, regressions don't.** No
  deletion of functionality under the guise of simplification.
- Delete useless features aggressively. If a surface is a placeholder, an
  alias of another tool, or nobody would miss it, cut it and update tests +
  `src/tools.json` in the same patch.

### 2. Dogfood `codesight-mcp` for every lookup.

We're building a retrieval engine — if we don't use it on ourselves we won't
notice when it's broken.

**Default → codesight mapping.** Every row is a *reach*: if you catch yourself
typing the left column for a lookup, stop and use the right column instead.

| Default tool (don't) | Codesight tool (do) | Args |
|---|---|---|
| `Grep` for a symbol/function/type name | `search_catalog` | `query_mode=keyword`, `unit_kinds=["symbol"]`, optional `kind=function\|class\|...` |
| `Grep` for a substring across files | `search_catalog` | `query_mode=keyword`, `unit_kinds=["file"]`, optional `file_pattern` / `scope` |
| `Grep -E` raw regex over files | `search_catalog` | `query_mode=regex`, `context_lines=N` (or `pattern=` alias for `query`) |
| `Grep pattern=X output_mode=count` | `search_catalog` | `output_mode=count` — returns `{count, files}` only |
| `Grep pattern=X \| wc -l` (plan scope) | `search_catalog` | `output_mode=count` or `output_mode=files_with_matches` |
| Fuzzy / "I'm not sure of the name" | `search_catalog` | `fuzzy=true` or `fuzzy_threshold=0.3` |
| "What's this about?" / natural question | `search_catalog` | `query_mode=natural_language`, `domain=hybrid` (hits symbols + docs) |
| `Glob` / `find` / `ls -R` tree walks | `inspect_structure` | `view=file_tree` (optional `path=` subtree) |
| Project-level "what's in this repo" | `inspect_structure` | `view=repo_outline` / `view=repository_rollup` |
| `Read` of a known file range | `read_content` | `refs=[{kind:"file", path, start_line, end_line}]` |
| `Read` to see a function/class body | `read_content` | `refs=[{kind:"symbol", path, start_line}]` (or `stable_id`) |
| Batch read N files/symbols | `read_content` | pass N items in `refs` — **one call, not N** |
| File's top-level outline | `inspect_structure` | `view=file_outline`, `file_path=...` (or batch `file_paths`) |
| "What does this file import?" | `inspect_graph` | `graph=imports`, `file_path=...` (outgoing deps) |
| "Where is X imported?" | `inspect_graph` | `graph=importers`, `file_path=...` (incoming deps) |
| "Where is symbol X referenced?" | `inspect_graph` | `graph=references`, `ref={...}` — or `find_references` with `identifier` |
| "What does this function call?" | `inspect_graph` | `graph=calls`, `symbol_id=...` |
| Cross-module deps / cycles | `inspect_graph` | `graph=dependencies` / `dependency_cycles` / `layer_violations` |
| Pre-refactor: who breaks? | `evaluate_impact` | `analysis=blast_radius`, `refs=[...]` |
| Rename safety check | `evaluate_impact` | `analysis=rename_safety`, `refs[0].new_name="..."` |
| Diff two symbol versions | `evaluate_impact` | `analysis=diff`, 2 refs in same file |
| Hotspots / complexity | `evaluate_quality` | `analysis=hotspots`, `min_complexity=20`, `top_n=20` |
| Dead code | `evaluate_quality` | `analysis=dead_code`, `granularity=symbol` |
| Repo health / doc coverage | `evaluate_quality` | `analysis=repo_health` / `doc_coverage` |
| Ticket kickoff context bundle | `assemble_context` | `mode=context_bundle`, `refs=[...]`, `max_tokens=N` |
| "Summarize this section" | `assemble_context` | `mode=section_context`, `refs=[{kind:"section",...}]` |
| Ranked suggestions from a query | `assemble_context` | `mode=ranked` or `query_suggestions` |
| Freshness / stale index | `manage_indexes` → `ingest_sources` | `action=status`; then `ingest_sources incremental=true` |
| List sources / source_id lookup | `catalog_sources` | (no args) |

### Effective use guidelines

- **Always start a session with `catalog_sources`.** It gives you the
  `source_id` every other tool needs, plus freshness/indexed_at. If stale,
  `ingest_sources path=. incremental=true` first.
- **Pick the right `query_mode`.** `keyword` for identifiers and BM25 —
  fastest, most precise. `regex` only when you truly need a pattern
  (auto-routes to file text, *ignores* `unit_kinds`). `natural_language`
  when asking a question, not naming a thing.
- **Narrow with `unit_kinds` + `scope`/`file_pattern`.** Default is
  "symbols in code". Pass `unit_kinds=["file"]` to grep file text,
  `["section"]` to hit docs, `["symbol","section"]` for hybrid.
  `scope="src/service/"` is a cheap LoC-token saver.
- **Batch reads.** `read_content` takes an array of `refs` — never make N
  sequential calls when one array call does it.
- **Prefer `inspect_graph` over manual reasoning.** If your next thought
  is "let me grep for every usage of X and see what imports it", that's
  `graph=references` or `graph=importers`. Don't simulate the graph by
  hand.
- **Token budget everything.** `max_tokens` / `token_budget` on
  `search_catalog`, `read_content`, `assemble_context`, `inspect_structure
  view=repo_outline`, and `evaluate_quality analysis=dead_code`. Default
  to `max_tokens=4000` for browsing, `8000` for deep context.
- **`_meta.tokens_saved`** is the session's scoreboard. If a call returns
  zero or negative savings, you picked the wrong tool or the wrong
  `unit_kinds`. Re-shape the query.
- **Fallback only when the tool is actually broken.** Default filesystem
  tools (`Grep`, `Read`, `Edit`, `Write`, `Bash`) remain the right choice
  for **writing** changes, running tests, and editing — not for
  read/search. Fall back to `Grep`/`Read` **only when** codesight can't
  answer (missing grammar, tool itself under repair) or you've already
  tried the codesight path and captured the gap. Every retrieval-path
  fallback is logged in [`REFACTOR_WORKLOG.md`](REFACTOR_WORKLOG.md) and
  fixed in the same session wherever practical.
- **Self-check trigger.** Before every `Grep`/`Glob`/`Read` call, ask:
  *"Which row of the table above covers this?"* If any row does, use that
  tool. If none do, log a new fallback entry in the worklog with the
  reason — that's a product bug.

### 3. Worklog-driven, test-drive-first workflow.

- [`REFACTOR_WORKLOG.md`](REFACTOR_WORKLOG.md) is the **single living state**
  file. Open items, current metrics, next-session backlog. Update it in the
  same patch as the change it describes.
- Every session starts with a test-drive against `codesight-mcp` running on
  this repo. Find bugs; fix or document them in the worklog.
- Closed items move out of the worklog (history lives in git).

### 4. Proactive improvement every session.

After the test-drive, ask: **"codesight-mcp is great, but what's *my* feature
request, and what should improve?"** Add the answer to the worklog's
improvement backlog. Key metric: **tokens saved per query** (from
`$storage_dir/_savings.json` via `check --savings`). Every change must make
the interface better — add/remove/reshape features to hyperoptimize for
token elimination, agentic-coding visibility, and speed. **Bloat is the
enemy.** Big backlog changes are welcome if they simplify or clarify.

### 5. Performance is paramount — bun-fast or bust.

Everything is local, on SSD, through SQLite. There is **no excuse** for
slow. Targets:

- Dispatch → handler response: **< 50 ms** for cached reads, **< 200 ms** for
  cold reads on a medium repo.
- Incremental reindex after a single-file edit: **< 100 ms**.
- Cold full ingest of this repo: **< 5 s**.

When you touch a hot path, verify with a targeted bench or timing print.
Regressions get rolled back the same session. Track the current numbers in
the worklog's performance section.

### 6. Tests are instant and targeted, or they're tech debt.

Tests are our **biggest technical debt** right now. Rules:

- **Targeted by default.** Run the smallest scope that proves your change —
  one test, one binary, one crate lib — via `cargo nextest run`.
- **Never** run the full suite unless the user asks or you're preparing an
  explicit merge gate.
- **Evaluate every run.** If a test takes > 1 s, note it in the worklog's
  slow-test watch list and plan a fix. We have thousands of tests; each one
  must be instant.
- Never delete or `#[ignore]` a test to silence failure. Root-cause it.
- If you discover a pre-existing failing test or clippy error, fix it in
  the same session — noticing a broken thing and walking past it is how
  this codebase rots.

---

## Layout

| Path | Purpose |
|---|---|
| [`src/`](src/) | Product crate: runtime, indexing, retrieval, MCP handlers, CLI. |
| [`src/cli/`](src/cli/) | Operator subcommands (`check`, `init`). |
| [`src/config/`](src/config/) | Layered `AppConfig` — env, TOML, defaults, trust. All `CODESIGHT_*` env names live in `src/config/env.rs`. |
| [`src/service/agent_v1/`](src/service/agent_v1/) | MCP dispatch. `validate.rs` coerces-then-validates against `tools.json`. Handlers consume `ValidatedArgs<T>`. `freshness.rs` gates read tools. |
| [`src/tools.json`](src/tools.json) | **Frozen MCP contract.** Schema changes ship with the handler change. |
| [`crates/codesight-core/`](crates/codesight-core) | Shared primitives (symbols, `FactQuality`). |
| [`crates/codesight-extractors/`](crates/codesight-extractors) | SCM engine + curated query packs. |
| [`crates/codesight-storage/`](crates/codesight-storage) | WAL-configured SQLite + forward-only migrations. |
| [`runtime/queries/`](runtime/queries) | Tree-sitter `.scm` symbol queries (30 langs). |
| [`tests/`](tests) | Integration tests. Fixtures under `tests/fixtures/`. |
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | Crate split, data flow, FactQuality, SQLite config. |
| [`CONFIGURATION.md`](CONFIGURATION.md) | Env vars, precedence, per-workspace `.codesight.toml`. |
| [`REFACTOR_WORKLOG.md`](REFACTOR_WORKLOG.md) | **Current state, open work, metrics, next-session backlog.** |

The **current** crate graph has four members. Dependencies point inward:
`codesight-core` ← `codesight-extractors` / `codesight-storage` ← root
`codesight-mcp`. The two middle crates don't depend on each other.
Product-only logic (MCP handlers, framework detection, CLI) stays in the root
crate until a split creates a cleaner ownership boundary and lower compile
surface.

New crates are allowed when they remove a coherent domain from the root crate
without introducing a shared-types dumping ground or upward dependencies.
Preferred first splits are self-contained domains with stable APIs and heavy
dependency footprints.

## Working rules

1. `agent-v1` + `src/tools.json` is the only public MCP surface. Extensibility
   goes through `view`/`analysis` enums — never parallel tool names.
2. **No backwards compatibility.** **Zero.** No shims, no aliases, no
   dual code paths, no `pub type OldName = NewName` re-exports, no
   one-line wrapper functions that forward to the new API, no
   `#[deprecated]` attributes, no "// removed" comments where deleted
   code used to live. When behavior or a name changes, the old
   identifier disappears in the same patch and every caller is renamed.
   Same rule for enum variant renames, struct field renames, module
   moves, and tool-argument renames. If you catch yourself adding a
   thin wrapper "to keep call sites readable," stop — rename the
   callers instead. `tools.json`, tests, and worklog all move in the
   same patch.
3. Structural parsers (tree-sitter, `.scm` queries, injections) before string
   scanning. Heuristics live at the edges.
4. New languages land as `runtime/queries/<lang>/tags.scm`. Reach for a Rust
   extractor override only when grammar or semantics require it.
5. Keep `lib.rs` skeletal — pure module declarations and re-exports.
6. All SQLite connections go through `codesight_storage::db::open_rw`/`open_ro`.
7. All logs go through `tracing` — no bare `eprintln!`/`println!` in libraries.
   Subscriber writes to stderr only; stdout is reserved for JSON-RPC.
8. Typed errors inside crates (`thiserror`); `ToolError` at the MCP boundary.
   Handlers consume `ValidatedArgs<T>::from_validated(v)?.into_inner()`.

## Commands

**Day-to-day (targeted, always):**

```bash
cargo nextest run --lib                          # crate lib unit tests
cargo nextest run --test <name>                  # one integration binary
cargo nextest run -p codesight-storage --lib
cargo nextest run -E 'test(my_case)'
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

`cargo nextest run` handles incremental compilation — rustc only
rebuilds changed crates. No wrapper script needed.

**Full suite (merge gate or sanity check):**

```bash
cargo nextest run                                # full suite
```
