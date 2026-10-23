# Contributing

## Toolchain

Stable Rust (edition 2024), a C/C++ compiler, and `git`. On Debian/Ubuntu,
`apt install build-essential` is enough. On macOS, the Xcode command-line
tools are enough. The repository does not pin a linker. If you want a faster
linker locally, set it in your own `~/.cargo/config.toml`, not in the repo.

## Dev loop

```sh
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo run -- self-check --workspace .
```

CI (`.github/workflows/ci.yml`) runs the same commands on Linux and macOS.

Integration tests are compiled into a single `integration` binary
(`tests/all.rs`). Run one file with, for example,
`cargo test --test integration reliability_probe`.

## Rules that tests enforce

- `crates/dowser-engine/src/tools.json` is the public contract. If you
  change a tool's schema, change the adapter in `service/agent_v1/cutplan.rs`
  and its tests in the same commit.
- `tests/routing_drift.rs` fails if `AGENTS.md`, `README.md`, or the routing
  rule in `plugins/dowser-mcp` mention a retired tool name, a
  `mode=`/`view=`/`kind=`/`analysis=` value that isn't in `tools.json`, or an
  undeclared warning code.
- `tests/crate_graph.rs` enforces the crate dependency rules described in
  `ARCHITECTURE.md`.

See `AGENTS.md` for the longer list of conventions that coding agents follow
in this repository.
