# AGENTS.md

This repository is the `codesight-mcp` Rust crate at repo root.

## Layout

| Path | Purpose |
|------|---------|
| [`Cargo.toml`](Cargo.toml) | Product crate manifest. |
| [`agentpack.toml`](agentpack.toml) | Agentpack manifest pointing at this crate. |
| [`src/`](src/) | Runtime, indexing, retrieval, extractors, MCP handlers. |
| [`runtime/queries/`](runtime/queries/) | Tree-sitter `.scm` symbol queries. |
| Workspace config | Optional repo-root `.codesight.toml` (TOML) for settings such as adaptive `languages` / `languages_adaptive`. |
| [`tests/`](tests/) | Integration tests. Checked-in fixtures: [`tests/fixtures/doc_reference/`](tests/fixtures/doc_reference/), [`tests/fixtures/code_reference/`](tests/fixtures/code_reference/), [`tests/fixtures/lang_samples/`](tests/fixtures/lang_samples/). |

## Tests (non-negotiable)

- **Do not lose tests.** Refactors and simplifications must keep the suite meaningful: no deleting tests to silence failures, no broad `#[ignore]` to hide breakage, no shrinking fixtures or assertions without an intentional contract change.
- When behavior changes on purpose, update tests and docs together so they still encode the product contract.
- New work adds or extends tests when it adds behavior; “cleanup” is not an excuse to drop coverage.
- Treat test wall-clock time as a product constraint. Default new coverage to the smallest layer that proves the behavior; end-to-end `agent-v1` and full-ingest tests should guard contracts, not absorb every internal case.

## Strategic focus (refactor and direction)

- **Goal:** A **simple, versatile, modular, high-performance** codebase that is **extensible and universal across arbitrary codebases**—not a pile of one-off branches for specific repos, frameworks, or fixtures.
- The active structural refactor program is tracked in [`REFACTOR_PLAN.md`](REFACTOR_PLAN.md). Treat it as the current working document for modularity, heuristic removal, and sequencing.
- If a change materially affects module boundaries, fallback heuristics, or refactor sequencing, update [`REFACTOR_PLAN.md`](REFACTOR_PLAN.md) in the same patch.
- Prefer **general mechanisms** (clear extension points, composable pipelines, structural parsing) over **edge-case coupling** in core paths. Special cases belong at the edges (language-specific extractors, query packs), not scattered through shared logic.
- Keep hot paths **allocation-conscious and predictable**; avoid accidental O(n²) or repeated I/O when designing abstractions.
- Keep the test loop practical. Prefer layered unit/slice tests for domain behavior and reserve full service/MCP indexing tests for public-contract coverage.
- **`agent-v1` and [`src/tools.json`](src/tools.json) remain the contract:** extensibility must not fragment the public MCP surface.

## Working rules

1. Keep the public MCP surface coherent. The live contract is defined by [`src/tools.json`](src/tools.json) and enforced by the Rust tests.
2. Prefer structural parsers and reusable abstractions over ad-hoc glue.
3. Do not add duplicate tool names, hidden routes, or parallel public MCP surfaces. `agent-v1` is the only supported API.
4. Keep tests self-owned inside this crate. New fixtures belong under [`tests/fixtures/`](tests/fixtures/), not in external oracle trees.
5. **No backwards compatibility.** Do not add shims, aliases, feature flags, or dual code paths to keep obsolete clients working. When behavior changes, update [`src/tools.json`](src/tools.json) and tests to the new contract—never paper over breaks in core or the MCP surface.

## Modularity guidance

- Organize by domain at the top level: extractor engine, framework detection, graph/impact analysis, retrieval/service wiring.
- Keep `lib.rs` skeletal and push logic into named modules with **clear dependency direction** (core types inward, adapters at the boundary).
- When a behavior family is open-ended, prefer **traits and small registries** over growing central `match` trees.
- Add new language support through `runtime/queries/{lang}/tags.scm` when possible. Reach for Rust extractor overrides only when the grammar or semantics require it.
- Workspace crates are allowed once a boundary is stable and the split reduces compile/test blast radius without creating a catch-all shared `types` crate.

## Commands

### Quality gates

Before merging or declaring work complete, both of the following must pass (treat them as non-negotiable CI/agent gates):

1. **Clippy (warnings denied):** `cargo clippy --all-targets -- -D warnings`
2. **Tests:** `./scripts/nextest-cache` (full suite). For a merge-style check that must not trust a cache skip, run `./scripts/nextest-cache --no-cache` once after meaningful changes (or `./scripts/nextest-cache clean` then `./scripts/nextest-cache`).

Formatting is expected to be clean: `cargo fmt --all -- --check` (or run `cargo fmt --all` before commit).

### Day-to-day commands

Use **`./scripts/nextest-cache`** for all test runs. It wraps **[cargo-nextest](https://nexte.st/)** (`cargo install cargo-nextest` or your OS package manager) and skips repeated work when inputs and args match a prior successful run—**do not** invoke `cargo nextest run` directly from agents or local loops; it is unnecessarily slow compared to the wrapper.

For local refactor loops, use focused runs through the wrapper where practical, then finish with the full suite.

```bash
./scripts/nextest-cache
./scripts/nextest-cache --lib
./scripts/nextest-cache --test <name>
./scripts/nextest-cache --no-cache          # force a real nextest run (same as uncached)
./scripts/nextest-cache clean               # clear wrapper cache
cargo fmt --all
cargo clippy --all-targets -- -D warnings
```

Use plain `cargo test` only when nextest is unavailable in an environment, and prefer restoring nextest there.

## Cleanup standard

Delete documents that only narrate obsolete workflows. Keep the repo focused on code, tests, and the minimum docs needed to ship and maintain the product.
