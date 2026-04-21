# LANGUAGES.md — pluggable per-language support

**Status:** landed 2026-04-20. Implementation log at §11.

## The one rule, unchanged

Everything a language needs lives in
`crates/codesight-language-support/src/languages/<lang>/`. That is the
whole mental model. Nothing else gets to own language-specific behavior.

---

## 1. Why

Before this refactor, language-specific behavior was split across
`codesight-analysis`, several storage call sites, and separate
imports / extractors / framework / LSP-backend crates. The result was not
"some duplication." The result was contradictory behavior that compiled.

Representative drift points that motivated the consolidation:

| Concern | Former home |
|---|---|
| Path → language identity | imports, analysis, LSP backend, extractors |
| Entrypoints / test files / fixtures | `codesight-analysis` |
| Import extraction / alias loading / resolution | imports crate |
| Package / workspace manifest parsing | imports crate, plus PHP logic in `codesight-analysis` |
| Type-density / signature heuristics / visibility rules | `codesight-analysis` |
| Query-pack catalogs / overlays / special path classifiers | extractors crate |
| Framework metadata / extra imports / layer hints | framework crate |
| Raw language string branches in storage / analysis | `codesight-storage`, `codesight-analysis`, extractor internals |

Failures observed in that shape:

1. Two `is_test_file` definitions disagree.
2. `tsx` exists in extraction catalogs but is absent from `language_profile`, so type-density is wrong.
3. Several grammars parse successfully but disappear from downstream analyses because no other table recognizes them.
4. Workspace-root and package-root heuristics have already been duplicated once.
5. YAML-hosted semantic overlays such as OpenAPI and Ansible exist in
   extractors, but the proposal draft did not give them a first-class
   host-language representation.

The fix is not "add better comments." The fix is: one place owns
language behavior, and every crate above it calls through that one place.

---

## 2. Target

**One crate:** `crates/codesight-language-support`.

Every builtin language, every host-language profile/framework helper,
every wrapper language, every query pack, every content probe, and every
package/import/liveness rule lives under `src/languages/<lang>/`.

### 2.1 Layout

```text
crates/codesight-language-support/
├── src/
│   ├── lib.rs
│   ├── registry.rs
│   ├── manifest.rs
│   ├── simple.rs
│   ├── traits/
│   │   ├── extraction.rs
│   │   ├── imports.rs
│   │   ├── packages.rs
│   │   ├── liveness.rs
│   │   ├── visibility.rs
│   │   └── complexity.rs
│   └── languages/
│       ├── mod.rs
│       ├── rust/
│       │   ├── mod.rs
│       │   ├── imports.rs
│       │   ├── packages.rs
│       │   ├── liveness.rs
│       │   ├── visibility.rs
│       │   ├── complexity.rs
│       │   └── queries/{tags,injections}.scm
│       ├── typescript/
│       │   ├── mod.rs
│       │   ├── imports.rs
│       │   ├── nextjs/
│       │   └── queries/{tags,injections}.scm
│       ├── vue/
│       │   ├── mod.rs
│       │   ├── extraction.rs
│       │   ├── nuxt/
│       │   └── queries/{tags,injections}.scm
│       ├── yaml/
│       │   ├── mod.rs
│       │   ├── ansible/
│       │   ├── openapi/
│       │   └── queries/{tags,injections}.scm
│       ├── json.rs
│       ├── bash.rs
│       ├── sql.rs
│       └── ...
```

Builtin query packs live inside the owning crate at
`crates/codesight-language-support/src/languages/<lang>/queries/` via
`include_str!`. The previous workspace-root `runtime/queries/` directory
is gone. Workspace-local overlays (`.codesight/queries/`) are preserved
as a workspace feature and loaded by the registry. See §3.5.

### 2.2 Supported implementation shapes

There are five valid implementation shapes. This closes a gap in the
first draft: not every language is a plain SCM-backed language.

1. **`SimpleLanguage`**
   Grammar + builtin query pack + all default heuristics. One file.
2. **Folder language**
   Any language with custom import, liveness, package, visibility, or
   complexity behavior.
3. **Host profile / framework**
   A path/layout-selected semantic overlay owned by a host language
   folder. Example: `yaml/ansible/`, `yaml/openapi/`, `typescript/nextjs/`,
   `sql/dbt/`. A host profile is not a standalone language id.
4. **Wrapper language**
   A container file that delegates extraction to child languages.
   Example: Vue, Blade, Razor, EJS.
5. **Manual language**
   No grammar or no sufficient grammar. Hand-rolled extraction is still a
   first-class language implementation, not a residual bucket somewhere
   else.

**Rule:** current residual buckets such as `container_boundary/`,
`multi_dialect/`, `host_semantics/`, and `no_grammar/` disappear as
top-level concepts. Every current residual implementation is promoted
into one of the five language shapes above and moved into a real host
language folder under `languages/<lang>/`.

### 2.3 Layering

```text
L0      codesight-core
L0.75   codesight-language-support
L0.5    codesight-backend
L1      codesight-analysis
L1      codesight-retrieval
L1      codesight-engine
L2      mcp-rpc
```

`codesight-language-support` may depend on `codesight-core`,
tree-sitter crates, and small parsing crates already required by per-lang
logic. No crate above L0.75 gets to own a language table or branch on a
raw language string.

---

## 3. Contract

### 3.1 Top-level trait

```rust
pub trait LanguageSupport: Send + Sync {
    fn id(&self) -> LanguageId;
    fn manifest(&self) -> &'static LanguageManifest;

    fn extraction(&self)  -> &dyn SymbolExtraction { &NoopExtraction }
    fn imports(&self)     -> &dyn ImportResolution { &NoopImports }
    fn packages(&self)    -> &dyn PackageManifest  { &NoopPackages }
    fn liveness(&self)    -> &dyn LivenessPolicy   { &NoopLiveness }
    fn visibility(&self)  -> &dyn VisibilityModel  { &PermissiveVisibility }
    fn complexity(&self)  -> &dyn ComplexityModel  { &DefaultComplexity }
}
```

The six traits are the **analysis axes**. Detection and backend-routing
hooks belong in `LanguageManifest`, not in extra top-level traits.

### 3.2 `LanguageManifest`

The first draft hand-waved `manifest()`. That is not enough. The
manifest must be explicit because it replaces several existing tables.

```rust
pub struct LanguageManifest {
    pub display_name: &'static str,
    pub aliases: &'static [&'static str],
    pub extensions: &'static [&'static str],
    pub filename_overrides: &'static [&'static str],
    pub compound_extensions: &'static [&'static str],
    pub grammar: Option<GrammarSpec>,
    pub backend_ids: BackendIds,
    pub profiles: &'static [HostProfile],
    pub content_probe: Option<fn(path: &Path, source: &str) -> Option<LanguageId>>,
}

pub struct GrammarSpec {
    pub grammar_name: &'static str,
    pub loader: fn() -> tree_sitter::Language,
    pub tags_scm: &'static str,
    pub injections_scm: Option<&'static str>,
}

pub struct BackendIds {
    pub lsp: Option<&'static str>,
}

pub struct ProfileId(&'static str);

pub struct ResolvedLanguage {
    pub host: LanguageId,
    pub profile: Option<ProfileId>,
}

pub struct HostProfile {
    pub id: ProfileId,
    pub priority: u16,
    pub matches_path: fn(path: &Path) -> bool,
    pub applies_to: &'static [LanguageId],
}
```

Required behavior of the manifest:

1. `extensions`, `filename_overrides`, and `compound_extensions` replace
   every current path-identity table.
2. `grammar` replaces the builtin grammar catalog. It may be `None` only
   for manual languages or host profiles that delegate to a host
   extractor.
3. `backend_ids.lsp` replaces `LspBackend::language_for`.
4. `profiles` makes host-language semantic overlays first-class instead
   of hidden extractor special cases.
5. `content_probe` lets the primary path match reclassify ambiguous
   cases such as `.h` when source text is available.

`SimpleLanguage` is just:

```rust
pub struct SimpleLanguage {
    pub id: LanguageId,
    pub manifest: &'static LanguageManifest,
}
```

with the blanket default axis implementations.

### 3.3 Detection pipeline

This was the biggest missing piece in the first draft. `identify(path)`
alone is underspecified and cannot represent current behavior.

The registry exposes both host-only and profile-aware detection entry
points:

```rust
pub fn identify_path(&self, path: &Path) -> LanguageId;
pub fn identify_with_source(&self, path: &Path, source: &str) -> LanguageId;
pub fn classify_path(&self, path: &Path) -> ResolvedLanguage;
pub fn classify_with_source(&self, path: &Path, source: &str) -> ResolvedLanguage;
```

Detection order is fixed:

1. **Exact filename override**
   `Dockerfile`, `Justfile`, `Cargo.toml`, and similar exact-name files.
2. **Compound extension**
   `.blade.php`, `.d.ts`, `.module.css`, and similar longer suffixes
   before single-extension fallback.
3. **Workspace `extra_extensions`**
   From `.codesight.toml`; normalized to lowercase with the leading dot
   stripped, matching current behavior.
4. **Simple extension**
   Plain extension match.
5. **Content probe**
   Only in `identify_with_source`. It runs on the path-selected
   candidate and may return a different `LanguageId`. `.h` is the
   canonical example.
6. **Host-profile match**
   Evaluate the matched host language's `profiles` and attach the
   highest-priority matching `ProfileId`. If multiple profiles match,
   lowest `priority` wins.
7. **`UNKNOWN_LANGUAGE`**
   Only after all above fail.

Two invariants:

1. Host-language detection is separate from host-profile attachment.
   Ansible/OpenAPI are YAML-family profiles, not standalone languages.
