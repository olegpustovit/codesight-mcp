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
- Extensibility should prefer query packs, language profiles, and generic structural runners over adding new Rust extractor types.
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

## Audit update (2026-04-08)

Pattern A still fits, but the crate is only halfway to the intended domain shape.

Primary findings from the current tree:
- `service` is no longer the main god-module problem for MCP dispatch, but it still owns ingest orchestration (`code_tools.rs`, `doc_tools.rs`) that belongs in indexing/storage domains.
- `model.rs`, `doc_storage.rs`, and `cross_repo.rs` are still legacy root-level bags; they blur ownership between core types, persistence rows, package metadata, and analysis logic.
- The highest remaining modularity risk is now inside extractor/fallback internals, not in the public `agent-v1` surface.
- The extractor system is already mostly query-first in spirit, but not yet in shape: 29 built-in query packs exist while the remaining Rust override extractors mostly represent container hosts or no-grammar dialects rather than parser gaps.
- `luau`, `scss`, `sql`, and `yaml` no longer bypass the shared SCM engine wholesale; `sql`/`yaml` now use override-backed query packs with tiny host-specific postpasses instead of bespoke structural walkers.
- Registration rigidity is easing: **`QUERY_FILES` is now build-generated** from `runtime/queries/*/tags.scm`, and adding a pack for an existing compiled grammar no longer requires touching `registry_builder`; `builtin_grammar` still owns compiled grammar availability.
- The real blocker for `vue`, `razor`, `blade`, and `ejs` is mixed-language orchestration and profile/alias handling, not an inability to express symbols with queries once the right subtree is isolated.
- “Works acceptably on any codebase” cannot mean “curate every language by hand.” We need a tiered extractor model: curated query packs first, generic AST fallback second, container/injection runners third, tiny residual overrides last.
- The highest remaining heuristic risk is now in analysis evidence (`dead_code`, `reference_check`, cross-repo package matching) and mixed-language fallback extractors.
- `lib.rs` is skeletal, but the export surface still reflects historical root modules more than stable domain facades.

## Current hotspots

These are the primary refactor drivers, not a full inventory.

| Hotspot | Problem | Direction |
|---|---|---|
| `src/extractors/engine/mod.rs` + `src/extractors/engine/scm.rs` + `src/extractors/registry_builder.rs` | Grammar lookup, embedded query files, and extractor registration are still compile-time Rust edits. | Split grammar catalog from query-pack/profile catalogs; make pack discovery data-driven and keep Rust registration for grammars only. |
| `runtime/queries/` | Pack layout is now standardized, but injections are only catalogued and most built-in metadata still lives in headers rather than richer pack files. | Keep the `tags.scm` / optional `injections.scm` / metadata contract stable and use it as the base for container/injection runner work. |
| `src/extractors/{yaml}.rs`, `src/extractors/sql/mod.rs` | These shims now delegate structural extraction to query packs, but still carry host-specific postprocess logic (`ansible`, dbt/Jinja/docstring shaping). | Keep them thin and move any reusable postprocess pieces into shared helpers before adding new host layers. |
| `src/extractors/engine/scm.rs` (1667 LOC) | Query header parsing, pattern metadata, query execution, scope inference, doc cleanup, and a heuristic post-pass still live in one file. | Split into engine submodules with `ScmExtractor` as a thin facade and named internal phases. |
| `src/indexing/imports/line.rs` (675 LOC) | Python, JS/TS, Vue, SQL, Dart, and Go fallbacks all live in one function family; service code duplicates capability knowledge with `LANGUAGES_WITH_IMPORT_EXTRACTION`. | Split fallback extractors by language and expose capability metadata from `indexing::imports`. |
| `src/extractors/vue.rs`, `src/extractors/razor.rs`, `src/extractors/al.rs`, `src/extractors/asm.rs`, `src/extractors/yaml.rs` | Mixed-language / dialect-specific override extractors still bundle block detection, classification, fallback logic, and symbol emission. | Convert each override into a small facade over strategy modules or staged helpers. |
| `src/service/code_tools.rs` + `src/service/doc_tools.rs` | Boundary layer still crawls the filesystem, computes deltas, reads files, builds dep-graphs, extracts package names, and writes indexes. | Move ingest orchestration into indexing/storage domains; keep service focused on argument validation and response shaping. |
| `src/doc_storage.rs` (562 LOC) | Doc-index schema, writes, reads, summary APIs, and stale checks are still a root-level module outside `storage/`. | Fold into `storage/doc_index/*` and align doc/code index APIs. |
| `src/model.rs` | Cross-domain type bag mixes symbol kinds, persistence rows, response DTOs, and indexing updates. | Move types to owning domains; keep only truly shared core types in a small `core` area. |
| `src/analysis/dead_code.rs`, `src/analysis/reference_check.rs`, `src/indexing/imports/cross_repo.rs` | Quality/cross-repo evidence still depends on basename checks, line scans, and substring matching. | Replace with typed evidence providers that consume structural facts first and document any residual heuristics. |
| `src/cross_repo.rs` | Root-level package registry and manifest parsing remain detached from both indexing and analysis domains. | Move into an explicit package metadata domain (`indexing/packages` or `analysis/cross_repo`) with manifest adapters. |
| `src/framework/laravel/provider.rs` (542 LOC) | The provider split helped, but project detection/load, context rendering, and extra-import assembly still accumulate in one facade. | Move toward typed framework snapshots plus smaller render/import stages. |
| `src/security.rs` (510 LOC) | Path validation, binary detection, secret detection, and file-policy rules remain packed together. | Split into focused policy modules once extractor/indexing boundaries settle. |

