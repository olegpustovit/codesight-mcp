# REFACTOR_PLAN.md

This is the active refactor program for `codesight-mcp`. Treat it as a living document.

Purpose:
- drive the crate toward a domain-component architecture with clear dependency direction
- replace ad-hoc content heuristics with tree-sitter or query-derived structure wherever practical
- keep `agent-v1` and `src/tools.json` as the only public MCP contract
- sequence work so we can refactor aggressively without losing test coverage

## Working stance

- This crate stays a single crate for now.
- We are not preserving internal backwards compatibility.
- Public MCP behavior changes only when explicitly called out and then updated in tests and `src/tools.json`.
- File-system conventions are acceptable when the framework itself defines semantics by path or filename.
- Content-derived semantics should come from tree-sitter ASTs, `.scm` queries, or explicit structural adapters before we reach for string scanning.
- If a heuristic fallback remains, it must be isolated, documented, and covered by tests.

## Architecture choice

Decision tree:
- Independently publishable crates: no. The current modules are tightly coupled around one product surface.
- Pure phase pipeline: no. The crate has distinct long-lived domains beyond a linear ingest pipeline.
- Distinct domains with their own types and logic: yes.

Decision:
- Use `rust-modularity` Pattern A: domain components, with internal layering where needed.

Why this fits:
- `service` is a boundary/facade domain.
- `extractors` and `framework` are separate extension domains.
- `storage`, `doc_storage`, `db`, and caches are persistence concerns.
- `dep_graph`, `class_hierarchy`, `complexity`, `security`, and `cross_repo` are analysis domains.
- `bm25_reference`, `semantic`, `sections`, `summarizer`, and `doc_summarizer` are retrieval/summarization domains.

Implication:
- `lib.rs` can stay skeletal, but the flat top-level module list should converge toward grouped domains instead of more root files.
- We should not split into workspace crates until a domain becomes independently reusable and stops sharing half the crate's types.

## Current hotspots

These are the primary refactor drivers, not a full inventory.

| Hotspot | Problem | Direction |
|---|---|---|
| ~~`src/service/agent_v1_tools.rs`~~ (removed) | Was a god module; workstream 1 split it into `src/service/agent_v1/*` with `registry.rs`, and moved cross-cutting analysis into `src/analysis/*`. | Done for the MCP handler surface; further tightening is incremental. |
| ~~`src/dep_graph.rs`~~ | ~~Algorithms and graph analysis~~ | **Resolved (WS6):** `src/analysis/graph/*`; `dep_graph.rs` re-exports for stable paths. |
| `src/extractors/engine/scm.rs` (1709 LOC) | Query catalog, query runtime, docstring cleanup, and heuristic post-processing live together. | Split query catalog/runtime/post-processing. Remove inline language-classification heuristics from the shared engine. |
| ~~`src/sections.rs`~~ | ~~Section parsing/scoring/formatting bundled~~ | **Resolved (WS6):** `src/sections/*` (markdown, RST, html, openapi/json, etc.). |
| ~~`src/storage.rs`~~ | ~~Index + search + helpers coupled~~ | **Resolved (WS6):** `src/storage/*` (schema, index_store, search, symbol/graph/analysis reads). |
| ~~`src/framework/laravel.rs`~~ | ~~Monolithic provider~~ | **Resolved (WS5):** `src/framework/laravel/*` — parsers vs `provider.rs` facade. |
| `src/extractors/vue.rs`, `src/extractors/al.rs`, `src/extractors/asm.rs`, `src/extractors/yaml.rs`, `src/extractors/sql.rs` | Override extractors each mix parsing, classification, fallback heuristics, and output shaping. | Normalize them around shared helpers and explicit fallback policy. |
| `src/import_extract.rs` | Duplicates import logic already present in `src/dep_graph.rs` and stays line-oriented. | Delete after replacing consumers with a shared structural import API. |

## Design-pattern issues

These are the main `rust-design-patterns` failures to correct.

| Area | Current smell | Target pattern |
|---|---|---|
| `agent-v1` handlers | Giant string-based dispatch with centrally edited mode branches | Command/Handler registry. Each tool family gets its own handler type. |
| Import extraction | Was duplicated between `dep_graph` and `import_extract` | **Improved:** single owner `indexing::imports` with language strategies (TS primary, line fallback). Trait-based `ImportExtractor` optional follow-up. |
| Framework providers | Top-level Strategy is fine, but each provider was internally monolithic | **Improved (WS5):** Laravel / Next.js / Nuxt / dbt split into focused modules; `*Provider` stays the `FrameworkProvider` facade. |
| Extractor specialization | Registry knows too much about special cases and path heuristics | Add explicit classifier/fallback layer instead of growing registry conditionals. |
| Heuristic analysis | Dead-code, cross-repo, and reference checks embed ecosystem rules inline | Introduce small detectors/seed providers instead of repeated local closures and scans. |