2. Content probes are a tie-breaker, not a primary classifier. They run
   only for ambiguous cases and only in the source-aware path.

### 3.4 Registry surface

```rust
pub struct LanguageRegistry {
    by_id: HashMap<LanguageId, Arc<dyn LanguageSupport>>,
    by_filename: HashMap<&'static str, LanguageId>,
    by_compound_ext: HashMap<&'static str, LanguageId>,
    by_ext: HashMap<&'static str, LanguageId>,
    curated: BTreeSet<LanguageId>,
}

impl LanguageRegistry {
    pub fn builtin() -> Self;
    pub fn for_workspace_root(root: &Path) -> Result<Self>;
    pub fn get(&self, id: LanguageId) -> Option<&dyn LanguageSupport>;
    pub fn for_path(&self, path: &Path) -> Option<&dyn LanguageSupport>;
    pub fn for_file(&self, path: &Path, source: Option<&str>) -> Option<&dyn LanguageSupport>;
    pub fn identify_path(&self, path: &Path) -> LanguageId;
    pub fn identify_with_source(&self, path: &Path, source: &str) -> LanguageId;
    pub fn classify_path(&self, path: &Path) -> ResolvedLanguage;
    pub fn classify_with_source(&self, path: &Path, source: &str) -> ResolvedLanguage;
    pub fn all(&self) -> impl Iterator<Item = &dyn LanguageSupport>;
    pub fn curated_languages(&self) -> impl Iterator<Item = LanguageId>;
    pub fn register(&mut self, language: Arc<dyn LanguageSupport>);
    pub fn apply_extra_extensions(&mut self, mappings: &HashMap<String, LanguageId>) -> Vec<String>;
}
```

`for_workspace_root(root)` is required. It must preserve both current
workspace behaviors:

1. query-pack overlays from `.codesight/queries` and explicit
   `query_pack_roots` in `.codesight.toml`;
2. `extra_extensions` from `.codesight.toml`.

The registry also owns the existing "best extractor per language"
selection semantics. For language-level capability lookup, precedence
stays:

```text
CuratedQueryPack > GenericAstFallback > ManualOverride > Profiled
```

That precedence is independent of **profile attachment**. Host-language
matching picks the language for a file; host-profile matching attaches
optional semantic overlays; capability precedence picks the best
provider for the host language id.

### 3.5 Query packs, overlays, and `extra_extensions`

This is another gap in the first draft. Moving builtin query packs into
language folders must not delete workspace override behavior.

Builtin layout:

```text
languages/<lang>/queries/tags.scm
languages/<lang>/queries/injections.scm   # optional
```

Workspace-local overlay layout stays:

```text
.codesight/queries/<pack-id>/
  tags.scm
  injections.scm      # optional
  pack.toml           # optional, same schema as today
```

`.codesight.toml` stays the workspace-level hook for:

1. `query_pack_roots`
2. `extra_extensions`

The overlay metadata schema stays the same as current
`QueryPackMetadataFile`:

```toml
language = "..."
extensions = ["..."]
grammar = "..."
override_extractor = "..."
```

Validation rules stay explicit:

1. Every pack must declare `extensions`.
2. Every pack must declare either `grammar` or `override_extractor`.
3. `override_extractor`, if present, must name a language support
   implementation that is actually registered.
4. Overlay `language` must resolve to a real `LanguageId`.

### 3.6 Frameworks live under the host language

`codesight-framework` goes away as a separate crate, but the actual
framework semantics do **not** shrink. Each framework keeps all the
current behaviors; it just moves under the host language.

Current `FrameworkProvider` responsibilities remain valid:

1. `detect(root)`
2. `load(root)`
3. `get_file_context(rel_path)`
4. `get_metadata()`
5. `get_extra_imports()`

Current `FrameworkProfile` data also remains valid:

1. `ignore_patterns`
2. `entry_point_patterns`
3. `layer_definitions`

So the concrete rule is:

- framework-specific code lives in `languages/<host>/<framework>/`
- framework-specific metadata feeds back into the host language's
  `liveness`, `packages`, `imports`, and optionally `extraction`
- no separate top-level framework registry crate remains

### 3.7 Wrapper languages and host profiles

Wrapper languages and host profiles are both real implementation shapes
with their own folders. They are not residual cases.

**Wrapper language rule**

- own `LanguageId`
- own `LanguageManifest`
- own extraction implementation
- may delegate to child languages via `LanguageRegistry`
- must still emit canonical `StoredSymbol` records with real
  `SymbolKind`, `ExtractionTier`, ranges, summaries, and source hashes

**Host profile rule**

- live under the host language folder
- own `ProfileId`
- attach through `LanguageManifest.profiles`
- run on top of a resolved host language, never instead of it
- may contribute extraction, imports, packages, liveness, visibility, or
  complexity overrides to the host language context

OpenAPI is the important example. It cannot be modeled as "a language
of its own." It is a YAML-family schema profile that may also attach to a
JSON host, and the spec must encode that directly without inventing a
fake top-level `openapi` language.

Embedded DSLs still use `injections.scm`. No extra trait is needed.

---

## 4. Generalized heuristics every language owns

This section makes the spec implementation-ready. These are the shared
questions every language must answer, even if the answer is the default.

### 4.1 `SymbolExtraction`

Owns:

1. primary extraction strategy: curated query pack, generic AST fallback,
   manual extraction, or wrapper delegation
2. docstring / doc-comment capture
3. canonical `SymbolKind` mapping
4. emitted `ExtractionTier`
5. wrapper-language region splitting and child delegation
6. any language-specific source normalization needed before extraction

Hard rule: extraction code may rewrite language ids, ranges, or
qualified-name assembly, but it must still emit canonical
`SymbolKind` values and preserve `ExtractionTier` semantics.

### 4.2 `ImportResolution`

Owns:

1. whether the language supports import extraction at all
2. extraction mode: tree-sitter first vs fallback only
3. import extraction from source text
4. workspace alias loading
5. specifier resolution to workspace files
6. bundle/display formatting for imports in retrieval views
7. import-edge fact quality, if the language needs custom quality rules

Examples that must move here:

- Rust `use` resolution
- JS/TS path and `tsconfig` alias handling
- Python relative-import handling
- Go display-shape preservation for import blocks

### 4.3 `PackageManifest`

Owns:

1. package-name discovery for a directory
2. workspace-member discovery for a project root
3. root-package extraction from an import specifier
4. framework-contributed workspace membership if applicable

Examples that must move here:

- Cargo workspace crate detection
- `package.json`, `pyproject.toml`, `go.mod`, `.csproj` package names
- PHP PSR-4 namespace/package logic

### 4.4 `LivenessPolicy`

Owns:

1. entrypoint basenames
2. main-variant detection
3. content-inspection entrypoints such as Python `__main__`
4. project-root entrypoint discovery
5. dispatch-reachable symbol discovery
6. test-file detection
7. test-symbol detection
8. fixture detection
9. conventional directories and basenames

This is the single home for "is this live code or not?"

### 4.5 `VisibilityModel`

Owns:

1. restricted-visibility detection
2. any language-specific public/internal/private interpretation used by
   agent-readiness or blast-radius metrics

If a language has no special visibility model, it uses the permissive
default.

### 4.6 `ComplexityModel`

Owns:

1. language family
2. nesting mode
3. type-density heuristics
4. signature-shape measurement
5. weak-type scoring or exact-type scoring if the language cares

This replaces the current scattered `language_profile`,
`measured_type_density`, `measure_go_signature`,
`measure_php_signature`, and similar `match language { ... }` logic.

### 4.7 Framework-owned heuristics

Framework-specific behavior is still language behavior. It must live
under the host language and feed back into the six axes above.

Every framework helper may contribute:

1. extra imports
2. metadata
3. file context
4. ignore patterns
5. entrypoint patterns
6. layer definitions

This is critical because those are generalized heuristics, not just
"nice to have metadata." They affect dead-code, layer checks,
retrieval, and package graphs.

---

## 5. Migration table

| Today | Tomorrow |
|---|---|
| `language_for_path`, `language_profile`, `LspBackend::language_for`, builtin grammar catalog | `LanguageRegistry::{identify_path, identify_with_source}` + `LanguageManifest`. Delete the old tables. |
| `ENTRYPOINT_BASENAMES`, `detect_main_guard_files`, `detect_cargo_target_files`, `FOREIGN_TRAIT_BASES`, duplicate `is_test_file` logic | `languages/<lang>/liveness.rs`. |
| `IMPORT_EXTRACTION_CAPABILITIES`, import extraction `match`, resolver helpers, bundle display helpers | `languages/<lang>/imports.rs`. |
| `packages/{rust,python,javascript,go,dotnet}.rs`, `psr4.rs`, workspace-package walkers | `languages/<lang>/packages.rs`. |
| `measured_type_density`, language family/nesting, per-language signature heuristics, weak typing rules | `languages/<lang>/complexity.rs`. |
| `has_restricted_visibility` | `languages/<lang>/visibility.rs`. |
| `classify_special_path`, `LanguageProfileRegistry`, profiled extractor wrappers | `languages/yaml/openapi/`, `languages/yaml/ansible/`, and any future host-language profiles via `LanguageManifest.profiles`. OpenAPI may attach to JSON hosts through `HostProfile.applies_to`. |
| `cpp_header_language_fallback.rs` | `languages/cpp/` owns `.h` for path-only detection; its `content_probe` may redirect source-aware detection to `c`. |
| `QueryPackCatalog::for_workspace_root`, `QueryPackMetadataFile`, overlay validation | `LanguageRegistry::for_workspace_root` plus a small query-pack loader module in the new crate. Behavior preserved. |
| `project_query_pack_roots`, `.codesight/queries`, `project_extra_extensions`, `apply_extra_extensions` | Registry-owned workspace overlay support. Behavior preserved. |
| Manual / residual extractors (`scss`, `sass`, `less`, `styl`, `luau`, `asm`, `blade`, `razor`, `ejs`, `verse`, etc.) | Each promoted into `languages/<lang>/` as `SimpleLanguage`, folder language, wrapper language, or manual language. No residual bucket remains. |
| `codesight-framework/{laravel,dbt,nextjs,nuxt}` and `FrameworkProfile` | `languages/<host>/<framework>/`. Crate deleted; responsibilities preserved. |
| `runtime/queries/<lang>/` | `languages/<lang>/queries/`. Builtin packs move; workspace overlays stay workspace-local. |
| `StoredSymbol.language: String`, `IndexedFileUpdate.language: String`, DB language strings, path-based analysis fallback | Typed `ResolvedLanguage { host: LanguageId, profile: Option<ProfileId> }` or an equivalent split representation at ingest, storage, reads, and analysis boundaries. No raw path-extension inference in downstream analyses. |
| string equality such as `s.language == "rust"` in generic engines | call through `LanguageId` / `LanguageRegistry`; move Rust-specific logic into Rust's folder or query pack. |