## Design-pattern issues

These are the main `rust-design-patterns` failures to correct.

| Area | Current smell | Target pattern |
|---|---|---|
| `agent-v1` handlers | **Improved:** registry-based dispatch is in place, but service methods still do non-boundary work | Keep command dispatch in `service/agent_v1/*`; move ingest/index operations behind dedicated domain coordinators. |
| Extractor registration | Grammars, query files, aliases, and overrides are all wired in Rust lists | `GrammarCatalog` + `QueryPackCatalog` + `LanguageProfileRegistry`, with Rust only owning compiled grammar availability. |
| SCM query engine | One extractor type owns query metadata parsing, capture collection, scope inference, and cleanup | Facade + internal phase modules (`query_pack`, `capture`, `scope`, `docs`). |
| Query protocol | Current capture/property dialect is crate-specific and narrower than Tree-sitter’s `tags` / `injections` conventions | Standard tags/injections vocabulary as baseline, with a small documented extension surface for current `SymbolKind`s. |
| Import fallback extraction | One large fallback file owns unrelated language families | Strategy per language fallback; dispatcher in `indexing::imports::mod.rs` only. |
| Language aliases | `openapi` and `ansible` are modeled as bespoke extractors even though they mostly reuse JSON/YAML structure | Language profiles layered over base packs, not custom extractors. |
| Override extractors | Many large overrides exist because the engine cannot yet express mixed-language or packless-grammar cases generically | Residual override budget only for no-grammar / multi-dialect cases; everything else moves to query packs or generic container runners. |
| Framework providers | Provider trait is fine, but project loading and rendering are still too coupled inside concrete providers | Snapshot builder + renderer/import collector stages behind each provider facade. |
| Heuristic analysis | Dead-code, cross-repo, and reference checks embed ecosystem rules inline | Evidence providers / seed detectors consuming indexed structural facts. |
| Shared type ownership | `model.rs` acts as a pseudo-“types bag” for unrelated domains | Domain-owned structs plus a very small shared core (`SymbolKind`, IDs, maybe fingerprints if still genuinely shared). |

## Heuristic debt inventory

These are the places where content semantics currently come from string or regex matching and should move toward structural sources.