## Heuristic debt inventory

These are the places where content semantics currently come from string or regex matching and should move toward structural sources.

| Location | Current behavior | Problem | Replacement target |
|---|---|---|---|
| ~~`src/extractors/engine/scm.rs:431-483`~~ | ~~`cpp` header language tweak~~ | ~~Inline in shared engine~~ | **Resolved:** `src/extractors/engine/cpp_header_language_fallback.rs` (named post-pass; still heuristic) |
| ~~`src/extractors/sql.rs`~~ | ~~dbt/CTE regex mixed into one file~~ | ~~Unclear pipeline order~~ | **Resolved:** `src/extractors/sql/mod.rs` + `sql/fallback.rs`; policy in `extractors/fallback_pipeline.rs` |
| `src/extractors/razor.rs` | Markup/`@page` still regex; `@code`/`@functions` and inline `<script>` were regex-only | Mixed grammars; naive C# regex missed real method shapes | **Primary path:** tree-sitter C# (wrapped fragment) + JavaScript; regex fallback + tests for nested generics |
| ~~`src/extractors/mod.rs:142-176`~~ | ~~OpenAPI/Ansible in registry~~ | ~~Duplicated routing~~ | **Resolved:** `src/extractors/classifier.rs` + `registry_builder.rs`; registry calls classifier first |
| ~~`src/extractors/yaml.rs`~~ | ~~Duplicate Ansible path list~~ | ~~Two copies of rules~~ | **Resolved:** YAML uses `classifier::path_suggests_ansible_layout` |
| `src/indexing/imports/` | Primary path: tree-sitter for Python, JS/TS/TSX/JSX, Go when parse has no errors; line fallback in `line.rs` for failures; Vue/SQL/Dart remain scanner/line-based | Fallbacks still format-sensitive for some edge cases | Extend query/AST coverage per language; narrow fallbacks |
| `src/indexing/imports/bundle.rs` | Python `context_bundle` import lines use verbatim `import`/`from` source lines for display parity (incl. `import *`); Go formats block from TS edges or raw lines | Display path is not identical to graph edge list for Python | Unify only if bundle output can represent star imports from `ImportEdge` alone |
| `src/indexing/imports/cross_repo.rs` | Cross-repo uses `extract_imports` + `import_edges_reference_package`; still allows `specifier.contains(pkg)` for parity with old line scanner | Substring match can over-trigger on short package names | Prefer npm scope / exact specifier rules where metadata exists |
| `src/analysis/dead_code.rs` (was `agent_v1_tools`) | Dead-code and dead-code-v2 use basenames, `__main__` substring checks, and symbol-name inclusion tests | Heuristic quality logic is hard to extend safely | Evolve toward explicit `EntrypointDetector` and graph-seed providers in `analysis/dead_code/*` |
| `src/framework/laravel/*` | Blade/Inertia and API-call matching still regex/heuristic | Migrations/models/routes：**PHP tree-sitter first** + regex gap-fill (columns, route `->name()`, fillable); Blade/Inertia remain heuristic | Expand structural Blade when grammar is solid |
| ~~`src/framework/nextjs.rs` (route handlers)~~ | ~~regex-only `next_api_methods_from_content`~~ | ~~Misses multiline `export function` headers~~ | **Resolved:** TypeScript + TSX AST walk; regex fallback (`src/framework/nextjs/routing.rs`). |

## Acceptable heuristics

Not every non-query rule is wrong. The following remain acceptable unless we find a better structural source:

- path-to-route mapping for frameworks whose semantics are explicitly file-system-based, such as Next.js and Nuxt
- extension and compound-extension routing when the file extension is the language contract
- repo-root and config-file detection such as `next.config.ts` or `composer.json`

Everything else should trend toward structural extraction, especially:
- imports
- symbol classification
- entrypoint discovery
- framework metadata parsed from source code
- cross-repo package references

## Target module shape

This is the target direction, not a rigid final tree.