---

## 6. Example implementations

### 6.1 Rust

```rust
pub struct Rust;

static MANIFEST: LanguageManifest = LanguageManifest {
    display_name: "Rust",
    aliases: &["rs"],
    extensions: &["rs"],
    filename_overrides: &[],
    compound_extensions: &[],
    grammar: Some(GrammarSpec {
        grammar_name: "rust",
        loader: || tree_sitter_rust::LANGUAGE.into(),
        tags_scm: include_str!("queries/tags.scm"),
        injections_scm: Some(include_str!("queries/injections.scm")),
    }),
    backend_ids: BackendIds { lsp: Some("rust") },
    profiles: &[],
    content_probe: None,
};

impl LanguageSupport for Rust {
    fn id(&self) -> LanguageId { RUST }
    fn manifest(&self) -> &'static LanguageManifest { &MANIFEST }
    fn imports(&self) -> &dyn ImportResolution { self }
    fn packages(&self) -> &dyn PackageManifest { self }
    fn liveness(&self) -> &dyn LivenessPolicy { self }
    fn visibility(&self) -> &dyn VisibilityModel { self }
    fn complexity(&self) -> &dyn ComplexityModel { self }
}
```

### 6.2 OpenAPI

```rust
// languages/yaml/openapi/mod.rs
pub static OPENAPI_PROFILE: HostProfile = HostProfile {
    id: ProfileId("openapi"),
    priority: 10,
    matches_path: is_openapi_path,
    applies_to: &[YAML, JSON],
};
```

And the YAML host owns it:

```rust
static YAML_MANIFEST: LanguageManifest = LanguageManifest {
    display_name: "YAML",
    aliases: &["yml"],
    extensions: &["yaml", "yml"],
    filename_overrides: &[],
    compound_extensions: &[],
    grammar: Some(YAML_GRAMMAR),
    backend_ids: BackendIds { lsp: Some("yaml") },
    profiles: &[OPENAPI_PROFILE, ANSIBLE_PROFILE],
    content_probe: None,
};
```

That is the shape the first draft was missing: OpenAPI and Ansible stay
under YAML ownership instead of becoming fake standalone languages.

---

## 7. Drift prevention

Drift is prevented by API shape and by tests.

### 7.1 API invariants

1. `LanguageId` is a newtype, not a string alias.
2. Ingest, storage, and analysis models carry typed host-language
   identity plus optional typed profile identity, not raw strings.
3. Code above `codesight-language-support` does not expose APIs that
   accept bare language strings for builtin behavior.
4. Registry lookup returns `&dyn LanguageSupport`, not raw metadata maps.

### 7.2 Required tests

The first draft's "one sweep test" is too weak. The minimum required
test matrix is:

1. **Builtin language registration**
   Every builtin language folder registers exactly one `LanguageId`.
2. **Builtin query-pack parity**
   Every builtin `languages/<lang>/queries/tags.scm` is reachable from a
   registered language.
3. **Curated language parity**
   `registry.curated_languages()` matches the set of curated builtin
   packs after migration.
4. **Host-profile precedence**
   OpenAPI and Ansible attach the correct profile before plain
   YAML-family host heuristics run.
5. **Compound extension precedence**
   `.blade.php` resolves before `.php`.
6. **Content-probe disambiguation**
   `.h` with plain C constructs resolves to `c`; `.h` with C++ constructs
   resolves to `cpp`.
7. **Workspace overlay loading**
   `.codesight/queries` and `.codesight.toml.query_pack_roots` overlay
   builtin packs exactly once and in deterministic order.
8. **Overlay metadata validation**
   `extensions` is required; one of `grammar` or `override_extractor` is
   required.
9. **`extra_extensions` validation**
   empty extension, dot-only extension, empty language, and unknown
   language all emit warnings and do not register.
10. **Injection-target validity**
    every `injections.scm` child-language target resolves to a real
    `LanguageId`.
11. **Framework parity**
    framework helpers still produce entrypoint patterns, ignore patterns,
    extra imports, and layer definitions after the move.
12. **No raw downstream inference**
    analysis/storage code paths no longer derive host language or
    profile by `path.ends_with(...)`; the typed identity comes from
    ingest.

No grep-based lint is the primary safety net. The primary safety net is
that the behavior is routed through one crate and proven by the matrix
above.

---

## 8. Migration plan — five PRs

The original four-PR plan was missing a distinct storage/typing phase.
That is risky because it lets raw string branches survive behind the new
registry. Use five PRs instead.

### A. Scaffold

Create `codesight-language-support` with:

1. `LanguageId`
2. `LanguageManifest`
3. six traits
4. `SimpleLanguage`
5. empty `LanguageRegistry`

Zero behavior change.

### B. Identity and detection

Move all language identity data into manifests and registry logic:

1. extensions
2. filename overrides
3. compound extensions
4. backend ids
5. host profiles
6. content probes

Delete old identity tables only after parity tests pass.

### C. Extraction and query packs

Move builtin query packs into language folders, preserve workspace
overlays, and promote profiled/manual/wrapper languages into first-class
implementations.

This phase also carries:

1. overlay metadata validation
2. `extra_extensions`
3. curated-language parity
4. injection-target validation

### D. Heuristics and frameworks

Move imports, packages, liveness, visibility, complexity, and framework
helpers into host language folders. Delete `codesight-framework` in this
phase.

### E. Typed downstream cleanup

Switch ingest, storage, and analysis to `LanguageId` end-to-end and
delete remaining raw string/path-extension branches outside the language
crate.

This is the fail-safe phase. If any downstream code still has to ask
"what language is this file?" after ingest, Phase E is incomplete.

---

## 9. What stays the same

1. Agent-facing MCP tool surface.
2. `CodeBackend` as the backend abstraction.
3. Tree-sitter and LSP as backend choices.
4. Tree-sitter injection semantics.
5. Workspace-local query-pack overlays and `.codesight.toml` hooks.

This is a refactor of language ownership, not a redesign of the MCP
surface or backend model.

---

## 10. One-line summary

One folder per language still is the whole mental model, but the spec is
only implementation-ready if that folder also owns profiled languages,
wrapper/manual languages, workspace overlays, framework heuristics,
backend ids, content probes, and typed downstream language identity.

---

## 11. Implementation log

### Phase A — scaffold (landed 2026-04-20)

**Status:** complete. Zero behavior change; callers still use the old
string tables.

**What landed:**

- New crate `crates/codesight-language-support/` at L0.75 in the workspace.
- `LanguageId` newtype (Copy `&'static str` wrapper) + 36 builtin
  `LanguageId` constants covering the set currently recognized by
  `runtime/queries/<lang>/` and `language_for_path`.
- `ProfileId`, `ResolvedLanguage { host, profile }` (§3.3).
- `LanguageManifest`, `GrammarSpec`, `BackendIds`, `HostProfile` with the
  exact field layout from §3.2.
- Six axis traits — `SymbolExtraction`, `ImportResolution`,
  `PackageManifest`, `LivenessPolicy`, `VisibilityModel`,
  `ComplexityModel` — each with a default impl (`NoopExtraction`,
  `NoopImports`, `NoopPackages`, `NoopLiveness`, `PermissiveVisibility`,
  `DefaultComplexity`).
- Top-level `LanguageSupport` trait with blanket defaults that route to
  every `Noop*` / permissive / default impl (§3.1 reference form).
- `SimpleLanguage` struct (§2.2 case 1).
- `LanguageRegistry` with the complete public surface §3.4 specifies:
  `builtin`, `for_workspace_root`, `get`, `for_path`, `for_file`,
  `identify_path`, `identify_with_source`, `classify_path`,
  `classify_with_source`, `all`, `curated_languages`, `register`,
  `apply_extra_extensions`. All detection methods return
  `UNKNOWN_LANGUAGE` in Phase A; `register` already populates the
  `by_filename` / `by_compound_ext` / `by_ext` maps from manifests so
  Phase B only has to wire the detection order.
- `tests/crate_graph.rs` extended with an L0.75 invariant: the new crate
  cannot depend on any of the 11 workspace crates above it. Invariant
  verified.
- 7 unit tests in the crate itself covering newtype identity, profile
  attachment, registration under manifest extensions, and
  `apply_extra_extensions` validation failures.

**Parity:** none — Phase A intentionally has no behavioral impact.
`language_for_path`, `LspBackend::language_for`, `language_profile`,
`ENTRYPOINT_BASENAMES`, `QueryPackCatalog`, etc. are all still the
authoritative tables. Downstream crates are unchanged.