| Location | Current behavior | Problem | Replacement target |
|---|---|---|---|
| ~~`src/extractors/engine/scm.rs:431-483`~~ | ~~`cpp` header language tweak~~ | ~~Inline in shared engine~~ | **Resolved:** `src/extractors/engine/cpp_header_language_fallback.rs` (named post-pass; still heuristic) |
| ~~`src/extractors/sql.rs`~~ | ~~dbt/CTE regex mixed into one file~~ | ~~Unclear pipeline order~~ | **Resolved:** `src/extractors/sql/mod.rs` + `sql/fallback.rs`; policy in `extractors/fallback_pipeline.rs` |
| `src/extractors/razor.rs` | Markup/`@page` still regex; `@code`/`@functions` and inline `<script>` were regex-only | Mixed grammars; naive C# regex missed real method shapes | **Primary path:** tree-sitter C# (wrapped fragment) + JavaScript; regex fallback + tests for nested generics |
| `src/extractors/vue.rs` | `<script>` block detection is regex-based and options/composition extraction share one large pass | SFC mixed-language handling is valid, but phase boundaries are implicit and hard to extend | Prefer a container/injection runner: isolate block extraction once, then drive JS/TS symbol capture from query packs plus tiny Vue-specific post-pass rules |
| `src/extractors/blade.rs` + `src/extractors/ejs.rs` | Template constructs and embedded languages are flattened into regex-only extractors | These languages are containers, not ordinary one-pass symbol grammars | Prefer grammar/injection-backed container profiles when stable grammars exist; otherwise keep only declarative segment extraction in Rust and delegate inner code to query packs |
| `src/extractors/asm.rs` | Dialect selection still uses `contains()` sniffing over full source | Content heuristics are broad and can silently select the wrong dialect | Prefer extension / explicit markers first; isolate content sniffing as a named fallback with dialect-specific tests |
| ~~`src/extractors/mod.rs:142-176`~~ | ~~OpenAPI/Ansible in registry~~ | ~~Duplicated routing~~ | **Resolved:** `src/extractors/classifier.rs` + `registry_builder.rs`; registry calls classifier first |
| ~~`src/extractors/yaml.rs`~~ | ~~Duplicate Ansible path list~~ | ~~Two copies of rules~~ | **Resolved:** YAML uses `classifier::path_suggests_ansible_layout` |
| `src/extractors/openapi.rs` + `src/extractors/yaml.rs` | OpenAPI and Ansible are still shaped as separate extractor types even though they reuse JSON/YAML structure | Path/profile semantics are leaking into extractor count | Replace with language profiles over shared query packs (`yaml`, `json`) and pack-specific shaping rules |
| `src/indexing/imports/` | Primary path: tree-sitter for Python, JS/TS/TSX/JSX, Go when parse has no errors; line fallback in `line.rs` for failures; Vue/SQL/Dart remain scanner/line-based | Fallbacks still format-sensitive for some edge cases | Extend query/AST coverage per language; narrow fallbacks |
| `src/indexing/imports/bundle.rs` | Python `context_bundle` import lines use verbatim `import`/`from` source lines for display parity (incl. `import *`); Go formats block from TS edges or raw lines | Display path is not identical to graph edge list for Python | Unify only if bundle output can represent star imports from `ImportEdge` alone |
| `src/indexing/imports/cross_repo.rs` | Cross-repo uses `extract_imports` + `import_edges_reference_package`; still allows `specifier.contains(pkg)` for parity with old line scanner | Substring match can over-trigger on short package names | Prefer npm scope / exact specifier rules where metadata exists |
| `src/analysis/reference_check.rs` | `check_references` still scans raw file lines for `import` / `from` and content hits | Reimplements “reference evidence” outside the import subsystem and does not explain evidence quality | Replace with import-edge / symbol-reference evidence providers and explicit fallback notes |
| `src/analysis/dead_code.rs` (was `agent_v1_tools`) | Dead-code and dead-code-v2 use basenames, `__main__` substring checks, and symbol-name inclusion tests | Heuristic quality logic is hard to extend safely | Evolve toward explicit `EntrypointDetector` and graph-seed providers in `analysis/dead_code/*` |
| `src/cross_repo.rs` | Package names are parsed via a mix of TOML/JSON reads, INI-style scanning, `go.mod` line scans, and XML substring extraction | Manifest knowledge is useful but lives outside a named package/manifest domain | Move into manifest adapters under a package metadata subsystem with per-ecosystem tests |
| `src/framework/laravel/*` | Blade/Inertia and API-call matching still regex/heuristic | Migrations/models/routes：**PHP tree-sitter first** + regex gap-fill (columns, route `->name()`, fillable); Blade/Inertia remain heuristic | Expand structural Blade when grammar is solid |
| ~~`src/framework/nextjs.rs` (route handlers)~~ | ~~regex-only `next_api_methods_from_content`~~ | ~~Misses multiline `export function` headers~~ | **Resolved:** TypeScript + TSX AST walk; regex fallback (`src/framework/nextjs/routing.rs`). |

