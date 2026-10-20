# Changelog

## v0.1.0 — 2026-10-20

First tagged release.

### Added

- MCP stdio server with six tools: `search`, `outline`, `read`, `references`,
  `impact`, `health` (schema in `crates/codesight-engine/src/tools.json`).
- Per-response trust label `_meta.confidence` (`safe`, `partial`,
  `heuristic`, `unreliable`).
- `self-check` command that probes the trust invariants against a workspace.
- `serve --workspace PATH` for MCP clients that don't start servers from the
  project root (for example Cursor).
- Apache-2.0 `LICENSE` and `NOTICE`. CI on Linux and macOS.

### Fixed

- `impact` (`blast_radius`, `rename_safety`) with a structured ref
  (`path::Name#kind` or `id:...`) that doesn't resolve to an indexed symbol
  now returns `NOT_FOUND`. It used to fall back to a bare-name search and
  could report `confidence: safe` for a symbol that doesn't exist.
  Covered by `reliability_probe::probe_impact_unresolved_structured_ref_is_not_found`.
- The build no longer forces `/usr/bin/clang` and `lld`. The repo-level
  `.cargo/config.toml` broke Linux builds and stock macOS installs.
- Claude Code hook suggestions and the dev plugin's rules pointed at retired
  tool names (`search_catalog`, `inspect_structure`, `read_content`,
  `evaluate_quality`). They now use the six public tools.
- `tools.json` examples used `#enum` refs, but the index stores Rust enums
  as `#type`.
- The reliability-probe tests changed the process working directory without
  a lock, so `cargo test` (threads in one process) failed them randomly.
- New clippy lints (`useless_borrows_in_formatting`, plus one in the
  catalog search).

### Removed

- Planning and worklog files (`CUTPLAN.md`, `FIXUP_PLAN.md`, `PLAN.md`,
  `ROADMAP.md`), one-off scripts, scratch examples, a committed `__pycache__`,
  generated stress-test reports, a machine-specific `pack.lock`, and a
  `.cursor/agents` symlink into `/tmp`.