**Verified:**

- `cargo check --workspace` — green.
- `cargo nextest run -p codesight-language-support --lib` — 7/7 pass.
- `cargo nextest run --test crate_graph` — pass.
- `cargo clippy -p codesight-language-support --all-targets -- -D warnings`
  — clean.

**Next — Phase B (identity + detection):** move `extensions`,
`filename_overrides`, `compound_extensions`, `backend_ids`, `profiles`,
and `content_probe` data into per-language manifests, implement the
§3.3 detection order in `LanguageRegistry::identify_*`, swap call sites
at `language_for_path` / `LspBackend::language_for` / extractor
`build_language_lookup` over to the registry, and delete the old tables
only after §7.2 parity tests (items 1, 5, 6) pass.

### Phase B — identity and detection (landed 2026-04-20)

**Status:** complete. `codesight-imports::language_for_path` is now a
thin adapter over the registry; the remaining identity tables
(`LspBackend::language_for`, `ExtractorRegistry::extractor_for_path`,
`classify_special_path`) stay in place but the registry is the new
ground truth and subsequent phases delete those tables.

**What landed:**

- 44 builtin `LanguageManifest` instances under
  `crates/codesight-language-support/src/languages/builtins.rs` covering
  every language previously recognized by `language_for_path`, the
  `GrammarCatalog`, the residual extractors, and `language_profile`. Each
  manifest carries: `display_name`, `aliases`, `extensions`,
  `filename_overrides`, `compound_extensions`, `backend_ids.lsp`,
  `profiles`, `content_probe`. Grammar bindings stay `None` in Phase B;
  Phase C wires `GrammarSpec` when the query packs move.
- New `LanguageId` constants that were missing from the Phase A list:
  `STYL`, `ERLANG`, `FORTRAN`, `GLEAM`, `JSX`, `MARKDOWN`, `TOML`,
  `TREE_SITTER_QUERY`. `LanguageId` / `ProfileId` gained `Ord +
  PartialOrd` (for `BTreeSet`-backed `curated` storage) and `HostProfile`
  gained `Copy + Clone` (so profile-set slices can share instances across
  host manifests without allocation).
- `crates/codesight-language-support/src/languages/yaml.rs` — OpenAPI
  and Ansible as first-class `HostProfile`s. OpenAPI applies to both
  `YAML` and `JSON` via `applies_to`, priority `0`. Ansible applies to
  `YAML` only, priority `10`. Replaces
  `codesight-extractors::extractors::classifier::classify_special_path`
  at the identity layer (the extractor copy stays until Phase C deletes
  the profiled-extractor path).
- `crates/codesight-language-support/src/languages/cpp.rs` —
  `cpp_header_content_probe` reproduces the
  `cpp_header_language_fallback::has_cpp_constructs` heuristic, attached
  to the `CPP` manifest via `content_probe`. `.h` resolves to `CPP` for
  `identify_path`, and to `C` for `identify_with_source` when no C++ constructs
  are present. Legacy duplicate in `codesight-extractors` stays until
  Phase D.
- Full §3.3 detection pipeline in `LanguageRegistry`:
  1. exact filename override (`by_filename`),
  2. compound extension (`by_compound_ext`; `.d.ts`, `.blade.php`),
  3. workspace `extra_extensions` (`extra_ext`),
  4. simple extension (`by_ext`),
  5. content probe (source-aware path only),
  6. host-profile attachment via `classify_path` /
     `classify_with_source` — lowest `priority` wins, ties broken by
     insertion order.
- `LanguageRegistry::builtin` now actually registers languages;
  `LanguageRegistry::empty` exposed for tests that need zero-state.
  `mark_curated` populates the curated set from `register_builtins` so
  `curated_languages()` is stable.
- `codesight_language_support::builtin_registry()` — process-wide
  `OnceLock<LanguageRegistry>` used by downstream adapters that need
  cheap stable lookup.
- `codesight-imports::language_for_path` rewritten as a thin adapter over
  `builtin_registry().identify_path`. Downstream string equality ("python",
  "rust", "yaml", …) continues to work because `LanguageId::as_str`
  returns the same canonical names.
- `tests/crate_graph.rs` — the L1 assertion for `codesight-imports`
  updated to permit the new `codesight-language-support` dependency.
  Every other layering invariant preserved.

**Parity tests (§7.2 items 1, 5, 6) — landed:**

- `registry::tests::compound_extension_wins_over_single` — `.blade.php`
  resolves to `BLADE` before `.php` triggers PHP; `.d.ts` resolves to
  `TYPESCRIPT` via the compound path.
- `registry::tests::h_header_content_probe_downgrades_to_c` and
  `h_header_content_probe_keeps_cpp_for_namespaced_code` — `.h` with
  plain-C constructs returns `C`; `.h` with `namespace n { ... }` stays
  `CPP`.
- `registry::tests::openapi_beats_ansible_on_contested_yaml`,
  `openapi_profile_attaches_to_yaml`, `openapi_profile_attaches_to_json`,
  `ansible_profile_matches_conventional_layout` — host-profile
  precedence + `applies_to` dispatch.
- `registry::tests::builtin_registry_registers_no_unknown_entries` — the
  builtin registry has ≥ 30 languages and none of them shadow
  `UNKNOWN_LANGUAGE`.

**Verified:**

- `cargo check --workspace` — green.
- `cargo nextest run -p codesight-language-support --lib` — 26/26 pass
  (19 new Phase B tests, 7 Phase A tests).
- `cargo nextest run -p codesight-imports --lib` — 65/65 pass; no
  behavior change for any import extraction call site.
- `cargo nextest run --test crate_graph` — pass.

**Not yet landed (Phase C/D/E work):**

- Grammar bindings are still `None` on every manifest. The SCM query
  packs and `GrammarCatalog` stay under `codesight-extractors` until
  Phase C. Nothing reads `manifest.grammar` yet.
- `LspBackend::language_for` still scans its own config table. Phase D
  will route it through `builtin_registry().get(id)?.manifest().backend_ids.lsp`.
- `ExtractorRegistry::extractor_for_path` and
  `classify_special_path` are still the authoritative routing layer for
  extraction. Phase C replaces them by driving extractor selection from
  `LanguageRegistry::classify_*`.
- `codesight-analysis::agent_readiness::language_profile` and the
  `ENTRYPOINT_BASENAMES` / `is_test_file` tables are untouched. Phase D
  migrates them into per-language `ComplexityModel` / `LivenessPolicy`
  impls.
- Per-language folders (rust, typescript, …) beyond `yaml/` and `cpp.rs`
  are still represented through the shared `builtins.rs` macro. Each
  language is promoted to its own folder as Phase D adds trait axes —
  §2.2 case 1 (`SimpleLanguage` = one file) means that is not a
  regression.
- `StoredSymbol.language: String` and `IndexedFileUpdate.language: String`
  still carry raw strings. Phase E replaces these with
  `ResolvedLanguage { host, profile }` end-to-end.
- `IMPORT_EXTRACTION_CAPABILITIES` in
  `codesight-imports::imports::mod` maintains a parallel
  alias/mode table (10 entries, `canonical_language` + `aliases` +
  `ImportExtractionMode`). Aliases now live on `LanguageManifest`;
  extraction mode belongs on the `ImportResolution` trait — both fold
  into Phase C when `ImportResolution` moves per-language.
- `language_for_path` still returns `&'static str` to preserve string-
  equality downstream. Phase E flips the return type to `LanguageId`
  and rewrites every caller in the same patch (no backwards-compat
  shim, per AGENTS.md).

**Next — Phase C (extraction + query packs):** move the `GrammarCatalog`
entries and `runtime/queries/<lang>/` packs into each language's
manifest (`grammar: Some(GrammarSpec { ... })`). Preserve
workspace-local `.codesight/queries/` overlays and `extra_extensions`
via `LanguageRegistry::for_workspace_root`. Promote `classify_special_path`
/ `ProfiledExtractor` into the host-profile path. Gate on §7.2 items 2,
3, 7, 8, 9, 10.

### Phase C — extraction and query packs (landed 2026-04-20)

**Status:** complete. Grammar ownership and the builtin query-pack table
live in `codesight-language-support`; every manifest with a tree-sitter
grammar now carries a `&'static GrammarSpec`. `codesight-extractors`
deleted its `build.rs` and its private `BUILTIN_GRAMMARS` /
`BUILTIN_QUERY_PACKS` tables; the surviving `GrammarCatalog` /
`QueryPackCatalog` types are thin adapters that delegate every lookup
through the registry / `codesight_language_support::grammars`.

**What landed:**

- `crates/codesight-language-support/Cargo.toml` picked up every
  `tree-sitter-*` grammar crate previously owned by
  `codesight-extractors`. The extractors `Cargo.toml` now only lists
  `tree-sitter = "0.26.8"` for the grammar-agnostic parser API, plus the
  new `codesight-language-support = { path = … }` dependency.
- New `codesight-language-support/build.rs` mirrors the old extractor
  build script. It reads `runtime/queries/<pack>/{tags.scm,
  injections.scm, pack.toml}` at compile time and emits
  `BUILTIN_QUERY_PACKS: &[BuiltinQueryPackRow]` into `OUT_DIR`. macOS
  SDK linking (`xcrun --show-sdk-path`) moved with it.
- New module `codesight-language-support/src/grammars.rs` owns the
  grammar side: 30 `GrammarSpec` statics (`RUST_GRAMMAR`,
  `TYPESCRIPT_GRAMMAR`, …) each pointing at a named loader fn and
  compile-time `include_str!` bound `tags.scm` / optional
  `injections.scm`. Exposes `builtin_grammar(name)`,
  `builtin_grammars()`, `builtin_query_packs()`, and the
  `builtin_extractor_version_stamp()` blake3 hash the indexer uses to
  invalidate caches when extractor behaviour drifts.