## Acceptable heuristics

Not every non-query rule is wrong. The following remain acceptable unless we find a better structural source:

- path-to-route mapping for frameworks whose semantics are explicitly file-system-based, such as Next.js and Nuxt
- extension and compound-extension routing when the file extension is the language contract
- repo-root and config-file detection such as `next.config.ts` or `composer.json`
- doc format detection by extension (`.md`, `.rst`, `.ipynb`, etc.)
- filename / extension policy checks in `security` when the policy itself is filename-based (binary extensions, secret suffixes)
- assembly dialect selection by extension or explicit assembler directives; broad content sniffing stays a fallback, not the primary path

Everything else should trend toward structural extraction, especially:
- imports
- symbol classification
- entrypoint discovery
- framework metadata parsed from source code
- cross-repo package references

## Query-first extractor policy

Goal:
- adding support for a new language or embedded-language variant should usually mean adding query data or a profile, not editing Rust core

Support tiers:

| Tier | Use when | Mechanism | Rust ownership |
|---|---|---|---|
| Tier 1: curated query pack | Grammar exists and we care about quality | `tags.scm` + optional `injections.scm` + pack metadata | shared engine only |
| Tier 2: generic AST fallback | Grammar exists but no curated pack exists yet | language-agnostic walker using node kind / `name` field conventions | one shared fallback, no language-specific extractor |
| Tier 3: container/injection runner | File embeds one or more child languages | grammar-backed injections or declarative segment extraction + delegation to child packs | shared container helpers only |
| Tier 4: residual override | No stable grammar exists, or the file is truly multi-dialect / host-specific | small named Rust extractor with exit criteria | explicitly budgeted and temporary by default |

Rules:
- Query packs are the default path for symbol extraction whenever a grammar exists.
- A new grammar should get at least Tier 2 generic AST extraction before we write a bespoke Rust extractor.
- Container languages should delegate inner-language symbol capture to existing packs; host Rust code should only find regions, map offsets, and add any unavoidable synthetic symbols.
- Path or product aliases (`openapi`, `ansible`) should be modeled as language profiles over shared packs, not distinct extractors.
- Residual overrides are allowed only when we can state exactly why Tier 1-3 are insufficient.

Current override triage:

| Bucket | Current files | Direction |
|---|---|---|
| Query-pack candidates now | none of the previously blocked grammar-backed paths | Keep new grammar-backed support query-first; do not reintroduce structural walkers for compiled grammars |
| Profile / alias candidates now | `openapi`, `ansible` | Replace with profiles layered over `json` / `yaml` packs |
| Container / injection targets | `vue`, `razor`, `blade`, `ejs`, dbt/Jinja in `sql` | Move to shared container runner with pack-driven child extraction |
| Residual manual for now | `asm`, `al`, `verse` | Keep as named residual overrides until grammar quality or dialect support is good enough |

What “acceptable on any codebase” means in extractor terms:
- if a built-in grammar exists but we have no curated pack yet, we should still emit a baseline symbol set through the generic AST fallback
- if a curated pack exists, it should win over the generic fallback
- if a container language embeds other languages, we should still extract acceptable child symbols even when host semantics are partial
- if no grammar exists, the system may fall back to text search only, but that should be an explicit support-tier decision, not an accidental gap

## Target module shape

This is the target direction, not a rigid final tree.

```text
src/
  core/
    ids/
    symbols/
    types/
  service/
    agent_v1/
  indexing/
    code_index/
    imports/
    packages/
  extractors/
    engine/
      catalogs/
      query_pack/
      generic_ast/
      injections/
      postprocess/
    containers/
      offsets/
      segments/
    profiles/
    residual/
  analysis/
    graph/
    quality/
    cross_repo/
  framework/
    dbt/
    laravel/
    nextjs/
    nuxt/
  retrieval/
    search/
    summaries/
    sections/
  storage/
    code_index/
    doc_index/
    cache/
```

