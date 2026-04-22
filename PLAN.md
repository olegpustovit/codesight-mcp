# Refactor plan — post-backend-collapse cleanup

Context: `codesight-backend` has been deleted; call sites use
`codesight_engine::indexing::Indexer` directly. `src/cli/hooks.rs` now
sources its `CODE_EXTENSIONS` / `DOC_EXTENSIONS` from
`codesight_language_support::{is_code_extension, is_doc_extension}`.
`phase_d_parity.rs` is now `language_parity.rs`. `cargo check
--workspace --tests` is green.

The remaining work is grouped into independent phases. Each phase is
self-contained — it can land as one PR — and ordered by risk/value.
Every phase ends with `cargo nextest run --lib -p <crate>` on the crate
it touches, then `cargo check --workspace --tests` before merge.

Guiding constraints (from AGENTS.md + memory):
- Zero backwards compatibility: rename callers in the same patch. No
  shims, no deprecated aliases, no `// removed` comments.
- Aggressive refactor: skip checks between edits *within* a phase; run
  `cargo check` only at phase boundaries.
- Shared helpers > 3 copies. Trait + registry > growing `match`.
  Maintained crate > hand-rolled util.
- Every language-specific heuristic belongs on `Language` (L0.75).
  Per-lang `const` tables in generic code are a bug.

---

## Phase 1 — Move `tests/parsers/` into `codesight-language-support`

**Why.** 17 parser-level tests live at the workspace root but exercise
only `codesight_language_support::{LanguageRegistry, scm_engine::*,
scm_imports}` and `codesight_core::{StoredSymbol, SymbolKind}`. They
have no business in the binary's test suite.

**Files to move** (17 + 1 module stub):

```
tests/parsers/al.rs
tests/parsers/arrow_functions.rs
tests/parsers/asm_language.rs
tests/parsers/blade.rs
tests/parsers/css.rs
tests/parsers/dart_imports.rs
tests/parsers/grammar_smoke.rs
tests/parsers/json.rs
tests/parsers/languages.rs          (56 KB — biggest)
tests/parsers/lua.rs
tests/parsers/luau_language.rs
tests/parsers/razor.rs
tests/parsers/reference_parser.rs
tests/parsers/sql_language.rs
tests/parsers/swift_parser.rs
tests/parsers/synthetic_repos.rs
tests/parsers/yaml_language.rs
tests/parsers.rs                    (module harness; delete)
```

**Fixtures.** Two trees are consumed only by parser tests:
- `tests/fixtures/code_reference/` → used by `al.rs`, `blade.rs`,
  `razor.rs` via `codesight_mcp::test_support::code_reference_fixtures_dir()`.
- `tests/fixtures/lang_samples/` → used by `languages.rs` via
  `include_str!`.

Both move to `crates/codesight-language-support/tests/fixtures/` (the
crate-level `tests/fixtures/` convention). `include_str!` relative paths
in `languages.rs` need one-level fewer `..`.

**Helper relocation.** Move `code_reference_fixtures_dir()` out of
`src/test_support/` (binary crate) and into either:
- (a) a `#[path = "../support/fixtures.rs"]`-shared helper next to
  the tests in `language-support/tests/`, or
- (b) a tiny `pub mod test_support` in `language-support/src/` gated on
  `#[cfg(any(test, feature = "test-support"))]`.

Prefer (a) — no `cfg` sprawl, no feature flag.

**Cargo**. `codesight-language-support` gets a `[dev-dependencies]` block
for the grammar crates currently pinned in the root `Cargo.toml` dev
deps (`tree-sitter-bash`, `tree-sitter-css`, `tree-sitter-json`,
`tree-sitter-lua`, `tree-sitter-luau`, `tree-sitter-php`,
`tree-sitter-scss`, `tree-sitter-sequel`, `tree-sitter-swift`,
`tree-sitter-yaml`, `tree-sitter-dart`, `tree-sitter-nix`,
`tree-sitter-xml`, `tree-sitter` itself). Drop those from the root
`Cargo.toml` dev deps.

**Acceptance.**
- `tests/parsers/` and `tests/parsers.rs` are gone.
- `tests/fixtures/code_reference/` and `tests/fixtures/lang_samples/`
  are gone; same content now lives in
  `crates/codesight-language-support/tests/fixtures/`.
- `cargo nextest run -p codesight-language-support --tests` runs the
  full parser suite and passes.
- `cargo nextest run --lib` at the root still passes (root test suite
  shrinks by ~17 binaries).
- Root `Cargo.toml` dev-deps table loses the tree-sitter-* pins.

**Risk.** Low. No production code moves; fixture path strings are the
only footgun.

**Rough effort.** 45 min.

---

## Phase 2 — Split `impact_more.rs` and `quality_more.rs`