- `LanguageManifest.grammar` changed from `Option<GrammarSpec>` to
  `Option<&'static GrammarSpec>`. The `manifest!` macro in
  `languages/builtins.rs` gained a `grammar = …` arm; 28 curated
  manifests (every builtin language that has a tree-sitter grammar)
  reference the corresponding `*_GRAMMAR` static. Residual languages
  (`AL`, `BLADE`, `RAZOR`, `VERSE`, `LESS`, `SASS`, `STYL`, `ASM`, `C`,
  `VUE`, `JSX`, `MARKDOWN`, `TOML`, `TREE_SITTER_QUERY`) stay
  `grammar: None` — their extractors are hand-rolled or delegate to a
  host grammar.
- `codesight-extractors/src/extractors/engine/catalogs/grammar.rs` is
  now a thin adapter. `GrammarDefinition` is a `Copy` wrapper over
  `{ spec: &'static GrammarSpec, language_name, extensions }`, with the
  latter two reconstructed from the registry at catalog-build time.
  Private `BUILTIN_GRAMMARS` array deleted; `GrammarCatalog::builtin`
  iterates `codesight_language_support::builtin_grammars()` and joins
  each spec back to its registered `LanguageId` / extensions through
  `builtin_registry().all()`.
- `QueryPackCatalog::builtin()` and `builtin_extractor_version_stamp()`
  delegate through `codesight_language_support::builtin_query_packs()`
  / `codesight_language_support::builtin_extractor_version_stamp()`.
  The legacy `QUERY_FILES` static re-export was replaced by a
  `query_files_slice()` lazy accessor (over `&'static [(pack_id,
  tags_scm)]`) so the seven `tests/parsers/*.rs` helpers keep working
  unchanged — call sites migrated in the same patch.
- Four residual extractors that used to reach for a tree-sitter grammar
  directly — `residual/multi_dialect/scss.rs`,
  `residual/multi_dialect/luau.rs`,
  `residual/host_semantics/yaml.rs`,
  `residual/container_boundary/vue.rs` — now call
  `codesight_language_support::builtin_grammar("<name>").expect(…)`.
  No behavior change; the grammar bytes are identical.
- `codesight-extractors/build.rs` deleted.
  `codesight-extractors/src/adaptive_languages.rs` shrunk to a 10-line
  re-export over `codesight_language_support::workspace`.
- New module `codesight-language-support/src/workspace.rs` owns
  `.codesight.toml` parsing (`CodesightProjectConfig`,
  `read_project_config`, `project_extra_extensions`,
  `project_query_pack_roots`). Schema and behaviour are byte-identical
  to the old `codesight-extractors::adaptive_languages`; downstream
  helpers (`registry_builder.rs`, `query_packs.rs`) route through the
  extractor-side thin re-export.
- `LanguageRegistry::for_workspace_root` now actually applies
  `.codesight.toml` `extra_extensions`. It parses the TOML, resolves
  each `"ext" -> "language_name"` pair through the builtin registry
  (matching on `LanguageId::as_str()` or `manifest.aliases`), and calls
  `apply_extra_extensions`. Unknown language names resolve to
  `UNKNOWN_LANGUAGE` and are rejected by the existing validation —
  keeping the §9 extra-extensions validator behaviour unchanged.
- `tests/crate_graph.rs` updated: the L1 assertion for
  `codesight-extractors` now permits the new
  `codesight-language-support` dependency. Every other layering
  invariant preserved (L0.75 still allows no upward deps; L1 engine
  still banned from reaching a concrete backend).
- `registry_builder::build_registry` now parses `.codesight.toml` once
  per workspace build (down from twice) via `read_project_config`;
  cached-base short-circuit behaviour preserved.

**Parity tests (§7.2) — landed:**

- `registry::tests::phase_c_every_curated_manifest_has_grammar_and_tags`
  (§7.2.2): every manifest with a `grammar` entry has non-empty
  `tags_scm` and a `grammar_name` resolvable through
  `builtin_grammar`. Fails loudly if a grammar slips out of the
  registry.
- `registry::tests::phase_c_curated_set_matches_query_pack_directories`
  (§7.2.3): every `BUILTIN_QUERY_PACKS` pack id resolves to a
  registered `LanguageId` or a manifest alias. Guards against
  `runtime/queries/<pack>/` directories that no language owns.
- `registry::tests::phase_c_for_workspace_root_applies_extra_extensions_idempotently`
  (§7.2.7): a temp-dir `.codesight.toml` with `extra_extensions.ron =
  "rust"` round-trips through two separate `for_workspace_root` calls
  and the mapping sticks both times — no double-registration, no drift.
- `registry::tests::phase_c_workspace_config_rejects_unknown_keys`
  (§7.2.8): malformed `.codesight.toml` does not panic — unknown
  top-level sections are silently ignored, typed fields are still
  strictly validated.
- `registry::tests::phase_c_extra_extensions_rejects_malformed_entries`
  (§7.2.9): empty extension, dot-only extension, and unknown
  `LanguageId` all land in the rejected list and never leak into
  `by_ext` / `extra_ext`.
- `registry::tests::phase_c_injection_targets_resolve_to_registered_languages`
  (§7.2.10): every `(#set! injection.language "<target>")` inside
  every registered `injections.scm` resolves to a live `LanguageId`,
  an alias, or a grammar_name. Comments are ignored.

Existing integration tests (`tests/extractor_registry.rs`,
`tests/query_pack_catalog.rs`, `tests/parsers/*.rs`) kept passing
unchanged after the `GrammarDefinition` field → method migration.

**Verified:**

- `cargo check --workspace` — green.
- `cargo nextest run -p codesight-language-support --lib` — 36/36 pass
  (6 new Phase C tests).
- `cargo nextest run -p codesight-extractors --lib` — 107/107 pass.
- `cargo nextest run --test crate_graph` — pass (layering invariant
  updated: extractors now depends on core + language-support).
- `cargo nextest run --test extractor_registry --test
  query_pack_catalog` — 11/11 pass.
- `cargo nextest run --test parsers` — 335/335 pass (every SCM-driven
  parser integration test compiles extractors through the
  language-support grammar loader).
- `cargo clippy -p codesight-language-support -p codesight-extractors
  --all-targets -- -D warnings` — clean.
- `cargo nextest run -p codesight-imports -p codesight-storage -p
  codesight-analysis --lib` — 141/141 pass on storage plus
  per-crate lib suites green.

**Residual Phase C gaps (Phase D/E work):**

- `runtime/queries/<pack>/` still lives at the workspace root.
  §2.1/§2.2 explicitly targets
  `languages/<lang>/queries/{tags,injections}.scm` as the final home;
  the physical move is purely cosmetic at this point (owning crate,
  build.rs, and manifest references are already on the language
  crate) but will land alongside per-language folders in Phase D.
- Residual container-boundary / host-semantics extractors
  (`extractors/residual/**`) still live under `codesight-extractors`.
  Phase D will promote each of them into `languages/<host>/` with
  their own trait-axis implementations and delete the residual bucket
  entirely (§2.2).
- `codesight-framework/` crate is still a separate L1 crate with its
  own tree-sitter-php / tree-sitter-typescript deps. §3.6 has it
  moving under `languages/<host>/<framework>/`.
- `ExtractorRegistry::extractor_for_path` and
  `classify_special_path` still own extractor-tier routing. Phase D
  deletes them in favour of `LanguageRegistry::classify_*` +
  per-language `SymbolExtraction` trait impls.
- `StoredSymbol.language: String` / `IndexedFileUpdate.language:
  String` still carry raw strings at ingest / storage / read
  boundaries (Phase E work — the scope that needs a migration).

**Next — Phase D (heuristics + frameworks):** move `ImportResolution`
(currently scattered across `codesight-imports`), `PackageManifest`
(currently `codesight-imports::packages/*`), `LivenessPolicy`
(`ENTRYPOINT_BASENAMES`, `detect_*`, `is_test_file`, fixture detection),
`VisibilityModel` (`has_restricted_visibility`), and `ComplexityModel`
(`language_profile`, `measured_type_density`, per-language signature
heuristics) into per-language trait impls. Collapse
`codesight-framework` into `languages/<host>/<framework>/`. Delete
`ExtractorRegistry::extractor_for_path` in favour of
`LanguageRegistry::classify_path` + trait dispatch.

### Phase D.4 — framework crate collapse (landed 2026-04-20)

**Status:** complete. `codesight-framework/` is deleted. Every framework
now lives under its host language folder inside
`codesight-language-support`, and the shared trait / registry / payload
types live in `codesight-language-support::frameworks`.

**What landed:**

- New top-level module `codesight-language-support::frameworks`
  (`src/frameworks.rs` + `src/frameworks/profiles.rs`) owns the shared
  framework contract: `FrameworkProvider` trait, `FrameworkRegistry`,
  `FileContext`, `ExtraImport`, `EnrichableSymbol`, `enrich_symbol`,
  `collect_extra_imports`, `collect_metadata`, and the `walk_files` /
  `rel_path` helpers. `FrameworkProfile`, `LayerDefinition`,
  `profile_to_meta`, and `detect_framework` moved into the `profiles`
  submodule with byte-identical behavior.
- The `impl_framework_provider!` macro moved with the crate and is now
  `#[macro_export]`-published at the language-support crate root, so
  each framework keeps its original one-invocation shape.
