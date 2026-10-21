# codesight-mcp

A local [MCP](https://modelcontextprotocol.io) server that gives coding agents
structural answers about a repository (symbols, references, imports, blast
radius) and labels each answer with how much it can be trusted.

It indexes a repository into SQLite using tree-sitter, then serves six tools
over stdio. Every response carries `_meta.confidence`. A symbol-level question
that the index can't resolve structurally gets a lower confidence label or an
error. It never gets a file-level guess presented as an answer.

- Rust workspace, 7 crates (root binary plus 6 libraries under `crates/`).
- 29 tree-sitter grammar crates. 46 language modules under
  `crates/codesight-language-support/src/languages/`; some of them (Vue, Razor,
  Blade and others) use heuristic extractors instead of a grammar.
- No network calls or LLM calls at runtime. The `mode=nl` search uses a
  vendored static embedding model (`potion-base-8M`, MIT) compiled into the
  binary.

## Build

Requirements: stable Rust with edition 2024 support, a C and C++ compiler
(`cc`/`c++`, for tree-sitter grammars and `esaxx-rs`), and `git` on `PATH`.
SQLite is bundled.

```sh
cargo install --path .        # installs ~/.cargo/bin/codesight-mcp
# or
cargo build --release         # target/release/codesight-mcp
```

On an 8-core Debian 13 box with Rust 1.99.1 and dependencies already compiled,
`cargo build --release` took 2 min 42 s (the release profile uses fat LTO with
one codegen unit). The release binary is 91 MB; 30 MB of that is the embedded
model.

## Quick start

Tools take no `source_id`. The server indexes the repository at its working
directory, and the first tool call indexes it automatically if needed. That
first call can time out on a large repository, so index ahead of time:

```sh
cd /path/to/repo
codesight-mcp ingest .        # prints the ingest envelope as JSON
codesight-mcp check --workspace .
```

Indexing this repository (494 code files, 14 doc files, 4,791 symbols) took
about 5.5 s with a debug build. The index lives under `CODESIGHT_INDEX_DIR`, or
by default the OS data directory: `~/.local/share/codesight-mcp` on Linux,
`~/Library/Application Support/codesight-mcp` on macOS. Nothing is written
into the repository.

### Claude Code

```sh
cd /path/to/repo
codesight-mcp init --claude-code       # writes/merges ./.mcp.json
# or
claude mcp add codesight -- "$(command -v codesight-mcp)" serve
```

Resulting `.mcp.json`:

```json
{
  "mcpServers": {
    "codesight": { "command": "/abs/path/to/codesight-mcp", "args": ["serve"] }
  }
}
```

Claude Code starts the server from the directory you run `claude` in. If you
start it somewhere else, add `"--workspace", "/path/to/repo"` to `args`. Optional hooks that steer Claude Code away from native
Grep/Glob are in [`plugins/codesight-hooks`](plugins/codesight-hooks/README.md).

### Codex

`~/.codex/config.toml` (or a project-scoped `.codex/config.toml`):

```toml
[mcp_servers.codesight]
command = "/abs/path/to/codesight-mcp"
args = ["serve"]
cwd = "/path/to/repo"
```

### Cursor

Cursor does not guarantee the server's working directory, so pass it
explicitly. `.cursor/mcp.json` in the repository:

```json
{
  "mcpServers": {
    "codesight": {
      "type": "stdio",
      "command": "/abs/path/to/codesight-mcp",
      "args": ["serve", "--workspace", "${workspaceFolder}"]
    }
  }
}
```

`serve --workspace PATH` changes into `PATH` before serving. Any MCP client can
use it.

## Tools

The authoritative schema is
[`crates/codesight-engine/src/tools.json`](crates/codesight-engine/src/tools.json).
Every tool returns `{ok, data, _meta: {confidence, timing_ms, tokens_saved?}, warnings}`.
On failure the shape is `{ok: false, error: {category, code, message, detail}}`.

Symbol refs are strings in the form `path::QualifiedName#kind`. Kinds come
from the index, so a Rust enum is `#type` (for example
`crates/codesight-core/src/core/symbols.rs::FactQuality#type`) and a method is
`path::Type.method#function`. To find exact ids, call `outline view=file` or
`search`.

| Tool | Required | Optional | What it returns |
|---|---|---|---|
| `search` | `query` | `mode` = `keyword` (default) \| `regex` \| `nl`; `scope` (glob); `limit` (25); `per_file_limit` (regex); `max_tokens`; `fuzzy` | Ranked symbols, doc sections, or text hits |
| `outline` | `view` = `file` \| `repo` | `path` or `paths[]` (file view); `max_tokens` (repo view) | Symbol list for a file, or a workspace summary (entrypoints, languages, modules by centrality) |
| `read` | `refs[]` | `context_lines` (0); `max_tokens`; `with_neighbors` | Content for `path:N-M`, `path:N`, `path::Name#kind`, `id:<stable_id>`, `section:path::heading` |
| `references` | `kind` = `symbol` \| `imports` \| `importers` | `ref` (symbol); `path` (imports/importers); `limit` (50); `scope`; `include_weak` | Reference sites with enclosing symbol, or import edges for a file |
| `impact` | `analysis` = `blast_radius` \| `rename_safety`; `ref` | `new_name` (rename); `limit` (50) | Files that use a symbol; rename coverage and name collisions |
| `health` | `analysis` = `quality_signal` \| `hotspots` \| `dead_code` \| `repo_health` \| `test_gaps` | `scope`; `top_n`; `min_confidence`; `include_tests`; `max_tokens` | Agent-readiness dimensions, churn × complexity hotspots, dead-code and heuristic test-gap candidates |

Regex search returns matching lines; `per_file_limit` caps each file and `limit` caps the total. Omitted lines produce `TRUNCATED`.

## Trust contract

`_meta.confidence` is one of:

| Value | Meaning | Examples |
|---|---|---|
| `safe` | Derived from index lookups or structurally resolved references | `read` of a symbol id; `references kind=imports` |
| `partial` | Incomplete or suggestive; candidates can be false positives and bindings may be unresolved. The response says why | name-based symbol references and blast radius; whole-file `read`; `search` with `NO_EXACT_MATCH` / `AMBIGUOUS_IDENTIFIER` / `OVERBROAD_QUERY`; `rename_safety` without binding resolution |
| `heuristic` | Best-effort string/regex/dispatch analysis | `health analysis=dead_code` |
| `unreliable` | No structural evidence. Primary results are empty and text mentions are only in a separate weak list | `impact blast_radius` when only text mentions exist (`weak_files_text_mentions`) |

Hard rules enforced in code and tests:

- `impact` with a structured ref (`path::Name#kind` or `id:...`) that does not
  resolve to an indexed symbol returns `ok: false` with `error.code =
  "NOT_FOUND"`. It never falls back to a bare-name search. A bare identifier
  (`"FactQuality"`) is still treated as a name search.
- `rename_safety` returns `safe: null`, `status: "undecided"`,
  `complete: false` and partial confidence. `identifier_match_pct` describes
  syntax versus text candidates; it does not establish reference completeness
  or rename safety.
- `blast_radius` returns symbol-scoped files, never file-importer counts.

Example from this repository:

```text
impact {"analysis":"blast_radius","ref":"crates/codesight-core/src/core/symbols.rs::FactQuality#type"}
→ historical v0.1.0 output: confidence=safe, symbol_scoped_file_count=10,
  weak_files_text_mentions=[crates/codesight-core/src/lib.rs, crates/codesight-language-support/src/traits.rs]

impact {"analysis":"blast_radius","ref":"no/such/file.rs::FactQuality#enum"}
→ ok=false, error.code=NOT_FOUND ("file is not in the index")

references {"kind":"importers","path":"crates/codesight-core/src/lib.rs"}
→ ok, confidence=partial, importer_count=76
```

## Self-check

`self-check` indexes a workspace and probes the trust invariants against it.
It exits non-zero if any probe fails.

```sh
codesight-mcp self-check --workspace .
```

Output on this repository for v0.1.0 (release build; `tool_latency` was 807.5 ms with a debug build):

```text
  ok    module_tree_reachability     dead_code_pct=0.00 count=0 (cap 2.00)
  ok    analysis_health              unresolved_use=0 roots=12
  ok    reference_provenance         split fields present; 0 line:0 or text_match rows across 50 results
  ok    strict_ref_not_found         unresolved structured ref → NOT_FOUND (as required)
  ok    tool_latency                 worst_timing_ms=301.6 (cap 2500)
  ok    agent_readiness              agent_readiness=0.684 (floor 0.65)
```

The probes are tuned for this repository: the dead-code cap, the readiness
floor, and the `CodeService` reference probe all assume this codebase. On
other repositories, use `check` for operator health and treat `self-check`
results as indicative only.

## Other commands

| Command | Purpose |
|---|---|
| `codesight-mcp serve [--workspace PATH]` | MCP over stdio (default when no subcommand is given) |
| `codesight-mcp ingest PATH [--domains code\|docs\|auto] [--full]` | Index a path and print the envelope |
| `codesight-mcp search SOURCE_ID QUERY [--limit N] [--domain code\|docs\|hybrid]` | Diagnostic search against an indexed source (`source_id` is printed by `ingest`) |
| `codesight-mcp check [--workspace PATH] [--savings]` | Storage, schema, query packs, `.codesight.toml`; `--savings` prints the token-savings ledger |
| `codesight-mcp init [--workspace] [--claude-code] [--force]` | Write a commented `.codesight.toml`, or a `.mcp.json` entry |
| `codesight-mcp hook-pretooluse` / `hook-posttooluse` / `hook-precompact` | Claude Code hook entry points |

Environment: `CODESIGHT_INDEX_DIR` (index location), `CODESIGHT_LOG_FORMAT`
(`pretty` or `json`; logs go to stderr), `CODESIGHT_PATH_MAP` (path-prefix
remapping for Docker or remote dev), and `RUST_LOG` (standard `EnvFilter`).

## Limitations

These were observed on this repository while preparing v0.1.0:

- **Cold first call.** If the repository isn't indexed, the first tool call
  starts indexing. In one run, `outline view=repo` hit its 5 s timeout and
  returned `TIMEOUT`, and the next `search` returned no items. Run
  `codesight-mcp ingest .` first. Timeout budgets: `read` 3 s, `search` /
  `outline` / `references` 5 s, `impact` / `health` 15 s.
- **Name-based references, not resolved bindings.** Primary reference and
  blast-radius results use definitions and grammar identifier tokens. Comments
  and string contents are weak mentions; interpolated expressions remain code.
  Aliases, shadowing and dynamic dispatch are not resolved. Unsupported or
  malformed source produces weak text evidence. These results remain partial.
- **Import extraction is uneven.** On this repository, `ingest` reported
  `languages_without_import_extractor` for al, blade, c, cpp, csharp, elixir,
  java, json, markdown, perl, php, razor, ruby, toml, and xml. Outgoing imports
  for languages without an extractor return an explicit unsupported error.
  Incoming edges from supported importers can still target those files.
- **OCaml scope.** `.ml` and `.mli` use the maintained tree-sitter OCaml
  implementation/interface grammars for structural functions, modules, types
  and function signatures. Imports, binding resolution and arbitrary PPX
  extensions are not implemented. `.rays` remains a separate unsupported DSL.
- **Test-gap evidence.** Candidates lack a recognized test-file dependency
  in the index. The count is a static proxy, not executed coverage or proof
  that tests are absent; same-package calls and indirect test protection can
  be missed.
- **Index coverage.** Repository outlines and health describe indexed files,
  with `INDEX_SCOPE_ONLY`, partial or weaker confidence and a built-in language
  registry list. Unsupported, ignored and unindexed files can be absent; no omitted-file
  count is inferred. Import graph requests reject unindexed targets. Failed read
  references appear in `data.errors[]` with `PARTIAL_BATCH`.
- **Importer evidence.** `importers` contains resolved import edges;
  `candidate_usage_importers` separately retains heuristic name-usage candidates,
  which can include text mentions. Both local lists share the requested cap,
  expose their unbounded totals and report truncation. Missing import extraction
  still makes the result incomplete.
- **Stale index.** Reopened preindexed workspaces activate background watching.
  Overlapping in-flight reindexes immediately emit `STALE_DATA_WARNING` and partial
  confidence. Symbol reads verify disk contents and reject drifted offsets with
  `STALE_ID` until reindex completes; retry the read. This check reads the source
  file and adds I/O compared with an unchecked cached body.

- **Platforms.** These changes were built and tested on Linux arm64.
  macOS and Windows runtime behavior was not verified in this validation.

## Repository layout

See [`ARCHITECTURE.md`](ARCHITECTURE.md) for crate boundaries and data flow,
[`CONTRIBUTING.md`](CONTRIBUTING.md) for the dev loop, and
[`AGENTS.md`](AGENTS.md) for the conventions coding agents follow in this repo.

## License

Apache-2.0. See [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE).