```text
src/
  api/
    agent_v1/
    mcp/
    service/
  indexing/
    extractors/
    imports/
    framework/
  analysis/
    graph/
    impact/
    quality/
  retrieval/
    search/
    summaries/
    sections/
  storage/
    code_index/
    doc_index/
    cache/
  core/
    config/
    ids/
    types/
```

Rules for the target shape:
- `api` depends inward on other domains; no reverse dependencies
- `indexing` produces structural facts; it does not know about public tool responses
- `analysis` consumes indexed facts; it does not scan raw source ad hoc in handlers
- `retrieval` owns ranking/summarization/section logic
- `storage` owns persistence and I/O, not business rules
- `core` stays small and focused on truly shared types/utilities

## Workstreams

### 1. Split `agent-v1` handlers

Status:
- [x] done (2026-04)

Tasks:
- create per-tool-family modules for ingest, catalog/search, content, structure, graph, impact, quality, context, and index admin
- introduce a small handler registry so adding a tool family is additive instead of editing a 6k-line file
- move dead-code, reference-check, and cross-repo analysis out of API handlers into analysis modules

Done when:
- no handler file exceeds 500 LOC
- `src/service/mod.rs` remains the thin facade and response wrapper
- the public tool names and schemas still match `src/tools.json`

Outcome:
- Dispatch: `src/service/agent_v1/mod.rs` + `src/service/agent_v1/registry.rs` (tool name → `CodeService` method).
- Shared args/types: `src/service/agent_v1/args.rs`, helpers in `util.rs` / `util_graph.rs`.
- Context assembly: `context.rs` (bundle/ranked), `context_tail.rs` (section/query/digest/unknown), `context_bundle.rs` (`context_bundle` mode).
- Analysis pulled out of handlers: `src/analysis/dead_code.rs`, `reference_check.rs`, `cross_repo_map.rs` (`lib.rs` exports `pub mod analysis`).
- Monolithic `agent_v1_tools.rs` deleted; `src/service/mod.rs` delegates to `agent_v1::dispatch`.

### 2. Create one structural import subsystem

Status:
- [x] done (2026-04)

Tasks:
- move import extraction out of `src/dep_graph.rs`
- replace `src/import_extract.rs` with calls into the shared subsystem, then delete it
- add tree-sitter or query-backed import capture for supported languages
- make dependency graph, context assembly, and cross-repo analysis consume the same import facts

Done when:
- one API owns import extraction
- no service handler scans import-looking lines directly
- import extraction tests live with the subsystem, not in unrelated handlers

Outcome:
- Owner: `src/indexing/imports/` (`mod.rs`, `line.rs`, `resolve.rs`, `bundle.rs`, `cross_repo.rs`, `tree_sitter/*`).
- `dep_graph` re-exports `extract_imports`, `resolve_specifier`, etc., for continuity; graph build uses `resolve_import_edge_for_graph` (Python dotted-module fallback unified).
- `import_extract.rs` deleted; `context_bundle` uses `context_bundle_import_lines`.
- Cross-repo: `cross_repo_map.rs` and `graph_tail` (`dependencies`) use `extract_imports` + `import_edges_reference_package` (no raw line scan).
- Tests: `indexing/imports` unit tests; integration tests import from `indexing::imports` where appropriate.

### 3. Decompose extractor routing and fallback policy

Status:
- [x] done (2026-04)

Tasks:
- split `ExtractorRegistry` into registry, classifier, and builder pieces
- remove duplicated Ansible/OpenAPI routing logic
- move special-case language classification out of the shared SCM engine
- require each override extractor to document why query-only extraction is insufficient

Done when:
- extractor selection logic no longer embeds domain-specific path rules in the registry
- each override extractor has a narrow reason to exist
- fallback heuristics are isolated and named

Outcome:
- Classifier: `src/extractors/classifier.rs` (OpenAPI basename + segment-aware Ansible paths).
- Builder: `src/extractors/registry_builder.rs`; `ExtractorRegistry::default()` delegates there.
- C/C++ header language relabel: `src/extractors/engine/cpp_header_language_fallback.rs`.
- Override extractors carry `//!` rationale (al, asm, blade, ejs, luau, openapi, razor, scss, sql, verse, vue, yaml).

### 4. Normalize tree-sitter-first extraction

Status:
- [x] done (2026-04)

Tasks:
- add supplemental query or AST passes before regex fallbacks in SQL, Razor, Laravel PHP parsing, and similar domains
- define a fallback policy: structural pass first, fallback second, dedupe last
- add tests that prove structural extraction is the primary path

