# dowser-mcp

A local MCP server for searching and reading source code. It indexes a repository into SQLite and uses tree-sitter and language-specific extractors to return symbols, references and code-analysis results over stdio.

![Actual MCP fixture results for outline, source reading and a missing reference](docs/demo/mcp.png)

The [local demo](docs/demo/README.md) runs the built stdio server against a synthetic JavaScript file. It checks symbol extraction, source reading and rejection of a missing-file reference. [Raw response envelopes](docs/demo/results.json) accompany the capture.

## Build and try

Use a stable Rust toolchain with edition 2024 support, Git, and C/C++ compilers for the native dependencies. SQLite and the static search embedding model are bundled in the build.

```sh
git clone https://github.com/olegpustovit/dowser-mcp.git
cd dowser-mcp
cargo build --locked --bin dowser-mcp
node docs/demo/run.cjs
```

Node.js is needed only for the demonstration harness. For an optimized binary, use `cargo build --locked --release --bin dowser-mcp`. Prebuilt downloads and agent-specific configuration are in the [installation reference](docs/reference.md).

## Index a repository

```sh
dowser-mcp ingest /absolute/path/to/repository
dowser-mcp serve --workspace /absolute/path/to/repository
```

Use the built binary's full path if it is not installed on your PATH. Configure your MCP client to launch the `serve` command. It uses stdio JSON-RPC rather than an HTTP port. Pre-indexing avoids doing the first ingest during a tool request.

The server exposes six tools: `search`, `outline`, `read`, `references`, `impact` and `health`. See the [tool reference](docs/reference.md#tools) and [schema](crates/dowser-engine/src/tools.json) for arguments. Each response includes confidence, warnings or error information. Treat those labels as the server's description of its evidence, not a guarantee of complete analysis.

By default, indexes live outside the source tree in the platform data directory. Set `DOWSER_INDEX_DIR` to use a separate location.

## Current limits

The source build and synthetic MCP workflow passed on Linux ARM64 with Rust 1.96.0. That check does not establish every language's reference resolution, rename safety, quality metrics or performance. Name-based references can be partial, import extraction varies by language, and indexes can become stale. Read the [limitations](docs/reference.md#limitations) before using analysis results to plan edits.

The reference preserves older release-specific examples and smoke-check claims with their original version labels. The new fixture capture does not revalidate those broader claims.

[Architecture](ARCHITECTURE.md) · [Contributing](CONTRIBUTING.md) · [Apache-2.0 license](LICENSE) · [Third-party notices](NOTICE)