**Why.** `impact/{...}.rs` + `impact_more.rs` and `quality/{...}.rs` +
`quality_more.rs` are arbitrary buckets. Each `_more` file is a
chronological overflow of handlers that should sit in their tool's
subdirectory.

**Splits.**

`crates/codesight-engine/src/service/agent_v1/impact_more.rs` (21 KB)
→
```
impact/change_velocity.rs       (change_velocity handler)
impact/impact_preview.rs        (impact_preview handler)
impact/rename_safety.rs         (rename_safety handler)
```
(`path_rename.rs` was already deleted per current git status.) Update
`impact/mod.rs` to pull these in and widen its `match` arms.

`crates/codesight-engine/src/service/agent_v1/quality_more.rs` (16 KB)
→
```
quality/hotspots.rs
quality/refactor_targets.rs
quality/broken_links.rs
quality/test_gaps.rs
```
(`doc_coverage.rs` and `repo_health.rs` already exist in that dir.)
Update `quality/mod.rs` accordingly.

The only shared helper in each `_more.rs` (e.g.
`quality_more::check_internal_link`) goes next to the single handler
that still uses it; if multiple new files need it, promote to
`quality/util.rs`.

**Acceptance.**
- `impact_more.rs` and `quality_more.rs` are gone.
- `registry.rs` still dispatches every analysis mode through the
  consolidated `impact/mod.rs` and `quality/mod.rs` entry points.
- `cargo nextest run -E 'binary(impact_) + binary(quality_) +
  binary(blast_radius) + binary(change_velocity)'` passes.

**Risk.** Low. Pure file move — no logic or signatures change.

**Rough effort.** 40 min.

---

## Phase 3 — Push `frameworks/profiles.rs` into per-language providers

**Why.** `crates/codesight-language-support/src/frameworks/profiles.rs`
hardcodes five per-language framework detectors in a central file:
Laravel (PHP), Nuxt / Next / Vue-SPA / React-SPA (JS/TS). Each of
these belongs on the owning `Language` via `framework_providers()`
(the trait method already exists, returning
`&'static [fn() -> Box<dyn FrameworkProvider>]`).

**Plan.**

1. Confirm the `FrameworkProvider` trait surface is sufficient to
   carry `name`, `ignore_patterns`, `entry_point_patterns`,
   `layer_definitions`, plus the per-profile `detect(root)` probe. If
   not, extend it (prefer methods returning `&'static [...]` over
   heap allocs). Keep `FrameworkProfile` / `LayerDefinition` as return
   types in `codesight-language-support::frameworks`.
2. **PHP — Laravel.** Add a provider in
   `languages/php/laravel/provider.rs` (the `laravel/` folder already
   exists). Move `laravel_profile()` logic there. Register via
   `impl Language for Php { fn framework_providers() -> &'static
   [...] { &[laravel_provider_factory] } }`.
3. **JavaScript / TypeScript — SPAs and meta-frameworks.** The four
   JS-family profiles (Nuxt, Next, Vue-SPA, React-SPA) key off
   config-file presence (`next.config.*`, `nuxt.config.*`,
   `vite.config.*`, `src/App.vue` vs `src/App.tsx`). Attach providers
   to `TypeScript` since the detectors inspect `.ts`/`.tsx`-shape
   markers; JS should inherit via `Jsx`/`JavaScript` sharing the same
   factory list (a `pub(super) const JS_FAMILY_FRAMEWORKS: &[...]`
   imported by all four impls).
4. **`frameworks/profiles.rs` becomes a thin dispatch layer.** The
   top-level `detect_framework(root)` iterates
   `builtin_registry().all()` and calls each language's provider
   factories, picking the first that claims the root. The per-language
   precedence from today's file becomes sort order on
   `(Language::id().as_str(), provider.priority())` — explicit, not
   file-ordering-dependent.