Rules for the target shape:
- `service` is the only MCP boundary and response-shaping layer; it should not crawl repos or own indexing algorithms
- `indexing` owns crawl/delta/import/package fact production; it does not know about public tool responses
- `extractors` own symbol extraction and mixed-language extraction helpers, not persistence or API DTOs
- `extractors/engine` owns shared grammar, pack, generic-AST, injection, and postprocess machinery
- `extractors/containers` owns offset mapping and host-file region extraction, not language-specific symbol logic
- `extractors/profiles` owns aliases such as `openapi` / `ansible` and any path-based language shaping
- `extractors/residual` is a deliberately small set of named manual extractors for unsupported grammars or dialect bundles
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

### 6A. Standardize the query-pack protocol and catalogs

Status:
- [x] done (2026-04-08)

Progress:
- [x] `build.rs` scans canonical `runtime/queries/*/tags.scm` packs plus optional `injections.scm` / `pack.toml`, and generates `OUT_DIR/builtin_query_packs.rs` with `BUILTIN_QUERY_PACKS` plus compatibility `QUERY_FILES`
- [x] pack id in `QUERY_FILES` is the directory name (`rust` under `runtime/queries/rust/`, not a separate Rust alias key)
- [x] `GrammarCatalog` / `QueryPackCatalog` / `LanguageProfileRegistry` split in code
- [x] workspace `.codesight.toml` can provide `query_pack_roots = ["..."]`; `ExtractorRegistry::for_workspace_root(...)` overlays those packs without editing Rust core
- [x] lightweight `pack.toml` metadata files are supported for workspace and built-in packs (`language`, `extensions`, `grammar`, `override_extractor`)
- [x] all built-in packs now use canonical `tags.scm`; `symbols.scm` is no longer part of built-in discovery
- [x] `QueryPackCatalog` validates pack metadata (`extensions`, `grammar` or `override_extractor`) and rejects malformed `pack.toml` input instead of silently ignoring it
- [x] pack compile / discovery tests enumerate pack directories from disk, enforce canonical `tags.scm` layout, and compile every built-in SCM pack
- [x] `runtime/queries/README.md` documents the pack layout, metadata precedence, baseline tag vocabulary, and the small query-property extensions still used by the engine

Tasks:
- replace the hardcoded `QUERY_FILES` list with auto-discovered built-in query-pack directories under `runtime/queries/*`
- separate `GrammarCatalog` (compiled grammars) from `QueryPackCatalog` (query data) and `LanguageProfileRegistry` (aliases/routing)
- move from the crate-local `symbols.scm` convention toward standard pack layout: `tags.scm`, optional `injections.scm`, and lightweight metadata
- adopt Tree-sitter tag/injection vocabulary as the baseline capture protocol and document the minimal extensions needed for current `SymbolKind`s
- support additional query-pack roots from workspace configuration so teams can extend extraction without changing Rust core

Done when:
- adding a plain language with an existing compiled grammar means adding a pack directory, not editing `QUERY_FILES` or `registry_builder`
- path aliases such as `openapi` and `ansible` are expressed as profiles over base packs
- pack compile tests discover packs from disk instead of maintaining a second manual list

### 6B. Add generic AST fallback and stop treating aliases as extractors

Status:
- [x] done (2026-04-08)

Progress:
- [x] shared `GenericAstExtractor` added and registered for built-in grammars that do not yet have curated query packs
- [x] `openapi` / `ansible` path aliases now resolve through `LanguageProfileRegistry` + generic `ProfiledExtractor` wrappers instead of bespoke alias extractor types
- [x] registry now exposes `curated_languages()` separately from the broader supported language set
- [x] `ExtractionTier` is stored on `StoredSymbol`, written to SQLite, read back through `SymbolRecord`, and surfaced in file/repo outline rows for downstream consumers
- [x] registry now has an explicit language-level lookup (`extractor_for_language`) that prefers structural tiers (`curated > generic AST > manual`) without changing path-based override routing
- [x] built-in grammar coverage is locked by tests: every grammar in `GrammarCatalog` resolves to either a curated pack or the shared generic AST fallback via registry language lookup
- [x] the leftover dead `OpenApiExtractor` implementation was removed; alias support now lives only in profiles/classifier routing
- [x] tests now assert that “supported language” and “curated language” are distinct registry concepts, and that profiled aliases rewrite emitted symbol language/tier

