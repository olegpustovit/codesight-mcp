# CONFIGURATION.md

Everything an operator can configure in `codesight-mcp`. For the data flow
and crate layout see [`ARCHITECTURE.md`](ARCHITECTURE.md).

## Precedence

For every setting, the first layer that sets it wins:

1. Command-line flag (where applicable)
2. Environment variable (`CODESIGHT_*`)
3. Global config file: `$XDG_CONFIG_HOME/codesight/config.toml`
   (or `~/Library/Application Support/codesight/config.toml` on macOS,
   `%APPDATA%\codesight\config.toml` on Windows)
4. Compiled-in default

A per-workspace `.codesight.toml` at the root of the indexed project
controls **workspace-local** concerns (adaptive languages, extra file
extensions, extra query-pack roots). It is loaded by the ingest flow, not
by the startup loader, because its scope is the workspace being indexed,
not the server's global lifetime.

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

Use it in deploys, smoke tests, and when diagnosing "it won't start".

## Environment variables

Canonical list. Every name lives in `src/config/env.rs` as a `const &str`;
do not scatter new `std::env::var("CODESIGHT_*")` calls in new code.

| Variable                         | Type    | Default | Meaning |
|----------------------------------|---------|---------|---------|
| `CODESIGHT_INDEX_DIR`            | path    | `~/.code-index/codesight` | Where code and doc indexes are stored. |
| `CODESIGHT_STORAGE_DIR`          | path    | (alias of `CODESIGHT_INDEX_DIR`) | Alias kept for operator convenience. |
| `CODESIGHT_AUTO_REINDEX`         | bool    | `true`  | Auto-reindex on stale detection. Set to `false` or `0` to disable. |
| `CODESIGHT_MAX_FOLDER_FILES`     | usize   | `0`     | Cap on files discovered from one folder; `0` = unlimited. |
| `CODESIGHT_CROSS_REPO_DEFAULT`   | bool    | `false` | Default for the `cross_repo` param on `inspect_graph` modes. |
| `CODESIGHT_STRICT_FRESHNESS`     | bool    | `false` | When `true`, read tools that opt in block until an in-flight reindex finishes. Otherwise they annotate the response with a stale-data warning. |
| `CODESIGHT_LOG_FORMAT`           | enum    | `pretty` | `pretty` or `json`. |
| `CODESIGHT_PATH_MAP`             | string  | *(none)* | `orig=repl;...` list for path remapping in MCP responses. |
| `CODESIGHT_GLOBAL_CONFIG`        | path    | *(none)* | Override location of the global `config.toml`. |
| `RUST_LOG`                       | string  | see below | Standard `tracing-subscriber` `EnvFilter` syntax. |

Default log filter: `warn,codesight_mcp=info,codesight_storage=info,codesight_extractors=info`.

### Bool parsing rules

Booleans coming from env vars accept `true`/`1` as true and `false`/`0`
(or empty) as false. Anything else is treated as "not set" and falls
through to the next precedence layer.

## Global config file (`config.toml`)

All keys are optional. Missing keys fall through to the env layer.

```toml
# ~/.config/codesight/config.toml
auto_reindex = true
index_dir = "/var/codesight/indexes"
max_folder_files = 50000
cross_repo_default = false
strict_freshness = false

# Path-trust policy. When whitelist mode is true (default), only paths
# inside these folders can be indexed. When false, the list is a blacklist.
trusted_folders = ["/srv/projects"]
trusted_folders_whitelist_mode = true
```

A malformed config file is logged at `WARN` and the server falls back to
defaults. `codesight-mcp check` will not catch a malformed *global*
config today; it catches malformed *project* configs.

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
`src/config/trust.rs`:

- **Whitelist mode** (default): only paths inside `trusted_folders` (or
  their descendants) are allowed. Empty list = allow any non-broad path.
- **Blacklist mode**: `trusted_folders` lists *untrusted* paths; empty
  list is an error (fail-closed).
- **Broad roots** (`/`, `/home`, `/Users`, `/tmp`, etc.) are refused
  outright unless an operator explicitly listed them. When they *are*
  explicitly trusted, a warning is attached to the response envelope.

See `src/config/trust.rs` for the full list and the policy logic.
