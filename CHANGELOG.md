# Changelog

## v0.2.0 — 2026-10-23

### Changed

- Renamed the project to Dowser. The package and binary are now `dowser-mcp`,
  environment variables use the `DOWSER_*` prefix, and configuration and index
  paths (`.dowser.toml`, `.dowser/`, the platform data directory `dowser-mcp`)
  and the MCP server name changed accordingly. Existing indexes must be rebuilt
  and MCP client configurations updated to the new binary and server name.

## v0.1.1 — 2026-10-22

- Capture whole JSON keys once and decode names/IDs, including escaped and empty keys, while preserving pair spans and custom scope prefixes.
- Remove deleted files and symbols when an extractor-version refresh disables fingerprint reuse.

- Propagate import resolution quality separately from extraction quality. Keep unique basename guesses in heuristic candidate lists, reject ambiguous guesses and exclude them from strict graphs; invalidate prior cached resolution on refresh.

- Add structural OCaml implementation/interface symbols; binding and import resolution remain unsupported.
- Include Dart function bodies in symbol ranges, and disclose index scope and static test-gap limits without upgrading weaker confidence.
- Stop and join background file watchers when their service is dropped.

- Upgrade rmcp to 3.6 and use its maintained nonblocking Linux pipe transport; retain upstream stdio fallback on other inputs/platforms.

- Regex search honors per-file limits, preserves matching lines and discloses truncation.
- Importers cap results and distinguish resolved import edges from heuristic usage candidates.
- Symbol references and blast radius exclude comment/string-only mentions from primary evidence; grammar identifier syntax remains explicitly name-based and partial.

## v0.1.0 — 2026-10-20

First tagged release.

### Added

- MCP stdio server with six tools: `search`, `outline`, `read`, `references`,
  `impact`, `health` (schema in `crates/dowser-engine/src/tools.json`).
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