Tasks:
- add a generic AST extractor for grammars with no curated pack yet, using node-kind and `name`-field conventions as a baseline structural fallback
- define confidence tiers so downstream analysis can distinguish curated-pack facts from generic-AST fallback facts
- replace `OpenApiExtractor` and `AnsibleExtractor` with profile/config routing over existing JSON/YAML structure
- keep the generic fallback small and shared; do not create one-off per-language fallback walkers when the grammar exists

Done when:
- every built-in grammar can produce at least baseline symbols without a bespoke extractor
- the number of extractor types no longer grows just to represent aliases or profile names
- “language supported” and “language curated” are distinct concepts in the registry and tests

### 6C. Add injection/container extraction and migrate query-capable overrides

Status:
- [x] done (2026-04-08)

Progress:
- [x] `luau` moved onto a curated `runtime/queries/luau/tags.scm` pack; registry dispatch now prefers the SCM path and the language tests exercise the pack via registry/query loading
- [x] `scss` moved onto a curated `runtime/queries/scss/tags.scm` pack; registry dispatch now prefers the SCM path and the CSS/SCSS tests exercise the pack rather than the old `ScssExtractor`
- [x] workspace pack roots can now add or override curated packs, which gives the shared container runner a stable pack-loading surface
- [x] `src/extractors/containers.rs` now owns shared delegated-language extraction, host offset remapping, wrapped fragment parsing, and HTML tag block discovery
- [x] `runtime/queries/yaml/tags.scm` and `runtime/queries/sql/tags.scm` provide structural query-pack coverage; `YamlExtractor` and `SqlExtractor` are now thin wrappers that keep only `ansible` and dbt/Jinja/docstring semantics
- [x] `vue`, `razor`, `blade`, and `ejs` now delegate embedded JS/TS/C#/PHP through the shared container helper instead of each one reimplementing byte/line remapping
- [x] override-backed pack compilation is covered in `tests/query_pack_catalog.rs`, and host/container behavior is exercised by the existing language-specific integration suites

Tasks:
- keep evolving the shared container runner toward true recursive `injections.scm` execution where host grammars can support it
- keep dbt/Jinja, Razor markup directives, Blade directives, and similar host-only semantics as tiny named post-passes rather than whole extractors
- avoid reintroducing per-language offset/segment remapping once `extractors/containers.rs` exists

Done when:
- query-capable overrides are deleted or reduced to thin container/postprocess shims
- mixed-language extraction logic is shared instead of reimplemented in each host language
- the residual override set is limited to real no-grammar / multi-dialect cases

### 7. Decompose the SCM query engine

Status:
- [~] in progress (2026-04-08)

Progress:
- [x] `src/extractors/engine/scm/` — `metadata.rs` (headers), `pattern.rs` (`#set!` props), `capture.rs` (raw rows + kind parse), `docs.rs` (docstring cleanup), `extract.rs` (capture → dedupe → standalone docs → scope stack → symbols + cpp header pass); phase ordering documented in `extract.rs`
- [x] `ScmExtractor` remains the public façade in `scm/mod.rs`; `QueryPackCatalog` + `ScmExtractor::from_pack` + `build_scm_extractors_from_catalog` add an explicit pack-loading seam before query execution
- [x] build-generated pack catalog remains re-exported via `scm::QUERY_FILES` so existing extractor tests keep one lookup surface during the refactor
- [ ] thin `mod.rs` after moving the large `#[cfg(test)]` suite to `scm/tests.rs` or similar
- [~] shared coordinator for pack vs generic-AST is in place through registry/catalog wiring, and `extractors/containers.rs` now provides the first shared host/container delegation seam; pack-driven recursive injections are the next coordinator gap