Done when:
- content-derived semantics come from AST/query in the default path
- regex fallback is limited to grammar gaps and called out explicitly in code and tests

Outcome:
- Policy + helpers: `src/extractors/fallback_pipeline.rs` (sort/dedupe; module doc describes ordering).
- SQL: `src/extractors/sql/mod.rs`, `sql/fallback.rs` (dbt + CTE regex); pipeline uses fallback helpers.
- Razor: `src/extractors/razor.rs` — C# + JS tree-sitter with regex fallback; `tests/razor.rs` nested-generic case.
- Laravel: `src/framework/laravel/php_structure.rs` — migrations (table), models (relationships/scopes), routes; merges with regex; `tests/agent_v1_framework_laravel.rs` multiline `Schema::create`.
- Next.js: `src/framework/nextjs/routing.rs` — `next_api_methods_from_content` uses TS/TSX AST, regex fallback; multiline test in `tests/agent_v1_framework_nextjs.rs`.

### 5. Split framework providers internally

Status:
- [x] done (2026-04)

Tasks:
- split `laravel.rs` into migrations, models, routes, views, inertia, and provider facade
- apply the same decomposition pattern to `nextjs.rs`, `nuxt.rs`, and `dbt.rs`
- reuse shared query/import helpers instead of provider-local string parsing where possible

Done when:
- provider files become facades, not logic dumps
- context building is separate from source parsing
- per-framework tests remain meaningful and grow where behavior changes

Outcome:
- Laravel: `src/framework/laravel/mod.rs` re-exports; `types.rs`, `migrations.rs`, `models.rs`, `routes.rs`, `blade.rs`, `facade.rs`, `inertia.rs`, `api_client.rs`, `php_structure.rs`, `provider.rs` (`LaravelProvider` + scans / `get_extra_imports`).
- Next.js: `nextjs/routing.rs`, `nextjs/middleware.rs`, `nextjs/provider.rs`.
- Nuxt: `nuxt/routing.rs`, `nuxt/provider.rs`.
- dbt: `dbt/project.rs`, `dbt/docs.rs`, `dbt/schema.rs`, `dbt/provider.rs`.
- Public paths `framework::{laravel,nextjs,nuxt,dbt}::*` unchanged for integration tests.

### 6. Break up oversized storage/retrieval/analysis modules

Status:
- [x] done (2026-04)

Tasks:
- split `src/dep_graph.rs`, `src/storage.rs`, and `src/sections.rs` by responsibility
- move Python import parsing helpers out of storage/analysis code that does not own parsing
- keep algorithms near their owning domain instead of hiding them in general-purpose files

Done when:
- each file answers one question in one sentence
- no file exceeds roughly 500 lines without a strong reason
- dependency direction is obvious from module names

Outcome:
- Graph: `src/analysis/graph/{cycles,layers,psr4,import_graph,coupling,pagerank,calls}.rs`, `mod.rs`, `tests.rs`; `src/dep_graph.rs` → `pub use crate::analysis::graph::*`.
- Storage: `src/storage/{schema,types,convert,index_store,search,symbol_reads,graph_reads,analysis_reads}.rs`, `mod.rs`; Python “most imported” uses `indexing::imports::python_top_level_import_stems` (`python_stems.rs`).
- Sections: `src/sections/{markdown,rst,asciidoc,plain,builder,html,preprocess,openapi_json,refs,paths}.rs`, `mod.rs`, `tests.rs`; public `sections::*` unchanged.

## Guardrails for every refactor patch

- Do not delete tests to make refactors easier.
- When moving logic, move or add tests in the same patch.
- Prefer replacing duplicate helpers with one shared implementation.
- Avoid adding new root-level modules unless they clearly define a new domain.
- Keep `lib.rs` skeletal.
- If a new heuristic is unavoidable, note it here and in code with the reason the structural path is insufficient.

## Open questions for the next iteration

- **Resolved:** import capture lives under `src/indexing/imports/` (produces indexed structural facts; `dep_graph` and handlers consume it).
- Which framework/source grammars are good enough today to replace the highest-value regex paths first?
- **Partially resolved:** OpenAPI/Ansible path routing lives in `src/extractors/classifier.rs` (segment-aware Ansible); a trait is optional if more stages need plugging in.
- Which storage types should move out of `src/model.rs` and into owning domains, and which truly belong in shared core types?
- What is the smallest first code refactor that proves the new shape without touching the public MCP surface?
