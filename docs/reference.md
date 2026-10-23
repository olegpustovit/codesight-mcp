# Installation and tool reference

## Install a prebuilt release

Linux and macOS users do not need Rust. Download the pinned installer, then run
it to install **v0.2.0** in `~/.local/bin`:

```sh
curl -fL https://github.com/olegpustovit/dowser-mcp/releases/download/v0.2.0/install.sh -o /tmp/dowser-install.sh
sh /tmp/dowser-install.sh v0.2.0
"$HOME/.local/bin/dowser-mcp" --version
```

The installer detects your OS and architecture, downloads the matching archive,
verifies its SHA-256 checksum, and checks the binary version before replacing an
existing installation. It requires `curl`, `tar`, and `sha256sum` or `shasum`.
Set `DOWSER_INSTALL_DIR=/absolute/directory` to choose another location.
Add `export PATH="$HOME/.local/bin:$PATH"` to your shell profile for CLI use;
the client examples below use the absolute path and do not depend on `PATH`.

| Platform | Release archive suffix | Compatibility |
|---|---|---|
| Linux x86_64 | `x86_64-unknown-linux-gnu` | Ubuntu 22.04 baseline, glibc 2.35 or newer, standard C/C++ runtime libraries |
| Linux arm64 | `aarch64-unknown-linux-gnu` | Ubuntu 22.04 baseline, glibc 2.35 or newer, standard C/C++ runtime libraries |
| macOS Intel | `x86_64-apple-darwin` | Built and smoke-tested on macOS 15 |
| macOS Apple Silicon | `aarch64-apple-darwin` | Built and smoke-tested on macOS 15 |

macOS binaries are unsigned and not notarized. Earlier macOS versions and
Windows are not supported by these prebuilt releases. Alpine Linux uses musl
and cannot run these glibc builds directly.

### Manual download and updates