Tasks:
- split `src/extractors/engine/scm.rs` into internal modules for query-pack metadata, pattern properties, raw capture collection, scope inference, and doc cleanup
- add explicit engine phases for pack loading and postprocess execution so pack-driven extraction, container recursion, and generic AST fallback share the same coordinator
- keep `ScmExtractor` as the public facade; new language/query packs should not require editing shared capture logic unless the protocol changes
- move engine-specific tests out of the production file where practical so failures point at the right phase
- document the internal phase ordering in code (`parse header -> compile query -> collect captures -> infer scopes -> clean docs -> finalize symbols`)

Done when:
- `src/extractors/engine/scm.rs` is primarily wiring and public API, not the full implementation dump
- adding a new `.scm` query pack is data-driven and does not invite new shared-engine conditionals
- engine tests cover header parsing, pattern props, scope inference, and doc cleanup separately

### 8. Split import fallback scanners and expose capability metadata

Status:
- [ ] proposed

Tasks:
- replace `src/indexing/imports/line.rs` with language-specific fallback modules (`python`, `js_ts`, `vue`, `sql`, `dart`, `go`, plus shared dedupe/helpers)
- expose import-extraction capability metadata from `indexing::imports` so callers stop hardcoding `LANGUAGES_WITH_IMPORT_EXTRACTION`
- keep `mod.rs` as the dispatcher and ownership point for supported languages; fallback implementations should be additive
- keep `bundle.rs` focused on presentation, not on raw extraction logic

Done when:
- no single fallback import file is a multi-language dumping ground
- ingest warnings derive from the import subsystem itself instead of stale parallel constants in `service`
- adding a new import fallback means adding a new module and registering it in one place

### 9. Budget and isolate residual overrides

Status:
- [ ] proposed

Tasks:
- classify every remaining Rust extractor as one of: `no_grammar`, `multi_dialect`, `container_boundary`, or `host_semantics`
- move residual extractors under an explicit `extractors/residual/*` area and keep each top-level file as a facade implementing `LanguageExtractor`
- require each residual override to document exit criteria: what pack, grammar, or container capability would let us delete it
- explicitly name the pattern in code comments where a strategy family exists (for example `AsmDialectStrategy`)
- do not add a new residual override until query-pack, generic-AST, and container options have been ruled out in this plan

Done when:
- the residual extractor directory is small, explicit, and stable
- new language support normally lands through packs/profiles instead of new Rust extractor files
- residual extractor tests explain the remaining unsupported grammar or dialect boundary they cover

### 10. Move ingest orchestration out of `service`

Status:
- [ ] proposed

Tasks:
- create domain-owned indexing entry points for code and docs (`indexing/code_index/*`, `storage/doc_index/*`, or equivalent) and move crawl/delta/store orchestration there
- reduce `service/code_tools.rs` and `service/doc_tools.rs` to argument validation, auto-index policy, and response shaping
- move doc index persistence out of `src/doc_storage.rs` into `src/storage/doc_index/*`
- unify code/doc indexing vocabulary where possible: crawler input, fingerprints, incremental planning, persistence result summaries

Done when:
- `CodeService` no longer walks directories, builds dep-graphs, or writes index rows directly
- code/doc indexing can be unit-tested without going through MCP dispatch
- the storage layer owns persistence APIs for both code and docs

### 11. Eliminate root-level bags and align type ownership

Status:
- [ ] proposed

Tasks:
- split `src/model.rs` into domain-owned types; keep only truly shared primitives in a small `core` area
- move `src/cross_repo.rs` into a named package metadata domain instead of leaving it as a root utility module
- move doc index structs out of `src/doc_storage.rs` to the storage domain that owns them
- update `lib.rs` exports to prefer domain facades over historical root modules as ownership becomes clear

Done when:
- `model.rs` is deleted or reduced to a minimal shared-core module
- new types are introduced in the owning domain by default instead of added to a global bag
- `lib.rs` remains skeletal while reflecting real domain boundaries instead of legacy drift

### 12. Replace heuristic quality and cross-repo analysis with typed evidence providers

Status:
- [ ] proposed