5. If no caller still uses `frameworks::profiles::*` directly (they
   shouldn't — detection goes through the trait), delete the legacy
   module.

**Acceptance.**
- `frameworks/profiles.rs` is gone or reduced to the cross-language
  dispatcher.
- Grep proves no per-framework literal (`"laravel"`, `"nuxt"`,
  `"next"`, `"vue-spa"`, `"react-spa"`) lives outside the owning
  language's folder.
- Framework-detection tests in
  `tests/agent_v1_framework_{core,dbt,laravel,nextjs,nuxt}.rs` still
  pass unchanged.

**Risk.** Medium. Touches the `FrameworkProvider` trait surface; watch
for allocation churn in hot paths. Run
`tests/tool_latency_budgets.rs` after to confirm no regression.

**Rough effort.** 2 h.

---

## Phase 4 — Push `packages/walker.rs` pruned dirs onto `Language`

**Why.** `PRUNED_DIR_NAMES` in
`crates/codesight-language-support/src/packages/walker.rs` is a
flat list of per-ecosystem dependency/build-output folders:
`node_modules` (JS/TS), `target` (Rust), `vendor` (PHP/Go),
`__pycache__` / `venv` / `.venv` / `.tox` (Python), `dist` / `build` /
`out` (JS/TS / C#), `bin` / `obj` (C#). Every one of these is a
per-language claim — the owning `Language` should declare it.

**Plan.**

1. Add to the `Language` trait:
   ```rust
   /// Directory names whose subtrees this language's ecosystem
   /// treats as generated / vendored / build-output, and that the
   /// package walker should prune.
   fn pruned_dirs(&self) -> &'static [&'static str] { &[] }
   ```
2. Fill in the obvious owners:
   - `Rust`: `&["target"]`
   - `Python`: `&["__pycache__", "venv", ".venv", ".tox"]`
   - `TypeScript` / `JavaScript`: `&["node_modules", "dist",
     "build", "out"]` (attach once, inherit via the JS family
     module pattern from Phase 3)
   - `Php`: `&["vendor"]`
   - `Go`: `&["vendor"]`
   - `Csharp`: `&["bin", "obj"]`
3. Rewrite `walker::PRUNED_DIR_NAMES` as a runtime-built static (via
   `once_cell::sync::Lazy` or a `OnceLock<Vec<&'static str>>`):
   ```rust
   fn pruned_dirs() -> &'static [&'static str] {
       static DIRS: OnceLock<Vec<&'static str>> = OnceLock::new();
       DIRS.get_or_init(|| {
           let mut out: Vec<&'static str> = crate::builtin_registry()
               .all()
               .flat_map(|l| l.pruned_dirs().iter().copied())
               .collect();
           out.sort_unstable();
           out.dedup();
           out
       })
   }
   ```
   VCS entries (`.git`, `.hg`, `.svn`) and the existing dot-prefix
   rule stay generic in `walker::is_pruned` — those aren't
   language-owned.
4. Delete `PRUNED_DIR_NAMES`; `is_pruned` consumes the helper above.

**Acceptance.**
- No language-named directory literal (`node_modules`, `target`,
  `__pycache__`, `vendor`, `.venv`, `.tox`, `dist`, `build`, `out`,
  `bin`, `obj`) appears in `walker.rs` source.
- `tests/packages_walker.rs` still passes.
- A new targeted test asserts that adding a `pruned_dirs()` override
  on a stub `Language` shows up in the walker output.

**Risk.** Low. Behaviour-equivalent refactor plus one new trait method.

**Rough effort.** 30 min.

---

## Phase 5 — Split the oversized `agent_v1/` files

Each of these is its own mini-phase. Do one per PR.

### 5a. `catalog_search/mod.rs` (33 KB, ~1000 LoC)

Split along the seam between search logic and response shaping:
```
catalog_search/
    mod.rs          (entry + dispatch only, ~100 LoC)
    bm25.rs         (BM25 + scope narrowing)
    fuzzy.rs        (already exists — keep)
    semantic.rs     (already exists — keep)
    regex_route.rs  (query_mode=regex file-text routing)
    response.rs     (JSON shaping + token budgeting + truncation)
    meta.rs         (the `_meta` builder / `tokens_saved` accounting)
```
Handlers remain `impl CodeService` but their bodies move into free
functions in the submodules.

### 5b. `index_admin.rs` (31 KB, ~800 LoC)

Extract the session surface:
```
agent_v1/
    index_admin.rs       (status, invalidate, delete, rebuild,
                          analysis_health — index-only ops)
    session_admin.rs     (session_stats, session_end,
                          snapshot_quality — session lifecycle)
```
The dispatch table in `registry.rs` picks up one new handler file; no
signatures change.

### 5c. `util.rs` (20 KB, ~600 LoC)

This is a kitchen sink. Cut along intent:
```
util/
    mod.rs        (re-exports; keep `pub use` crate-local surface)
    json_shape.rs (symbol/section wrapping, envelope helpers)
    token.rs      (token budgeting math, oversize stubs)
    neighbors.rs  (outline-neighbor walk, heading ancestors)
    scope.rs      (scope/file_pattern/exclude matching)
```

**Acceptance (each sub-phase).**
- Top-level file replaced with a directory; old file gone.
- No function's name or signature changes.
- `cargo nextest run` on the affected integration tests passes.
- Per-file LoC budget: every new file under 500 LoC.

**Risk.** Medium — lots of `pub(super)` / `pub(in crate::service)`
visibility to retarget.

**Rough effort.** 5a 90 min, 5b 45 min, 5c 60 min.

---

## Phase 6 — Wider agent_v1 cleanup

Treat as one pass after 5a/5b/5c. Targets that remain once the big
files are gone:

- **`args.rs` (14 KB) and `validate.rs` (13 KB).** Both are MCP
  infrastructure, not agent-specific. They read `tools.json` and
  coerce/validate request bodies. Leaving them inside
  `agent_v1/` until the contract is version-bumped. *No action this
  round — document and move on.*
- **`reference_check/evidence.rs` (6 KB).** `EvidenceNote` /
  `file_has_fallback_content_reference` logic is analysis, not agent.
  Move the analysis helpers into `codesight-analysis::evidence`; leave
  the agent-facing JSON shaping in
  `reference_check/mod.rs`.
- **`context_tail/` + `context_bundle/`.** Currently parallel shapes.
  Merge `context_tail/` into `context_bundle/` as
  `context_bundle/tail/{query_suggestions,rename_plan,section_context}.rs`
  — one subsystem owns everything "context around a symbol".

**Acceptance.**
- `crates/codesight-engine/src/service/agent_v1/context_tail/` is gone.
- `codesight-analysis::evidence` picks up the migrated helpers, and
  `reference_check/evidence.rs` shrinks to the agent-facing wrapper.
- No `_more`-suffixed files and no files over 500 LoC inside
  `agent_v1/` (excluding `args.rs`, `validate.rs`, `tools.json`
  schemas).

**Risk.** Medium — touches dispatch, JSON shape, and analysis layer
boundary. Requires one `cargo check --workspace --tests` after each
move.

**Rough effort.** 2 h.

---

## Phase 7 — `mcp-rpc` → `rmcp` migration

**Verdict at the time of writing: defer.** 360-line hand-rolled framing
works, ships bounded concurrency + SIGINT/SIGTERM drain that `rmcp`
doesn't give us out of the box, and doesn't drift because the MCP spec
hasn't broken our surface.

Revisit when **any** of the following is true:
1. The MCP spec bumps and our `initialize`/`tools/list`/`tools/call`
   framing lags (diff our `dispatch_method` against the new spec).
2. We need server features we currently don't have: prompts,
   resources, progress notifications, sampling.
3. `rmcp` ships a built-in graceful-shutdown + bounded-concurrency
   middleware we'd be reimplementing otherwise.

**When we do migrate.**

1. Add `rmcp = "<latest>"` dev-first, behind `codesight-engine`'s
   existing `rpc` module boundary. Keep both paths alive in the same
   PR so we can flip back.
2. Implement `rmcp::ServerHandler` on `CodeService` directly —
   `list_tools` parses our frozen `crates/codesight-engine/src/tools.json`
   into `Vec<rmcp::model::Tool>` once at startup; `call_tool` delegates
   to the current `service::dispatch`. No `schemars`, no per-tool
   typed param structs.
3. Wrap `rmcp`'s service runtime with our own `JoinSet + Semaphore`
   for bounded concurrency, and wire `tokio::signal` → `service.cancel()`
   to preserve signal-driven drain. ~30 LoC.
4. Delete `crates/mcp-rpc/` in the same PR. Update `tests/crate_graph.rs`.

**Acceptance.**
- `initialize` / `tools/list` / `tools/call` responses are
  byte-for-byte identical to today against the existing integration
  envelope tests (`tests/envelope_shape.rs`,
  `tests/agent_v1_envelope.rs`).
- `tools.json` is still the contract — no schema derivation.
- Bounded-concurrency + SIGINT/SIGTERM drain behaviour is preserved
  (covered by `tests/tool_latency_budgets.rs`).

**Risk.** High. Protocol layer changes touch every MCP integration
test.

**Rough effort.** 4–6 h.

---

## Ordering + branch strategy

Land in this order — each PR is independent, each leaves the tree
green:

1. **Phase 1** — tests/parsers move (tight, bounded) **← start here**
2. **Phase 2** — impact/quality split (small, bounded)
3. **Phase 4** — walker pruned_dirs (small, per-language trait win)
4. **Phase 3** — frameworks/profiles (bigger per-language trait win)
5. **Phase 5a** — catalog_search/mod.rs split (biggest file first)
6. **Phase 5b** — index_admin split
7. **Phase 5c** — util.rs split
8. **Phase 6** — remaining agent_v1 structural cleanup
9. **Phase 7** — `rmcp` migration (only if triggers above fire)

Phases 1–4 are one session each. Phase 5 is one session per sub-phase.
Phases 6 and 7 each want their own session.

## Global invariants (do not violate mid-refactor)

- Cyclomatic complexity per fn `<= 20`, file LoC `<= 500` soft
  (AGENTS.md thresholds).
- No layer skips: `codesight-language-support` may only depend on
  `codesight-core`; `codesight-engine` never pulls concrete backend
  types through its lib (enforced by `tests/crate_graph.rs`).
- `tools.json` is the frozen MCP contract. Any schema change lands in
  the same patch as its handler and tests.
- Every phase ends with `cargo check --workspace --tests` and a
  targeted `cargo nextest run -p <crate>` on what changed — nothing
  else.