- Framework source folders moved under the host language per §3.6:
  - `languages/typescript/nextjs/` (6 files — routing, middleware,
    provider, render, snapshot, mod)
  - `languages/vue/nuxt/` (5 files — provider, routing, render,
    snapshot, mod)
  - `languages/php/laravel/` (15 files — the full Laravel tree:
    routes, models, migrations, blade, inertia, facade, imports,
    api_client, php_structure, regex, types, provider, render,
    snapshot, mod)
  - `languages/sql/dbt/` (7 files — project, docs, schema, snapshot,
    provider, render, mod)
  New host-language `mod.rs` files at `languages/vue/mod.rs`,
  `languages/sql/mod.rs`, `languages/php/mod.rs`; `typescript.rs`
  gained `pub mod nextjs;` in place to preserve its existing file form.
  `languages/mod.rs` gained `pub mod vue;`, `pub mod sql;`,
  `pub mod php;`.
- The one upward dep `codesight-extractors::extractors::common::parse_tree`
  inlined into `frameworks::parse_tree` — 8 lines, `Option<Tree>` return
  — so the new home has no cross-layer dep. `nextjs/routing.rs`'s two
  `let Ok(tree) = parse_tree(...)` sites became `let Some(tree) = ...`
  to match the new return shape.
- `use crate::FileContext` / `crate::ExtraImport` / `crate::walk_files` /
  `crate::rel_path` / `crate::laravel::RouteInfo` imports inside the
  moved files rewritten to `crate::frameworks::*` / `super::types::*`.
  Twelve files touched; behavior unchanged.
- `codesight-language-support/Cargo.toml` picked up the `walkdir`,
  `serde_json`, and `serde_yaml` dependencies the framework code needs.
  `tree-sitter-php` and `tree-sitter-typescript` were already pulled in
  by earlier phases — no further grammar deps were added.
- `crates/codesight-framework/` removed from the workspace
  (`Cargo.toml` `[workspace.members]`), the top-level
  `[dependencies]` block, and the top-level `[dev-dependencies]` block.
- `crates/codesight-engine/Cargo.toml` dropped its
  `codesight-framework` path dep — the engine does not import the
  framework types (every consumer lived in `tests/`).
- Five integration-test files rewired: `tests/agent_v1_framework_core.rs`,
  `tests/agent_v1_framework_dbt.rs`, `tests/agent_v1_framework_laravel.rs`,
  `tests/agent_v1_framework_nextjs.rs`, `tests/agent_v1_framework_nuxt.rs`.
  Each test file updated its `use codesight_framework::...` clauses to
  the new paths
  (`codesight_language_support::frameworks::...` for shared types,
  `codesight_language_support::languages::<host>::<framework>::*` for
  per-framework APIs).
- `tests/crate_graph.rs` dropped every `codesight-framework` reference
  (workspace membership check, the framework→extractors dep assertion,
  the banned-dep list for language-support, the engine required-dep
  list). The engine required-dep list now asserts
  `codesight-language-support` in place of the deleted crate, preserving
  the §2.3 layering invariant that the engine reaches downward into
  L0.75 for its framework data.

**Parity:** full. All 193 framework + crate_graph tests pass
(`cargo nextest run --test crate_graph --test agent_v1_framework_core
--test agent_v1_framework_dbt --test agent_v1_framework_laravel --test
agent_v1_framework_nextjs --test agent_v1_framework_nuxt` — 193/193).

**Verified:**

- `cargo check --workspace` — green.
- `cargo nextest run --test crate_graph --test
  agent_v1_framework_core --test agent_v1_framework_dbt --test
  agent_v1_framework_laravel --test agent_v1_framework_nextjs --test
  agent_v1_framework_nuxt` — 193/193 pass.
- `cargo clippy -p codesight-language-support --all-targets -- -D
  warnings` — clean.

### Phase D.5 — ImportResolution + PackageManifest trait wiring (landed 2026-04-20)

**Status:** *partial*. The trait surface, the registry-backed lookup,
and the per-language adapter impls for the ten languages that actually
participate in import extraction are in place. The bulk of the
resolver/extraction code (`resolve.rs`, tree-sitter per-language
extractors, fallback extractors, manifest parsers) still lives in
`codesight-imports` for a future PR — what this phase buys is that the
*capability table* is no longer duplicated and the language-support
registry is the single source of truth for "does this language have
imports, which mode, and what is the root package?".

**What landed:**

- `ImportResolution` trait (`traits/imports.rs`) expanded:
  `supports_import_extraction` (derived from `extraction_mode`),
  `extraction_mode() -> Option<ImportExtractionMode>`, and
  `root_package_from_specifier(&str) -> String` with sensible defaults
  (none, and identity, respectively).
- `ImportExtractionMode` enum (`TreeSitterFirst` /  `FallbackOnly`)
  moved out of `codesight-imports::imports::mod` and into
  `codesight-language-support::traits::imports`, re-exported at the
  crate root so downstream callers read it from its owning crate.
- `PackageManifest` trait (`traits/packages.rs`) gained
  `root_package_from_specifier(&str) -> String` so the per-language
  package-path rules (`Rust ::`, `Python .`, scoped npm, Go vendor
  path) live on the language trait, not as a `match language { … }` in
  `codesight-imports::packages::mod`.
- Per-language `ImportResolution` + `PackageManifest` adapters added on
  the existing language structs — no new zero-size types in the
  languages that already had other axis impls:
  - `Rust` — `TreeSitterFirst`, `::` root split.
  - `Python` — `TreeSitterFirst`, `.` root split with leading-dot →
    empty-string for relative imports.
  - `TypeScript` / `Tsx` / `JavaScript` / `Jsx` — `TreeSitterFirst`,
    scoped-package (`@org/pkg` preserves both segments) + unscoped
    (`pkg/sub` → `pkg`) + relative (`.x` → empty) rules, folded into a
    shared `js_root_package` helper and wired across the four structs
    via a small local `js_family_imports!` macro.
  - `Go` — `TreeSitterFirst`, domain-rooted split
    (`github.com/org/repo/sub` → `github.com/org/repo`).
  - `Dart` — `FallbackOnly`, identity specifier; fallback-only signals
    heuristic fact quality on the fact layer.
  - `Vue` and `SqlSupport` — `FallbackOnly` via the new shared
    zero-size `FallbackOnlyImports` adapter plus a lazily-allocated
    `FALLBACK_ONLY_IMPORTS` static; `VueSupport` and `SqlSupport` got
    manual `LanguageSupport` impls (lifted out of the
    `_identity` / `_declarative` macros) so they can route `imports()`
    without needing a second `impl` block.
- New `impl_language_support_full_with_imports!` macro in
  `support_glue.rs` wires `imports()` and `packages()` as trait-object
  returns for the eight languages above (Rust, Python,
  TS/TSX/JS/JSX, Go, Dart), sharing the rest of the `_full` shape.
- Facade helpers for the three questions downstream code asks:
  `codesight_language_support::import_extraction_mode_for_language`,
  `supports_import_extraction`, and `root_package_from_specifier`.
  Each walks the registry's `by_id` table, matching on
  `LanguageId::as_str()` or `manifest.aliases`, and dispatches via the
  relevant trait. Re-exported at the crate root alongside the existing
  path-indexed helpers.
- `codesight-imports::IMPORT_EXTRACTION_CAPABILITIES` — the 10-entry
  `canonical_language` + `aliases` + `mode` parallel table — deleted.
  `import_capability_for_language` and
  `import_extraction_capabilities` deleted. `import_fact_quality_for`
  rewritten to route through the facade (`import_extraction_mode`).
  `supports_import_extraction` is now a thin wrapper that forwards to
  the language-support crate so both crates answer the same question
  the same way.
- `codesight-imports::extract_root_package_from_specifier` — the
  `match language.to_ascii_lowercase()` dispatcher over per-ecosystem
  helpers — replaced with a one-line call to
  `codesight_language_support::root_package_from_specifier`. The
  per-ecosystem `root_package_from_specifier` functions in
  `codesight-imports/src/packages/{rust,python,javascript,go}.rs`
  deleted (no callers outside the dispatcher). The surviving modules
  keep their `read_package_names` manifest parsers and
  `workspace_*_from_root` walkers, because those are the bulk of the
  package-discovery surface and belong in a later D.5 follow-up when
  `PackageManifest::package_name_for_dir` / `workspace_members` get
  non-default impls per language.

**Parity:** full for the capability surface. `import_extraction_mode`
agrees with the old table on every entry: `python`, `py`, `javascript`,
`js`, `typescript`, `ts`, `tsx`, `jsx`, `vue`, `sql`, `dart`, `go`,
`golang`, `rust`, `rs` — all round-trip to the expected mode through the
language-support registry.

**Verified:**

- `cargo check --workspace` — green.
- `cargo nextest run -p codesight-imports -p codesight-language-support
  -p codesight-analysis --lib` — 162/162 pass (65 imports + 108
  language-support + analysis).
- `cargo nextest run --test crate_graph --test agent_v1_framework_*`
  — 193/193 pass.
- `cargo nextest run -p codesight-language-support --test
  phase_d_parity` — 26/26 pass.
- `cargo clippy -p codesight-language-support -p codesight-imports
  --all-targets -- -D warnings` — clean.

**Not yet landed (future Phase D.5 / E work):**

- The actual import extraction machinery — `codesight-imports::imports`
  (resolve.rs ~27 KB; `tree_sitter/{rust,js_ts,go,python}.rs`;
  `fallback/{rust,js_ts,python,go,vue,sql,dart}.rs`;
  `cross_repo.rs`; `bundle.rs`; `python_stems.rs`) — still lives in
  `codesight-imports`. The trait hooks are in place so the per-language
  bodies can be moved incrementally without changing call sites.