Tasks:
- split dead-code analysis into explicit evidence modules such as entrypoint detection, live-root seeding, symbol reachability, and confidence scoring
- replace `reference_check` line scanning with evidence derived from import edges, symbol references, and any unavoidable fallback scans that are explicitly labeled
- replace `specifier.contains(pkg)` matching with language-aware package matchers backed by manifest/package metadata where available
- surface evidence notes in analysis outputs so users can see why a result is structural, heuristic, or mixed

Done when:
- dead-code decisions are explainable as named evidence signals rather than one large heuristic function
- `check_references` no longer scans raw `import` / `from` lines as its primary path
- cross-repo package detection prefers exact/scoped/module-aware matches before any fallback substring rule

### 13. Make framework providers snapshot-based

Status:
- [ ] proposed

Tasks:
- keep `FrameworkProvider` as the public strategy boundary, but split each concrete provider into project snapshot loading, context rendering, metadata emission, and extra-import emission
- reduce `provider.rs` files to facades over typed per-framework snapshot structs
- keep framework-specific heuristics isolated in snapshot builders with tests that prove why they are still needed

Done when:
- concrete provider files stop accumulating unrelated phases
- adding a new framework metadata field does not require editing detection, rendering, and import code in one place
- framework context/import generation is testable from a typed snapshot without rescanning the filesystem

### 14. Split security/file policy into focused modules

Status:
- [ ] backlog

Tasks:
- split `security.rs` into path validation, secret-file policy, binary-file policy, and encoding safety modules
- document which filename-based heuristics are intentional policy and which should move toward structural checks
- keep shared file-policy helpers small and reusable from indexing code

Done when:
- `security` is no longer one 500+ LOC utility grab-bag
- indexing code can depend on narrowly scoped security/file-policy helpers
- policy changes update focused tests instead of one broad test surface

## Guardrails for every refactor patch

- Do not delete tests to make refactors easier.
- When moving logic, move or add tests in the same patch.
- Prefer replacing duplicate helpers with one shared implementation.
- Avoid adding new root-level modules unless they clearly define a new domain.
- Keep `lib.rs` skeletal.
- Do not add a new Rust extractor until query-pack, generic-AST fallback, and container/profile options have been considered and rejected explicitly.
- If a new heuristic is unavoidable, note it here and in code with the reason the structural path is insufficient.

## Recommended execution order

- Start with **WS6A** and **WS7** together. The main blocker is the rigid query-pack protocol plus the monolithic engine file; those seams should move first.
- Follow with **WS6B** so every built-in grammar has at least a generic structural path and aliases stop inflating extractor count.
- Land **WS7** next. `6C` is now in place, so the remaining extractor-engine win is tightening the SCM/internal coordinator around pack loading, postprocess stages, and future recursive injections.
- Keep **WS8** and the import work moving in parallel where practical, but do not let import fallback cleanup distract from the extractor architecture reset.
- Treat **WS9** as the clean-up pass that codifies which Rust overrides are still justified after the pack/profile/container work lands.
- Do **WS10** and **WS11** together or back-to-back. Moving ingest/storage boundaries without fixing type ownership will just create a new shared bag somewhere else.
- Land **WS12** after import/package/entrypoint facts are stronger; its goal is to consume better facts, not invent another round of heuristics.
- Treat **WS13** as the next framework-focused pass after the extractor/import/storage boundaries are stable.
- Keep **WS14** behind the above work unless security policy changes are actively blocking indexing work.

## Open questions for the next iteration

- Should project-local query packs live under a fixed convention (for example `.codesight/queries/`) or only through explicit `.codesight.toml` roots?
- How much confidence metadata do downstream tools need from the generic AST fallback before they can safely use it for graph/quality analysis?
- For container languages, do we want one generic segment DSL in Rust, or should every serious host language move to grammar-backed `injections.scm` as soon as a stable grammar exists?
- Which types truly need to remain shared after WS10/WS11? The default assumption should be “fewer than we think.”
- Do we want `indexing/packages` or `analysis/cross_repo` to own manifest/package metadata long-term? The answer should follow who mutates and persists that data, not who reads it most often.