All archives, per-file checksums and `SHA256SUMS` are on the
[v0.2.0 release page](https://github.com/olegpustovit/dowser-mcp/releases/tag/v0.2.0).
For example, on Linux x86_64:

```sh
archive=dowser-mcp-v0.2.0-x86_64-unknown-linux-gnu.tar.gz
base=https://github.com/olegpustovit/dowser-mcp/releases/download/v0.2.0
curl -fLO "$base/$archive"
curl -fLO "$base/$archive.sha256"
sha256sum -c "$archive.sha256"   # macOS: shasum -a 256 -c "$archive.sha256"
tar -xzf "$archive" dowser-mcp
mkdir -p "$HOME/.local/bin"
install -m 755 dowser-mcp "$HOME/.local/bin/dowser-mcp"
```

There is no automatic update. To update, download the installer from the desired
release and run it with that same version. Running the commands above again
reinstalls v0.2.0. Restart your MCP client after an update. Archives include the
license, third-party notices and `BUILD-INFO.txt` with the exact source commit.

## Add Dowser to your coding agent

Choose the repository you want the agent to work on and index it first. Replace
`/absolute/path/to/your/repo` below. The server keeps its SQLite index outside
your repository and makes no network or model API calls.

```sh
cd /absolute/path/to/your/repo
"$HOME/.local/bin/dowser-mcp" ingest "$PWD"
"$HOME/.local/bin/dowser-mcp" check --workspace "$PWD"
```

The first tool call can index automatically, but pre-indexing avoids a cold-call
timeout on a large repository. By default the index is under
`~/.local/share/dowser-mcp` on Linux and
`~/Library/Application Support/dowser-mcp` on macOS;
`DOWSER_INDEX_DIR` overrides that location.

### Claude Code

Run this from the target repository:

```sh
claude mcp add --transport stdio --scope local dowser -- \
  "$HOME/.local/bin/dowser-mcp" serve --workspace "$PWD"
claude mcp get dowser
```

Restart Claude Code and check `/mcp`. The server is configured for this
repository. See [Claude Code's MCP documentation](https://code.claude.com/docs/en/mcp).
Optional routing hooks are in
[`plugins/dowser-hooks`](../plugins/dowser-hooks/README.md).

### Codex

Run this from the target repository:

```sh
codex mcp add dowser -- "$HOME/.local/bin/dowser-mcp" serve --workspace "$PWD"
codex mcp list
```

This adds a server to Codex's user configuration, pinned to this repository.
Use a different server name for each repository if you keep several configured.
Restart Codex after adding it. You can instead add this to the repository's
trusted `.codex/config.toml`, replacing both absolute paths:

```toml
[mcp_servers.dowser]
command = "/absolute/path/to/.local/bin/dowser-mcp"
args = ["serve", "--workspace", "/absolute/path/to/your/repo"]
```

See [Codex's MCP documentation](https://learn.chatgpt.com/docs/extend/mcp?surface=cli).

### Cursor

Create `.cursor/mcp.json` in the target repository. Replace the executable path
with the absolute path printed by the installer:

```json
{
  "mcpServers": {
    "dowser": {
      "command": "/absolute/path/to/.local/bin/dowser-mcp",
      "args": ["serve", "--workspace", "${workspaceFolder}"]
    }
  }
}
```

Restart Cursor, then check the server under MCP settings. See
[Cursor's MCP documentation](https://cursor.com/docs/mcp).
`serve --workspace PATH` makes the workspace explicit for any MCP client.

## Build from source

Requirements: stable Rust with edition 2024 support, a C and C++ compiler
(`cc`/`c++`, for tree-sitter grammars and `esaxx-rs`), and `git` on `PATH`.
SQLite is bundled. From a checkout of this repository:

```sh
cargo install --locked --path .   # installs ~/.cargo/bin/dowser-mcp
# or
cargo build --locked --release --bin dowser-mcp
```

The release profile uses LTO with one codegen unit. No profile overrides are
applied to the prebuilt releases. Native release builds run in GitHub Actions;
each binary must pass version, fresh-index, MCP handshake, six-tool discovery,
outline/read content and installer checks before publication.

## Paper and reproduction package

The [SANER 2027 Tool Demo paper and artifact release](https://github.com/olegpustovit/dowser-mcp/releases/tag/saner-2027-artifact-v1)
contains the paper PDF, LaTeX manuscript source archive,
reproduction archive and checksums. GitHub’s automatic **Source code** downloads
on that release contain the measured software snapshot. This is a submission
candidate, with no claim of submission or acceptance. The reported study uses revision `5d0f620adda96e2d39b2259e188a48e318ad4208`; later fixes, v0.1.1 and v0.2.0
have not been reevaluated in that study. Use that release’s automatic software source download to
reproduce the paper and v0.2.0 for the current installable tool.

## Tools

The authoritative schema is
[`crates/dowser-engine/src/tools.json`](../crates/dowser-engine/src/tools.json).
Every tool returns `{ok, data, _meta: {confidence, timing_ms, tokens_saved?}, warnings}`.
On failure the shape is `{ok: false, error: {category, code, message, detail}}`.

Symbol refs are strings in the form `path::QualifiedName#kind`. Kinds come
from the index, so a Rust enum is `#type` (for example
`crates/dowser-core/src/core/symbols.rs::FactQuality#type`) and a method is
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
impact {"analysis":"blast_radius","ref":"crates/dowser-core/src/core/symbols.rs::FactQuality#type"}
→ historical v0.1.0 output: confidence=safe, symbol_scoped_file_count=10,
  weak_files_text_mentions=[crates/dowser-core/src/lib.rs, crates/dowser-language-support/src/traits.rs]

impact {"analysis":"blast_radius","ref":"no/such/file.rs::FactQuality#enum"}
→ ok=false, error.code=NOT_FOUND ("file is not in the index")

references {"kind":"importers","path":"crates/dowser-core/src/lib.rs"}
→ ok, confidence=partial, importer_count=76
```

## Self-check

`self-check` indexes a workspace and probes the trust invariants against it.
It exits non-zero if any probe fails.

```sh
dowser-mcp self-check --workspace .
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

That historical `analysis_health` line used a mislabeled heuristic-edge counter;
it did not measure unresolved Rust uses. The current `import_evidence_integrity`
probe instead checks nonzero roots and zero heuristic edges in the strict graph.
It reports stored `heuristic_import_edges` and explicitly marks
`unresolved_import_coverage` as `not_measured`; a pass establishes evidence
separation, not import-resolution completeness.

The probes are tuned for this repository: the dead-code cap, the readiness
floor, and the `CodeService` reference probe all assume this codebase. On
other repositories, use `check` for operator health and treat `self-check`
results as indicative only.

## Other commands

| Command | Purpose |
|---|---|
| `dowser-mcp serve [--workspace PATH]` | MCP over stdio (default when no subcommand is given) |
| `dowser-mcp ingest PATH [--domains code\|docs\|auto] [--full]` | Index a path and print the envelope |
| `dowser-mcp search SOURCE_ID QUERY [--limit N] [--domain code\|docs\|hybrid]` | Diagnostic search against an indexed source (`source_id` is printed by `ingest`) |
| `dowser-mcp check [--workspace PATH] [--savings]` | Storage, schema, query packs, `.dowser.toml`; `--savings` prints the token-savings ledger |
| `dowser-mcp init [--workspace] [--claude-code] [--force]` | Write a commented `.dowser.toml`, or a `.mcp.json` entry |
| `dowser-mcp hook-pretooluse` / `hook-posttooluse` / `hook-precompact` | Claude Code hook entry points |

Environment: `DOWSER_INDEX_DIR` (index location), `DOWSER_LOG_FORMAT`
(`pretty` or `json`; logs go to stderr), `DOWSER_PATH_MAP` (path-prefix
remapping for Docker or remote dev), and `RUST_LOG` (standard `EnvFilter`).

## Limitations

These were observed on this repository while preparing v0.1.0:

- **Cold first call.** If the repository isn't indexed, the first tool call
  starts indexing. In one run, `outline view=repo` hit its 5 s timeout and
  returned `TIMEOUT`, and the next `search` returned no items. Run
  `dowser-mcp ingest .` first. Timeout budgets: `read` 3 s, `search` /
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
- **Import evidence.** `imports` and `importers` contain structural import
  resolution. `candidate_imports` retains unique basename guesses;
  `candidate_usage_importers` retains heuristic resolution and name-usage
  candidates, including text mentions. Each query shares its requested cap
  between structural and candidate lists, exposes totals and reports truncation.
  Duplicate basename guesses remain unresolved. These bounded local graphs have
  partial confidence and `complete: false`; an empty list does not prove that
  no dependency exists. Strict analyses exclude heuristic resolution.
  `has_imports` describes the structural list only; check `candidate_imports`
  and `complete` before interpreting an empty structural list.
- **Stale index.** Reopened preindexed workspaces activate background watching.
  Overlapping in-flight reindexes immediately emit `STALE_DATA_WARNING` and partial
  confidence. Symbol reads verify disk contents and reject drifted offsets with
  `STALE_ID` until reindex completes; retry the read. This check reads the source
  file and adds I/O compared with an unchecked cached body.

- **Platforms.** The older validation described above ran on Linux arm64.
  Native release smoke checks cover the four platforms in the installation
  table; those smoke checks do not replace the paper study or establish
  complete behavior on every platform.

## Repository layout

See [`ARCHITECTURE.md`](../ARCHITECTURE.md) for crate boundaries and data flow,
[`CONTRIBUTING.md`](../CONTRIBUTING.md) for the dev loop, and
[`AGENTS.md`](../AGENTS.md) for the conventions coding agents follow in this repo.

## License

Apache-2.0. See [`LICENSE`](../LICENSE) and [`NOTICE`](../NOTICE).
