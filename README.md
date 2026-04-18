# codesight-mcp

A local MCP (Model Context Protocol) server that gives coding agents structural, token-efficient access to any codebase. It indexes symbols, imports, docs, and dependency graphs into a WAL-configured SQLite database and exposes them through the consolidated `agent-v1` tool surface.

The design goal is simple: when your agent can answer a code question by asking codesight-mcp instead of pulling the whole file through `Read`, it saves tokens. This repo is the Rust implementation — 30+ languages via tree-sitter query packs, an eight-crate workspace (`codesight-core`, `codesight-extractors`, `codesight-storage`, `codesight-analysis`, `codesight-framework`, `codesight-imports`, `codesight-retrieval`, and the root `codesight-mcp` crate), and a consolidated MCP surface for ingest, catalog search, structure, graph, impact, quality, context, and index management.

## Install

```sh
cargo install --path .
```

The binary `codesight-mcp` lands in `~/.cargo/bin/codesight-mcp`. Pre-built release artifacts are a follow-up; for now this builds from source in about 90 seconds on a modern laptop.

Requirements: stable Rust toolchain, SQLite (bundled via `rusqlite`), and `git` in `$PATH` (used for commit-diff impact analyses).

## Wire into Claude Code

The fastest path is the built-in init helper — run it from the repo you want codesight-mcp to index:

```sh
codesight-mcp init --claude-code
```

That writes (or merges into) a `.mcp.json` at the current workspace root with a `codesight` server entry pointing at your binary. Claude Code picks it up automatically the next time it starts in that directory.

If you'd rather register it globally, the equivalent CLI is:

```sh
claude mcp add codesight "$(which codesight-mcp)"
```

For manual wiring (or if `claude mcp add` isn't available in your environment), drop the following into `.mcp.json`:

```json
{
  "mcpServers": {
    "codesight": {
      "command": "/absolute/path/to/codesight-mcp",
      "args": ["serve"],
      "env": {
        "CODESIGHT_INDEX_DIR": "/absolute/path/to/index-storage",
        "CODESIGHT_LOG_FORMAT": "pretty"
      }
    }
  }
}
```

`CODESIGHT_INDEX_DIR` is optional — it defaults to the OS-native application data directory (`~/Library/Application Support/codesight-mcp` on macOS, `$XDG_DATA_HOME/codesight-mcp` or `~/.local/share/codesight-mcp` on Linux, `%APPDATA%\codesight-mcp` on Windows). `CODESIGHT_LOG_FORMAT=json` switches stderr output to structured JSON for log aggregators. These two plus `CODESIGHT_PATH_MAP` (Docker / remote-dev path remapping) are the only env knobs; everything else is a compiled-in default. See [`CONFIGURATION.md`](CONFIGURATION.md).

## First run

From the workspace root you want to index:

```sh
codesight-mcp init --workspace       # optional: writes a commented .codesight.toml
codesight-mcp check --workspace .    # validates storage, migrations, query packs, workspace config
```

Then, from your Claude Code session in that workspace, ask the agent to index the repo. Under the hood it calls the `ingest_sources` tool:

```
Call codesight ingest_sources { "path": "." }
```

Once indexing finishes (usually a few seconds for medium repos, longer for monorepos), the other tools become useful:

```
Call codesight search_catalog { "source_id": "<repo>", "query": "dispatch", "limit": 10 }
Call codesight inspect_structure { "source_id": "<repo>", "view": "file_outline", "path": "src/main.rs" }
```

The response carries `timing_ms`, `tokens_saved` telemetry, and the raw result set. Current envelope work and follow-on changes are tracked in [`REFACTOR_WORKLOG.md`](REFACTOR_WORKLOG.md).

## Where state lives

SQLite databases, embedding stores, and the `_savings.json` ledger all live under `CODESIGHT_INDEX_DIR`. The `check` subcommand inspects and reports on every piece of it; see [`CONFIGURATION.md`](CONFIGURATION.md) for the env knobs and per-workspace `.codesight.toml` overrides.

## When to reach for codesight-mcp vs Read / Grep / Glob

**Reach for codesight-mcp when** you're asking a *structural* question: "where is this symbol defined / called / imported?", "what breaks if I rename `Foo`?", "give me an outline of `crates/codesight-storage`", "search the catalog for handlers that mention 'envelope'", "show me the dependency cycle between these modules". Those are the calls that replace expensive full-file reads with small, targeted responses and save the most tokens.

**Reach for Read / Grep / Glob when** you need the exact bytes of a specific file you already know about, you're editing in a client that insists on its own native-read acknowledgement, or you're looking for a string that isn't a symbol or declaration. codesight-mcp should cover the exploration and targeted-retrieval phase; client-native reads are a harness quirk, not the discovery path.

To get the token savings to actually land, add a short instruction to your workspace `CLAUDE.md` (or `~/.claude/CLAUDE.md`) so the agent prefers codesight-mcp tools by default:

```md
## Using codesight-mcp

This workspace has codesight-mcp wired up (see `.mcp.json`). Prefer its tools
over `Read` / `Grep` / `Glob` for code exploration once the index is built:

- `search_catalog` for keyword and fuzzy symbol/file lookup.
- `search_catalog include_body=true` when you want the symbol/section body in the same response.
- `search_catalog exclude=[...]` to suppress plan/generated/vendor noise without over-tightening `scope`.
- `inspect_structure view=file_outline` instead of `Read` for "what's in this file?".
- `inspect_graph graph=references` or `inspect_graph graph=importers` instead of grepping for callers.
- `evaluate_impact analysis=blast_radius|rename_safety` before risky refactors.

Run `ingest_sources path=.` once per workspace; rerun after large changes.
```

## Further reading

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — crate split, data flow, FactQuality taxonomy, dependency direction.
- [`CONFIGURATION.md`](CONFIGURATION.md) — every `CODESIGHT_*` env var, precedence chain, `check` subcommand output.
- [`REFACTOR_WORKLOG.md`](REFACTOR_WORKLOG.md) — current state: open workstreams, metrics, test-drive issues, improvement backlog.
- [`AGENTS.md`](AGENTS.md) — repo conventions, test layering, day-to-day commands.
