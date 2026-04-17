# CONFIGURATION.md

Everything an operator can configure in `codesight-mcp`. For the data flow
and crate layout see [`ARCHITECTURE.md`](ARCHITECTURE.md).

## Philosophy

Runtime behavior is **defaults-only**. There is no global TOML, no
precedence chain, no hypothetical tuning knobs. Three environment
variables cover every real operator scenario; everything else is a
compile-time constant in `src/config/defaults.rs`. If you want to change
a threshold, edit the const and recompile.

Per-workspace `.codesight.toml` at the root of the indexed project is a
different thing: it carries workspace-local concerns (adaptive languages,
extra file extensions, extra query-pack roots) and is loaded by the
ingest flow, not by the startup loader.

## Command-line flags

```
codesight-mcp [OPTIONS] [COMMAND]

OPTIONS:
  --log-format <pretty|json>   Log output format on stderr.
                               Default: pretty.   Env: CODESIGHT_LOG_FORMAT

SUBCOMMANDS:
  serve                        Serve the MCP protocol over stdio (default).
  check [--workspace PATH]     Run the operator health check and exit.
```

### `codesight-mcp check`

Runs a fixed pipeline and prints a human-readable report. Exits `0` if
every check passes, `1` on the first failure.

Checks performed:

- **storage_dir** — index root is creatable and writable.
- **schema_version** — the target `PRAGMA user_version` is reachable.
- **query_packs** — curated query-pack catalog loads cleanly.
- **workspace** — (when `--workspace` given) is a readable directory.
- **project_config** — (when `--workspace` given) `.codesight.toml` parses
  if present.

## Environment variables

Canonical list. Every name lives in `src/config/env.rs`. Do not scatter
new `std::env::var("CODESIGHT_*")` calls in new code.

| Variable              | Type    | Default                    | Meaning |
|-----------------------|---------|----------------------------|---------|
| `CODESIGHT_INDEX_DIR` | path    | `~/.code-index/codesight`  | Where code and doc indexes are stored. |
| `CODESIGHT_LOG_FORMAT`| enum    | `pretty`                   | `pretty` or `json`. |
| `CODESIGHT_PATH_MAP`  | string  | *(none)*                   | `orig=repl,...` list for path remapping in MCP responses (Docker / remote-dev workflows). |
| `RUST_LOG`            | string  | see below                  | Standard `tracing-subscriber` `EnvFilter` syntax. |

Default log filter: `warn,codesight_mcp=info,codesight_storage=info,codesight_extractors=info`.

## Compiled-in defaults

These used to be environment-tunable; they are now constants in
`src/config/defaults.rs`. Change them by editing the source and
recompiling.

| Constant                  | Value    | Meaning |
|---------------------------|----------|---------|
| `HANDLER_TIMEOUT_MS`      | `2_000`  | Per-handler wall-clock budget. Exceeding it yields a `TIMEOUT` error. |
| `SHUTDOWN_DRAIN_SECS`     | `5`      | Shutdown-drain wait before aborting in-flight dispatches. |
| `DISPATCH_CONCURRENCY`    | `4`      | In-flight `tools/call` dispatches. |
| `TOKEN_HEAVY_READ_BYTES`  | `4_096`  | Byte threshold for the `TOKEN_HEAVY_READ` warning in `read_content`. |

## Per-workspace `.codesight.toml`

Loaded from the root of the indexed workspace by the ingest flow.

```toml
# <repo>/.codesight.toml

# Adaptive language detection: if true, codesight rewrites the `languages`
# block below to match what it actually finds in the repo.
languages_adaptive = true

# The active language set. When `languages_adaptive = true` this is
# managed by codesight; when false it is the operator-curated set.
languages = ["rust", "typescript", "python"]

# Optional: extra file extensions to treat as belonging to a language.
[extra_extensions]
"sqlx" = "sql"

# Optional: additional directories to scan for `tags.scm` / `injections.scm`
# query packs. Useful when you want per-repo overrides.
query_pack_roots = [".codesight/queries"]
```

The narrow extractor-facing schema (`extra_extensions`, `query_pack_roots`)
is owned by `codesight-extractors`. The product crate adds
`languages_adaptive` and `languages` on top.

## Path trust

Before any indexing, the path is checked against the trust policy in
`src/config/trust.rs`. `AppConfig.trusted_folders` is a whitelist:

- Non-empty: only paths inside the list (or their descendants) may be indexed.
- Empty (default): any non-broad path is allowed.
- Broad roots (`/`, `/home`, `/Users`, `/tmp`, etc.) are refused outright
  unless explicitly listed. When explicitly trusted, a warning is attached
  to the response envelope.

`trusted_folders` is populated programmatically (via `AppConfig::new`
callers) or from a per-workspace `.codesight.toml` section when indexing.

---

## Warning Codes

Warnings appear in the `warnings` array of any tool response. Each warning
has a `code`, `message`, and optional `detail` object.

| Code | Tool | Trigger | Recovery |
|------|------|---------|----------|
| `TOKEN_HEAVY_READ` | `read_content` | Whole-file read exceeds `TOKEN_HEAVY_READ_BYTES` (4 KB) | Use `inspect_structure view=file_outline`, symbol reads, or bounded file ranges |
| `STALE_DATA_WARNING` | all read tools | Index is stale relative to on-disk files | Run `ingest_sources incremental=true` |
| `SEMANTIC_FALLBACK` | `search_catalog` | Dense retrieval unavailable or failed | Results are BM25-only; check embedding model |
| `CALL_HEURISTIC` | `inspect_graph` | Call graph used text heuristics instead of AST | Results may include false positives |
| `REGRESSION_DETECTED` | `evaluate_quality` | Quality regression vs. snapshot | Review regressed symbols |

---

## Hook Environment Variables

These variables configure the Claude Code hook subcommands
(`hook-pretooluse`, `hook-posttooluse`, `hook-precompact`).

| Env var | Default | Effect |
|---------|---------|--------|
| `CODESIGHT_HOOK_MIN_SIZE` | `4096` | Files below this size pass through the Read hook silently |
| `CODESIGHT_HOOK_BLOCK_READS` | `0` | `1` = deny (block) instead of warn |
| `CODESIGHT_HOOK_ALLOW_EDIT_READS` | `1` | `1` = allow targeted reads with offset+limit (edit prep) |