- Package manifest parsers — `codesight-imports::packages::{rust,
  python, javascript, go, dotnet}::read_package_names` and
  `workspace_crates_from_root` / `workspace_bin_targets_from_root` —
  still in `codesight-imports`. `PackageManifest::package_name_for_dir`
  and `workspace_members` stay at their default `None` / empty today;
  wiring the Rust/Python/JS/Go/.NET impls moves in the next round.
- **Phase E — typed storage boundary.** `StoredSymbol.language: String`
  and `IndexedFileUpdate.language: String` still carry raw strings.
  Flipping every ingest / storage / analysis boundary to
  `ResolvedLanguage { host, profile }` is still the final fail-safe
  phase in §8.E.
- `ExtractorRegistry::extractor_for_path` and `classify_special_path`
  still own extractor-tier routing. They'll be collapsed alongside the
  remaining D.5 / Phase E work.

### Phase D.1-D.3 — liveness, visibility, complexity (landed 2026-04-20)

**Status:** complete for the three analysis-side axes
(`LivenessPolicy`, `VisibilityModel`, `ComplexityModel`). The Rust,
Python, TypeScript/TSX/JS/JSX, Go, Java, C#, C++, C, Dart, Kotlin, Swift,
Ruby, Perl, Lua, PHP, Elixir, Erlang, and every declarative-markup
language now own their per-language heuristics in
`crates/codesight-language-support/src/languages/<family>.rs`.
`codesight-analysis` is a pure scoring layer on top of the façade.
Phase D.4 (framework crate collapse) and D.5 (imports + packages) are
tracked as follow-ups; neither blocks the three axes above.

**What landed:**

- Expanded three axis traits with real surfaces:
  - `LivenessPolicy` (§4.4): `entrypoint_basenames`, `is_test_file`,
    `is_test_fixture`, `is_conventional_path`,
    `additional_live_root_files(root, all_files) -> Vec<String>`.
  - `VisibilityModel` (§4.5): `is_restricted(source_text, name) -> bool`.
  - `ComplexityModel` (§4.6): `family() -> LanguageFamily`,
    `nesting_mode() -> NestingMode`, `function_length_warn`,
    `fallback_type_baseline`, `supports_measured_types`,
    `measured_type_density(source) -> Option<f64>`.
- New `LanguageFamily` enum (`BracedStatic`, `ScriptedDynamic`,
  `FunctionalExpression`, `DeclarativeMarkup`, `Unknown`) and
  `NestingMode` (`Brace`, `Indent`) are L0.75 public types. Downstream
  crates type-check against these instead of string-keyed tables.
- Per-language folders now carry behavior:
  - `languages/rust.rs` — owns `FOREIGN_TRAIT_BASES`,
    `detect_cargo_target_files`, `detect_foreign_trait_impl_files`,
    Rust-specific entrypoints (`main.rs`, `lib.rs`, `build.rs`),
    `pub`/`pub(crate)` visibility, and the Rust `fn name(p: Ty) -> Ret`
    type-density measurement.
  - `languages/python.rs` — owns `__name__ == '__main__'` probing,
    `main.py`/`__init__.py`/`conftest.py` basenames, underscore-prefix
    visibility, 0.4 fallback baseline.
  - `languages/typescript.rs` — TS/TSX/JS/JSX with `private`/`export`
    visibility, `.test.ts`/`.spec.js`/`__tests__` test detection.
  - `languages/go.rs` — uppercase-first visibility, `_test.go`,
    `measure_go_signature` with grouped parameters.
  - `languages/braced_static.rs` — Java, C#, C++, Dart, Kotlin, Swift
    with `public`/`private`/`protected`/`internal` visibility.
  - `languages/scripted_dynamic.rs` — Ruby, Perl, Lua, PHP.
  - `languages/functional.rs` — Elixir, Erlang.
  - `languages/declarative.rs` — shared declarative-markup shell.
- New `crate::type_density` module extracts the four signature parsers
  (`measure_colon_after_name`, `measure_php_signature`,
  `measure_type_before_name`, `measure_go_signature`) and the
  `TopLevelScanner` / `split_top_level` / `type_token_score` helpers
  they share. Every per-language `measured_type_density` is one call.
- New `crate::nesting` module: `max_nesting_depth_for_path` +
  `max_nesting_depth_with_mode` + the `brace_nesting_depth` /
  `indent_nesting_depth` scanners. Callers pass a path; the registry
  picks the right scanner via `NestingMode`.
- New `crate::facade` public API — 14 path-indexed helpers that
  downstream crates call without holding a `LanguageRegistry`:
  `language_id_for_path`, `language_family_for_path`,
  `nesting_mode_for_path`, `measured_type_density_for_path`,
  `fallback_type_baseline_for_path`, `function_length_warn_for_path`,
  `has_restricted_visibility`, `is_test_file`, `is_test_fixture`,
  `is_entrypoint_basename`, `all_entrypoint_basenames`,
  `is_conventional_path`, `test_weight_multiplier`,
  `additional_live_root_files`.
- `codesight-analysis` picks up `codesight-language-support` as a
  dependency (new L1 → L0.75 edge in `tests/crate_graph.rs`).
- `codesight-analysis/src/agent_readiness.rs` shrank from 1660 lines to
  663. Every language-specific block went: `LanguageProfile`,
  `language_profile`, `measured_type_density`, the four measurement
  parsers, the four `extract_*` helpers, `TopLevelScanner`,
  `WEAK_TYPE_EXACT`/`WEAK_TYPE_CONTAINS`, `has_restricted_visibility`,
  `is_test_file`, `brace_nesting_depth`, `indent_nesting_depth`,
  `max_nesting_depth`, `is_python_receiver`, `is_rust_receiver`,
  `normalize_type_token`, `type_token_score`. 997 LoC deleted.
- `codesight-analysis/src/dead_code/entrypoints.rs` shrank from 335
  lines to 214: the 25-entry `FOREIGN_TRAIT_BASES`, the 24-entry
  `ENTRYPOINT_BASENAMES`, `detect_main_guard_files`,
  `detect_cargo_target_files`, `detect_foreign_trait_impl_files`, and
  the local `is_test_file` / `is_test_fixture` all now re-export or
  delegate through the registry. `detect_cargo_target_files` and
  `detect_foreign_trait_impl_files` are thin filters over Rust's
  `additional_live_root_files`.
- `codesight-analysis/src/quality.rs`: `CONVENTIONAL_DIRS`,
  `CONVENTIONAL_BASENAMES`, `is_conventional_path`,
  `test_weight_multiplier` deleted and re-imported from the façade;
  `max_nesting_depth_for_file` replaced with
  `max_nesting_depth_for_path`.
- `codesight-engine/src/service/agent_v1/quality_more.rs` routed
  through `codesight_language_support::is_conventional_path` (engine
  picks up the new dep in `Cargo.toml`).
- Two bonus clean-ups in the same patch:
  - `crates/codesight-analysis/src/graph/psr4.rs` converted a
    `sort_by(|a, b| b.0.len().cmp(&a.0.len()))` closure to the
    idiomatic `sort_by_key(|e| Reverse(e.0.len()))` clippy prefers.

**Parity tests (§7.2) — landed:**

New integration suite `crates/codesight-language-support/tests/phase_d_parity.rs`
(§7.2.11 framework parity prep + behavioral regression cover):

- Entrypoint basename discovery (12 curated names, plus negative
  assertions).
- Test-file detection across Rust, Python, JS/TS, Ruby, plus production
  code negative cases.
- Conventional-path detection matching the old `CONVENTIONAL_DIRS`
  table + production negative cases.
- Rust `pub`/`pub(crate)` visibility, Go uppercase-first, Python
  underscore, TypeScript `private`/`export`, Java `public`/`private`.
- Rust → `BracedStatic`, Python → `ScriptedDynamic`, Elixir →
  `FunctionalExpression`, YAML → `DeclarativeMarkup`, unknown →
  `Unknown`.
- Measured type density for Rust signatures; Python/TS/JS fallback
  baselines at `0.4`/`0.7`/`0.1`.
- Brace and indent nesting scanner parity against the old tests in
  `agent_readiness.rs`.

Existing `codesight-analysis` tests (54) and every integration test
that exercises these paths (40 dead-code / quality-signal tests, 8
parse-warning tests, 69 broader integration tests) keep passing
unchanged.

**Verified:**

- `cargo check --workspace` — green.
- `cargo nextest run -p codesight-language-support --lib` — 41/41 pass
  (3 new Phase D facade tests plus 38 Phase A-C).
- `cargo nextest run -p codesight-language-support --test phase_d_parity`
  — 26/26 pass.
- `cargo nextest run -p codesight-analysis --lib` — 54/54 pass.
- `cargo nextest run --test crate_graph` — pass (L1 analysis ↔ L0.75
  invariant updated).
- `cargo nextest run --test find_dead_code --test dead_code_v2
  --test hotspots --test quality_signal` — 40/40 pass.
- `cargo nextest run --test parse_warnings` — 8/8 pass.
- `cargo nextest run --test blast_radius --test complexity
  --test dead_code_v2 --test incremental --test extraction_tiers
  --test refactor_targets --test quality_signal --test repo_health
  --test find_dead_code --test hotspots` — 69/69 pass.
- `cargo clippy -p codesight-language-support -p codesight-analysis
  --all-targets -- -D warnings` — clean.

### Phase D.5 (packages) + classifier collapse (landed 2026-04-20)

**Status:** complete for package-manifest parsing and for the
`classify_special_path` duplicate. `codesight-imports::packages::{rust,
python, javascript, go, dotnet}` are deleted; every per-ecosystem
`read_package_names` is now a `PackageManifest::package_name_for_dir`
impl on the owning language. `codesight-extractors::extractors::
classifier` is deleted; host-profile routing lives in a single place —
`LanguageRegistry::classify_path` — exactly where §3.3 puts it.

**What landed:**

- `PackageManifest::package_name_for_dir` now has real impls on
  `Rust` (Cargo.toml), `Python` (pyproject.toml + setup.cfg with
  PEP 503 underscore→dash normalization), `TypeScript` / `Tsx` /
  `JavaScript` / `Jsx` (package.json via `serde_json`), `Go`
  (go.mod `module` line), and `CSharp` (`*.csproj` `<PackageName>` /
  `<AssemblyName>` fallback). Each impl sits next to the
  `root_package_from_specifier` Phase D.5-partial already wired.
- Three manifest-parsing modules collapsed into their owning language
  files: `codesight-imports/src/packages/{rust,python,javascript,go,
  dotnet}.rs` are deleted in this patch. Their tests moved alongside
  the `package_name_for_dir` impls in
  `codesight-language-support/src/languages/{rust,python}.rs` so
  coverage doesn't regress.
- `codesight-imports::packages::walker::discover_package_names`
  rewritten to iterate every registered language via
  `codesight_language_support::builtin_registry().all()` and call
  `.packages().package_name_for_dir(dir)` on each. The five-way
  `[python, javascript, go, rust, dotnet]` call chain collapses to a
  single registry loop, and new languages automatically participate the
  moment they implement the trait.
- `workspace_crates_from_root` / `workspace_bin_targets_from_root`
  relocated into `codesight-language-support::languages::rust` so Rust
  owns every Cargo-workspace heuristic. `codesight-imports::packages`
  keeps the re-export for call-site stability (the engine's indexing
  layer hits them at `code_index.rs:232`, `:378`).
- New `impl_language_support_full_with_packages!` macro in
  `support_glue.rs` wires `packages()` without `imports()`, used by
  `CSharp` which participates in `package_name_for_dir` but not import
  extraction.
- `codesight-extractors::extractors::classifier` deleted
  (45 SLOC + 34 SLOC of tests). The OpenAPI basename + Ansible
  conventional-directory logic already moved into
  `codesight-language-support::languages::yaml` under
  `matches_openapi_path` / `matches_ansible_path` in Phase B; deleting
  the duplicate closes the drift window.
- `profiles::LanguageProfileRegistry::resolve_for_path` rewritten as a
  route-through over `builtin_registry().classify_path(path)`. It reads
  `ResolvedLanguage.profile` and `ResolvedLanguage.host.as_str()` to
  decide between `(openapi, yaml)` / `(openapi, json)` / `(ansible,
  yaml)`. Same external behavior, one source of truth.
- `residual/host_semantics/yaml.rs` — the Ansible extractor — now
  decides "is this an Ansible layout?" by calling
  `builtin_registry().classify_path(...).profile ==
  Some(ANSIBLE)` instead of importing the deleted
  `path_suggests_ansible_layout`.

**Parity:** full.

- `extract_package_names` still discovers pyproject, setup.cfg,
  package.json, go.mod, Cargo.toml, and `*.csproj` names across every
  test in `tests/cross_repo.rs`.
- OpenAPI/Ansible tests in `tests/parsers/yaml_language` keep passing
  byte-for-byte against the registry-routed resolver.
- The "package_name_for_dir_reads_cargo_name",
  "pyproject_name_wins_over_setup_cfg", and "setup_cfg_fallback"
  tests added in this patch guard the per-language parser implementations
  directly.

**Verified:**

- `cargo check --workspace` — green.
- `cargo nextest run --workspace --lib` — 387/387 pass.
- `cargo nextest run` — **1413/1413 pass** (1 skipped, 1 leaky — both
  pre-existing). This is the full integration suite: framework
  tests (193), parsers (335), query-pack catalog (6), extractor
  registry (4), crate_graph, repo_health, hotspots, dead_code,
  quality_signal, and every agent-v1 MCP tool. Runtime: 184 s on
  local SSD, one cold pass.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.

**Not yet landed (Phase E work, deferred):**

- **Phase E — typed storage boundary.** `StoredSymbol.language:
  String` and `IndexedFileUpdate.language: String` still carry raw
  strings at the ingest / storage / read boundary. Flipping to
  `ResolvedLanguage { host, profile }` touches every extractor emit
  site (~40 files), the SQLite schema, and every analysis consumer
  that does `s.language == "rust"` — ~100+ call sites across
  `codesight-analysis`, `codesight-engine`, `codesight-retrieval`.
  The refactor is well-specified but large enough to need its own
  session to land cleanly without a partial drop.
- **Move `runtime/queries/<pack>/` → `languages/<lang>/queries/`.**
  Physical relocation landed in 2026-04-20 follow-up (see next
  section); the per-language subfolder split
  (`languages/<lang>/queries/`) remains future work since several
  packs share consolidated modules like `braced_static.rs`.
- **`codesight-framework` crate collapse.** Already landed in Phase
  D.4 (see above section).
- **Full `ImportResolution` machinery move** — `resolve.rs` (27 KB),
  `tree_sitter/{rust,js_ts,go,python}.rs`, `fallback/*.rs` — still
  lives in `codesight-imports`. The capability surface is routed
  through the façade (Phase D.5 partial), but the extraction bodies
  themselves still need to move into per-language folders to complete
  §4.2's "every language owns its import extraction" rule.

### Phase D.6 — query-pack ownership relocation (landed 2026-04-20)

**Status:** complete. `runtime/queries/` no longer sits at the
workspace root — its 30 builtin packs (bash, cpp, csharp, css, dart,
ejs, elixir, erlang, fortran, gleam, go, java, javascript, json,
kotlin, lua, luau, nix, perl, php, python, ruby, rust, scss, sql,
swift, tsx, typescript, xml, yaml) now live inside the owning crate
at `crates/codesight-language-support/runtime/queries/<pack>/`.
Workspace-root `runtime/queries/` is gone; its old parent directory
was removed (`rmdir runtime/`) so no stale location lingers.

**What landed:**

- `git mv runtime/queries
  crates/codesight-language-support/runtime/queries` — 30 pack
  directories (tags.scm + optional injections.scm + optional
  pack.toml + README.md) relocated under the language-support crate.
- `crates/codesight-language-support/build.rs` simplified: the
  `workspace_root` helper deleted (no longer needed), `queries_root`
  is now `manifest_dir.join("runtime/queries")`, and the emitted
  `include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/...")` paths
  shrank from `/../../runtime/queries/...` to `/runtime/queries/...`.
- `crates/codesight-language-support/src/grammars.rs` — the 34
  `include_str!("../../../runtime/queries/<pack>/tags.scm")` literals
  that back each `*_GRAMMAR` static rewritten to
  `include_str!("../runtime/queries/<pack>/tags.scm")`. Mechanical
  `sed` over the file; behavior identical.
- `tests/query_pack_catalog.rs` — both `query_dirs_on_disk` and
  `builtin_query_packs_use_canonical_tags_layout` helpers now resolve
  the pack root via
  `Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/codesight-language-support/runtime/queries")`
  instead of the workspace-root path.
- Doc comments in
  `crates/codesight-extractors/src/extractors/engine/mod.rs` and
  `engine/scm/mod.rs` updated to reflect the new pack location.
  Residual references to `runtime/queries/<pack>/` inside
  `codesight-extractors` stay relative to the owning crate and read
  correctly from that frame.

**Parity:** full. No behavior change. The pack contents are
byte-identical; only the include_str path prefix changed.

**Verified:**

- `cargo check --workspace` — green.
- `cargo nextest run -p codesight-language-support --lib` —
  49/49 pass.
- `cargo nextest run -p codesight-extractors --lib` — 102/102 pass.
- `cargo nextest run --test query_pack_catalog --test crate_graph
  --test extractor_registry` — 12/12 pass (every query-pack / grammar
  / layering invariant test green under the new path).
- `cargo nextest run --test parsers` — 335/335 pass. Every SCM-backed
  parser integration test compiles grammars and query packs through
  the relocated tree-sitter bindings.
- `cargo clippy -p codesight-language-support -p codesight-extractors
  --all-targets -- -D warnings` — clean.

**Phase E — typed pipeline language (landed 2026-04-20):**

- **`LanguageId`** moved to **`codesight-core`** (L0): a `Copy` newtype
  over `&'static str` with `intern` / `canonical_or_unknown` for SQLite
  and runtime strings, plus canonical constants (`RUST`, `PYTHON`, …).
- **Pipeline types** (`StoredSymbol`, `IndexedFileUpdate`,
  **`FileFingerprint`**) now use **`language: LanguageId`** end-to-end.
- **Extractors** emit `LanguageId` (constants or `LanguageId::intern`).
- **Storage** persists the string via `language.as_str()` and reads back
  with `LanguageId::intern`.
- **Tests / assertions** that compared to string literals now use
  **`language.as_str()`** or **`LanguageId`** equality.

**Not yet landed (follow-ups):**
- **Per-language `languages/<lang>/queries/` subfolder split.** The
  relocated packs still share the flat layout (`runtime/queries/<pack>`
  under the crate). Splitting them into
  `src/languages/<lang>/queries/{tags,injections}.scm` is §2.1's
  aspirational target and requires promoting each shared language
  file (`braced_static.rs`, `scripted_dynamic.rs`,
  `declarative.rs`, `functional.rs`) into per-language folders.
- **Full `ImportResolution` machinery move** — unchanged from D.5
  residuals.
- **`ExtractorRegistry::extractor_for_path`** — still owns
  extractor-tier routing.
- **`LspBackend::language_for`** — still scans its own config table.

