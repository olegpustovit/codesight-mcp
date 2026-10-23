//! The consolidated `Language` trait — one trait per language, every method
//! default-bodied so overrides are opt-in.
//!
//! This is the single home every crate above L0.75 consumes: downstream
//! callers take a `&dyn Language` from [`crate::LanguageRegistry`] and call
//! methods directly. Legacy axis traits (`SymbolExtraction`, `ImportResolution`,
//! `LivenessPolicy`, `PackageManifest`, `VisibilityModel`, `ComplexityModel`)
//! are gone — their methods fold in here.

use rustc_hash::{FxHashMap, FxHashSet};
use std::path::Path;

use path_clean::PathClean;

use dowser_core::StoredSymbol;
use dowser_core::reachability::StructuralEdge;
use dowser_core::symbols::{ExtractionTier, FactQuality};

use crate::family::{LanguageFamily, NestingMode};
use crate::frameworks::FrameworkProvider;
use crate::imports_types::ImportEdge;
use crate::manifest::{GrammarSpec, LanguageId, LanguageManifest};

/// Extraction strategy for a language's imports.
///
/// `TreeSitterFirst` — prefer the structural tree-sitter path; fall back to
/// line scans only when the grammar fails to parse or yields an empty
/// result. Emits `FactQuality::BaselineStructural` when the structural path
/// succeeds.
///
/// `FallbackOnly` — no curated tree-sitter query is available; extraction
/// is a line scanner only. Always emits `FactQuality::Heuristic`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportExtractionMode {
    TreeSitterFirst,
    FallbackOnly,
}

/// Confidence tier for a language-declared live root. Mirrors the
/// `EvidenceNote` structural/heuristic split without pulling analysis into
/// this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveRootConfidence {
    /// Fact: a build manifest or parser confirms this file is a runtime
    /// entry (Cargo `[[bin]]`, Python `__main__.py` entrypoint, etc.).
    Structural,
    /// Heuristic: pattern match — e.g. `if __name__ == '__main__'` guard,
    /// foreign-trait impl detection.
    Heuristic,
}

/// A file-level live root surfaced by a language. Analysis unions these
/// across every registered language; it never branches on the language id.
#[derive(Debug, Clone)]
pub struct LiveRoot {
    /// Repo-relative path of the live root.
    pub path: String,
    /// Stable reason key (`"main_guard"`, `"cargo_target"`,
    /// `"foreign_trait_impl"`, `"nextjs_page"`, …). Surfaces in the
    /// evidence note's `code` field.
    pub reason: &'static str,
    /// Human-readable message describing why the file is live.
    pub message: String,
    pub confidence: LiveRootConfidence,
    /// When true, the presence of this root counts as an explicit
    /// entry for the "no-entry-points" detector — analysis will not
    /// emit a framework-warning when at least one explicit root is
    /// found.
    pub explicit: bool,
}

/// Shape of a language's primary type-annotation convention. Drives the
/// default [`Language::measured_type_density`] — languages only declare
/// their style and inherit the walker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeAnnotationStyle {
    /// `name: Type` (TypeScript, Kotlin, Swift, JavaScript w/ JSDoc).
    ColonAfterName,
    /// `Type name` (C, C++, C#, Java, Dart).
    TypeBeforeName,
    /// Language computes its own density (Go grouped params, PHP unions,
    /// Python receivers, Rust receivers).
    Custom,
}

/// Everything a language owns. New-language onboarding is a single `impl
/// Language for MyLang { fn id() = …; fn manifest() = …; /* overrides */ }`
/// — any unoverridden method inherits [`crate::FallbackLanguage`]'s default.
pub trait Language: Send + Sync {
    // ── Identity ──────────────────────────────────────────────────────────
    fn id(&self) -> LanguageId;
    fn manifest(&self) -> &'static LanguageManifest;

    // ── Grammar + SCM queries (all derived from `grammar_spec()` by default) ──
    /// Tree-sitter grammar + SCM bundle. Languages with a grammar override
    /// this to return `Some(&MY_GRAMMAR)`; query-method defaults read from
    /// the returned spec.
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        None
    }

    /// Tree-sitter grammar handle. Default derives from `grammar_spec()`.
    fn grammar(&self) -> Option<tree_sitter::Language> {
        self.grammar_spec().map(|spec| spec.language())
    }

    /// Grammar node classes that carry identifier syntax, without binding resolution.
    fn is_identifier_node(&self, kind: &str) -> bool {
        kind.ends_with("identifier")
    }

    /// SCM tags query for this language. Default reads from `grammar_spec()`.
    fn tags_query(&self) -> Option<&'static str> {
        self.grammar_spec().map(|spec| spec.tags_scm)
    }

    /// SCM injections query. Default reads from `grammar_spec()`.
    fn injections_query(&self) -> Option<&'static str> {
        self.grammar_spec().and_then(|spec| spec.injections_scm)
    }

    // ── Symbol extraction ───────────────────────────────────────────────
    /// Whether this language produces extracted symbols. Languages without a
    /// grammar or a manual extractor return `false`.
    fn supports_extraction(&self) -> bool {
        false
    }

    /// Extraction tier this language emits.
    fn extraction_tier(&self) -> ExtractionTier {
        if self.grammar_spec().is_some() {
            ExtractionTier::CuratedQueryPack
        } else {
            ExtractionTier::ManualOverride
        }
    }

    /// Primary extraction entry point. Default runs the curated SCM pipeline
    /// when `grammar_spec()` is available. Languages with bespoke extraction
    /// (AL, Blade, Vue, Razor, EJS, YAML/Ansible, SQL/dbt, ASM dialects,
    /// SCSS dialects, Luau dialects, Verse) override.
    fn extract_symbols(&self, file_path: &str, source: &str) -> anyhow::Result<Vec<StoredSymbol>> {
        let Some(spec) = self.grammar_spec() else {
            return Ok(Vec::new());
        };
        if spec.tags_scm.is_empty() {
            return Ok(Vec::new());
        }
        crate::scm_engine::extract_with_spec(self.id().as_str(), spec, file_path, source)
    }

    /// Post-extraction symbol fixup — runs after `extract_symbols` / the SCM
    /// engine produces the raw symbol list. C++ uses it to downgrade plain-C
    /// `.h` headers to `C`; other languages can hook content-aware rewrites
    /// (TSX/JSX dual-identity, Razor host language, …) here without adding
    /// C++-specific branches to generic code. Default: no-op.
    fn postprocess_symbols(&self, _symbols: &mut [StoredSymbol], _file_path: &str, _source: &str) {}

    // ── Imports ──────────────────────────────────────────────────────────
    /// Extraction strategy for this language, or `None` when extraction is
    /// not implemented (the default for every language that doesn't
    /// explicitly opt in).
    fn extraction_mode(&self) -> Option<ImportExtractionMode> {
        None
    }

    /// Convenience: whether imports are extractable at all.
    fn supports_import_extraction(&self) -> bool {
        self.extraction_mode().is_some()
    }

    /// Root package name for an import specifier (e.g. `"serde::de::T"` →
    /// `"serde"`, `"flask.blueprints"` → `"flask"`, `".utils"` → `""`).
    /// Default returns the specifier unchanged — languages override when
    /// they have package-path semantics.
    fn root_package_from_specifier(&self, specifier: &str) -> String {
        specifier.to_string()
    }

    /// SCM query that captures import statements. Conventional captures:
    /// `@specifier` on the import path string/node; `@name` on each imported
    /// identifier. Default `None`; languages with a grammar that ship an
    /// `imports.scm` override.
    fn imports_query(&self) -> Option<&'static str> {
        None
    }

    /// Extract structured import edges from `source`. Default dispatches
    /// through `grammar()` + `imports_query()` via [`crate::scm_imports`];
    /// languages whose imports don't map cleanly to SCM (Rust's nested
    /// `use`) override with a hand-rolled walker.
    fn extract_imports(&self, source: &str) -> Vec<ImportEdge> {
        if let (Some(grammar), Some(query)) = (self.grammar(), self.imports_query())
            && let Some(edges) = crate::scm_imports::extract(source, grammar, query)
        {
            return edges;
        }
        Vec::new()
    }

    /// Language-specific display format for import edges (shown in bundle /
    /// context output). Default emits ES-module-style `import {...} from '…'`
    /// lines. Go and Python override.
    fn display_import_lines(&self, edges: &[ImportEdge]) -> Vec<String> {
        crate::imports_types::format_esm_import_lines(edges)
    }

    /// Verbatim source lines for display when a language's import block has
    /// meaningful shape (Go `import (…)`, Python `import X` / `from X`).
    /// Default: ignore source, use `display_import_lines` on extracted edges.
    fn display_import_lines_from_source(&self, source: &str) -> Vec<String> {
        self.display_import_lines(&self.extract_imports(source))
    }

    /// Top-level module/package stems referenced from `source` (Python's
    /// specific case today). Default empty; Python overrides.
    fn top_level_import_stems(&self, _source: &str, _path: &str) -> Vec<String> {
        Vec::new()
    }

    // ── Structural reachability (ingest-time) ────────────────────────────
    /// Extract structural reachability edges from this file. Called once
    /// per file during ingest; the returned edges are persisted in
    /// `reachability_edges` and consumed by `dead_code` analysis.
    ///
    /// Edges describe re-exports, path references, match-arm dispatch,
    /// `mod`-style declarations, import statements, and type-only refs.
    /// Each edge points at either a target file (when the language can
    /// resolve it cheaply during extraction) or a bare symbol name (when
    /// the resolution would require a workspace-wide symbol index — the
    /// dead-code reader does the symbol→file join via the persisted
    /// symbol table).
    ///
    /// Default impl: translate every `ImportEdge` returned by
    /// [`Self::extract_imports`] into one symbol edge per imported name
    /// (plus the root package). Languages with no import extraction fall
    /// back to [`Self::regex_reachability_patterns`]. Languages with
    /// rich shapes beyond imports (Rust's `pub use`, `mod`, match-arm
    /// dispatch) override this method.
    fn reachability_edges(&self, content: &str, source_file: &str) -> Vec<StructuralEdge> {
        let imports = self.extract_imports(content);
        if !imports.is_empty() {
            return crate::reachability_default::edges_from_imports(self, source_file, &imports);
        }
        let patterns = self.regex_reachability_patterns();
        if patterns.is_empty() {
            return Vec::new();
        }
        crate::reachability_default::extract_with_patterns(source_file, content, patterns)
    }

    /// Per-edge-kind regex patterns used by the default implementation of
    /// [`Self::reachability_edges`]. Each entry is
    /// `(EdgeKind, capturing_regex)` where the regex's first capture group
    /// names a *symbol* (bare identifier). Default empty — languages with
    /// a tree-sitter grammar override `reachability_edges` directly.
    fn regex_reachability_patterns(&self) -> &'static [(dowser_core::EdgeKind, &'static str)] {
        &[]
    }

    /// Path aliases for this language discovered in `root` (e.g. tsconfig
    /// `paths`, pyproject `[tool.…]`). Each alias maps to an ordered list of
    /// replacement prefixes (tsconfig's `paths` allows multiple fallbacks).
    /// Default empty.
    fn path_aliases(&self, _root: &Path) -> FxHashMap<String, Vec<String>> {
        FxHashMap::default()
    }

    /// Resolve an import edge to a project-relative file path. Default
    /// dispatches to the generic relative / alias / basename resolver in
    /// [`crate::imports_orchestration::resolve_specifier`]. Rust overrides
    /// with its anchor-aware `crate::` / `self::` / `super::` walker;
    /// Python overrides to add a dotted-module stem fallback.
    #[allow(clippy::too_many_arguments)]
    fn resolve_import_edge(
        &self,
        edge: &ImportEdge,
        from_file: &str,
        source_files: &FxHashSet<String>,
        _known_files_vec: &[String],
        alias_map: Option<&FxHashMap<String, Vec<String>>>,
        _workspace_crates: Option<&FxHashMap<String, String>>,
    ) -> Option<(String, FactQuality)> {
        crate::imports_orchestration::resolve_specifier(
            self,
            &edge.specifier,
            from_file,
            source_files,
            alias_map,
        )
    }

    /// Resolve a `./` or `../` relative import specifier for this language.
    /// Default: normalize against `from_file` and try the exact path only.
    /// JS-family languages (JavaScript, TypeScript, JSX, TSX, Vue) override
    /// with extension and index-file probing.
    fn resolve_relative_specifier(
        &self,
        specifier: &str,
        from_file: &str,
        source_files: &FxHashSet<String>,
    ) -> Option<String> {
        let dir = match from_file.rfind('/') {
            Some(idx) => &from_file[..idx],
            None => ".",
        };
        let normalized = Path::new(&format!("{dir}/{specifier}"))
            .clean()
            .to_string_lossy()
            .replace('\\', "/");
        source_files.contains(&normalized).then_some(normalized)
    }

    // ── Packages / workspaces ────────────────────────────────────────────
    fn package_name_for_dir(&self, _dir: &Path) -> Option<String> {
        None
    }

    fn workspace_members(&self, _root: &Path) -> Vec<String> {
        Vec::new()
    }

    /// Namespace-to-directory autoload map (PHP PSR-4, etc.). Default empty.
    fn namespace_map(&self, _root: &Path) -> FxHashMap<String, String> {
        FxHashMap::default()
    }

    /// Resolve a language-native fully-qualified name to a `symbol_id`
    /// (e.g. `"App\\Models\\User"` → `"app/Models/User.php::User#class"`).
    /// Default `None`; languages with a namespace convention override.
    fn resolve_fqn(
        &self,
        _fqn: &str,
        _root: &Path,
        _source_files: &FxHashSet<String>,
    ) -> Option<String> {
        None
    }

    // ── Liveness / entrypoints / tests ───────────────────────────────────
    /// Basenames always treated as entry-point roots for this language
    /// (`main.rs`, `lib.rs`, `build.rs`, `main.py`, `__main__.py`, etc.).
    fn entrypoint_basenames(&self) -> &'static [&'static str] {
        &[]
    }

    /// Basename prefixes (case-insensitive) that mark a test file (Python's
    /// `test_`, `tests`). Default empty.
    fn test_file_basename_prefixes(&self) -> &'static [&'static str] {
        &[]
    }

    /// Lowercased path suffixes that mark a test file (`.test.ts`,
    /// `_test.go`, `_spec.rb`). Default empty.
    fn test_file_suffixes(&self) -> &'static [&'static str] {
        &[]
    }

    /// Lowercased path substrings (directory markers) that mark a test file
    /// (`/tests/`, `/__tests__/`, `/spec/`). Default empty.
    fn test_file_dirs(&self) -> &'static [&'static str] {
        &[]
    }

    /// Whether this language considers `path` a test file. Default scans
    /// [`Self::test_file_basename_prefixes`], [`Self::test_file_suffixes`],
    /// and [`Self::test_file_dirs`] (matched with a leading `/` anchor so
    /// `"/tests/"` catches both `tests/foo.rs` and `src/tests/bar.rs`).
    /// Languages with exotic rules override.
    fn is_test_file(&self, path: &str) -> bool {
        let lower = path.to_ascii_lowercase();
        let basename = lower.rsplit('/').next().unwrap_or(&lower);
        if self
            .test_file_basename_prefixes()
            .iter()
            .any(|p| basename.starts_with(p))
        {
            return true;
        }
        if self.test_file_suffixes().iter().any(|s| lower.ends_with(s)) {
            return true;
        }
        let anchored = format!("/{lower}");
        if self.test_file_dirs().iter().any(|d| anchored.contains(d)) {
            return true;
        }
        false
    }

    /// Lowercased directory markers for fixture / snapshot trees
    /// (`/__snapshots__/`, `/fixtures/`). Default empty.
    fn test_fixture_dirs(&self) -> &'static [&'static str] {
        &[]
    }

    /// Lowercased path suffixes for fixture artifacts (`.snap`,
    /// `.snap.new`, `.pending.snap`). Default empty.
    fn test_fixture_suffixes(&self) -> &'static [&'static str] {
        &[]
    }

    /// Whether `path` is a fixture / golden / snapshot file this language's
    /// reachability analysis should skip entirely. Default scans
    /// [`Self::test_fixture_dirs`] (anchored with leading `/`) and
    /// [`Self::test_fixture_suffixes`].
    fn is_test_fixture(&self, path: &str) -> bool {
        let lower = path.to_ascii_lowercase();
        if self
            .test_fixture_suffixes()
            .iter()
            .any(|s| lower.ends_with(s))
        {
            return true;
        }
        let anchored = format!("/{lower}");
        self.test_fixture_dirs()
            .iter()
            .any(|d| anchored.contains(d))
    }

    /// Whether `path` is a top-level test-binary entry point — the root of an
    /// integration test target the build system links separately from the
    /// production binary (e.g. Rust's `tests/foo.rs`,
    /// `crates/X/tests/foo.rs`). Languages with no equivalent return `false`;
    /// the dead-code analyzer uses this to seed reachability from test
    /// binaries without dragging in nested `tests/foo/helper.rs` modules.
    /// Default false (most languages don't separate test binaries this way).
    fn is_test_target_root(&self, _path: &str) -> bool {
        false
    }

    /// Whether `path` is a conventional non-production directory whose files
    /// should be default-excluded from hotspot ranking (scripts/, examples/,
    /// benches/, …).
    fn is_conventional_path(&self, _path: &str) -> bool {
        false
    }

    /// Directory prefixes this language's ecosystem treats as non-production
    /// (scripts, examples, benches, testdata, fixtures, …). The facade
    /// unions these across every registered language to answer
    /// [`crate::facade::is_conventional_path`] for paths whose own language
    /// did not claim them. Default empty.
    fn conventional_dirs(&self) -> &'static [&'static str] {
        &[]
    }

    /// Basenames of "barrel" / package-index files that re-export symbols
    /// for downstream importers (Python `__init__.py`, TS/JS `index.ts` etc).
    /// Dead-code analysis treats a symbol as reachable if its name appears
    /// inside any of these files. Default empty.
    fn barrel_file_basenames(&self) -> &'static [&'static str] {
        &[]
    }

    /// Hook for Rust's foreign-trait impl sweep, Python's
    /// `if __name__ == '__main__':` probe, Cargo targets, and other
    /// language-specific live-root discoveries that require file I/O.
    ///
    /// Each returned [`LiveRoot`] carries a stable `reason` key, a
    /// human-readable message, a confidence level, and whether the root
    /// should count as an **explicit** entry (suppresses the
    /// "no-entry-points" warning in analysis). Analysis consumers never
    /// need to know which language emitted each root.
    fn additional_live_roots(&self, _root: Option<&Path>, _all_files: &[String]) -> Vec<LiveRoot> {
        Vec::new()
    }

    // ── Visibility ───────────────────────────────────────────────────────
    /// True when the symbol at `source_text` with the given `name` is
    /// restricted (non-public) under this language's visibility model.
    /// Default: underscore-prefix convention (Python/Ruby/Perl/Lua style).
    fn is_restricted(&self, _source_text: &str, name: &str) -> bool {
        name.starts_with('_')
    }

    // ── Complexity / type density ────────────────────────────────────────
    fn family(&self) -> LanguageFamily {
        LanguageFamily::Unknown
    }

    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }

    fn function_length_warn(&self) -> Option<u32> {
        match self.family() {
            LanguageFamily::BracedStatic => Some(75),
            LanguageFamily::ScriptedDynamic => Some(40),
            LanguageFamily::FunctionalExpression => Some(30),
            LanguageFamily::DeclarativeMarkup | LanguageFamily::Unknown => None,
        }
    }

    fn fallback_type_baseline(&self) -> Option<f64> {
        match self.family() {
            LanguageFamily::BracedStatic => Some(1.0),
            LanguageFamily::ScriptedDynamic => Some(0.3),
            LanguageFamily::FunctionalExpression => Some(0.5),
            LanguageFamily::DeclarativeMarkup | LanguageFamily::Unknown => None,
        }
    }

    /// Shape of this language's type annotations. Drives the default
    /// [`Self::measured_type_density`] and [`Self::supports_measured_types`]
    /// so each language only needs to declare its style — no per-language
    /// dispatch code. [`TypeAnnotationStyle::Custom`] means the language
    /// implements its own signature walker (Go, Python, Rust, PHP).
    fn type_annotation_style(&self) -> TypeAnnotationStyle {
        TypeAnnotationStyle::Custom
    }

    /// Whether measured type density is available for this language. Default
    /// is `true` when [`Self::type_annotation_style`] is anything other than
    /// [`TypeAnnotationStyle::Custom`]; languages with a bespoke walker
    /// override to `true` as well.
    fn supports_measured_types(&self) -> bool {
        !matches!(self.type_annotation_style(), TypeAnnotationStyle::Custom)
    }

    /// Measure the density of informative type annotations in `source`.
    /// Default dispatches on [`Self::type_annotation_style`]. Languages with
    /// custom signature shapes (Go's grouped params, PHP's complex union
    /// types) override.
    fn measured_type_density(&self, source: &str) -> Option<f64> {
        match self.type_annotation_style() {
            TypeAnnotationStyle::ColonAfterName => {
                crate::signature_density::measure_colon_after_name(source, self, false, |_| false)
            }
            TypeAnnotationStyle::TypeBeforeName => {
                crate::signature_density::measure_type_before_name(source, self)
            }
            TypeAnnotationStyle::Custom => None,
        }
    }

    /// Normalized type tokens that collapse to this language's "any" shape
    /// via exact match (after `normalize_type_token`). Examples: Go's
    /// `any` / `interface{}`, C#'s `dynamic` / `object`, Python's `any` /
    /// `object`, Kotlin's `any`. Default empty — languages without a weak-
    /// type vocabulary inherit the strict default.
    fn weak_type_tokens(&self) -> &'static [&'static str] {
        &[]
    }

    /// Normalized type-token substrings that mark a weak annotation (e.g.
    /// Rust's `<any>` and `dynany`, Python's `<any>` and `typing.any`).
    /// Default empty.
    fn weak_type_substrings(&self) -> &'static [&'static str] {
        &[]
    }

    /// Score one type token from a signature: 1.0 when the annotation is
    /// informative, 0.0 when it collapses to the language's "any" shape
    /// (Python's `Any`, TypeScript's `any`, Go's `interface{}`, …), and
    /// intermediate values for languages that recognize soft gradations
    /// (TypeScript's `unknown` → 0.3). Default uses `weak_type_tokens()` /
    /// `weak_type_substrings()`; languages with gradations override.
    fn type_token_score(&self, token: &str) -> f64 {
        let normalized = crate::signature_density::normalize_type_token(token);
        if normalized.is_empty() {
            return 0.0;
        }
        if self.weak_type_tokens().contains(&normalized.as_str()) {
            return 0.0;
        }
        if self
            .weak_type_substrings()
            .iter()
            .any(|s| normalized.contains(s))
        {
            return 0.0;
        }
        1.0
    }

    /// True when this language writes a return-type after the parameter
    /// list as `: Type` (TypeScript, Kotlin, Scala, PHP, Swift). Languages
    /// that only use the arrow form (`-> Type`) or a prefix return type
    /// (Java/C++) leave this `false`. Used by the generic signature-density
    /// parser to decide whether a `:` following the params is a return type
    /// or a block opener.
    fn uses_colon_return_suffix(&self) -> bool {
        false
    }

    /// Branch keywords that contribute one decision point to cyclomatic
    /// complexity (split on non-identifier boundaries). Default covers the
    /// C-family + scripted union; languages override for their specific
    /// vocabulary (Rust's `match`, SQL's `CASE WHEN`, Ruby's `unless`, …).
    fn branch_keywords(&self) -> &'static [&'static str] {
        &[
            "if", "elif", "else if", "for", "while", "except", "catch", "case", "and", "or",
        ]
    }

    /// Inline branch-operator strings that contribute one decision point
    /// (e.g. `&&`, `||`). Default covers C-family operators + Python
    /// ternary; languages without short-circuit operators override empty.
    fn branch_operators(&self) -> &'static [&'static str] {
        &["&&", "||"]
    }

    /// Line-comment prefixes to skip while scanning for branch tokens.
    /// Default covers the C-family (`//`), shell / Python / Perl (`#`),
    /// and SQL / Lua (`--`).
    fn line_comment_prefixes(&self) -> &'static [&'static str] {
        &["//", "#", "--"]
    }

    /// Line-leading prefixes that mark a declaration / import / export
    /// rather than an identifier usage site. Used by reference-evidence
    /// fallback scans to avoid counting `class Foo` as a reference to
    /// some earlier `Foo`. Default empty — languages with distinct
    /// declaration keywords override.
    fn declaration_line_prefixes(&self) -> &'static [&'static str] {
        &[]
    }

    /// Per-language cyclomatic-complexity heuristic. Default runs the
    /// generic branch counter over `branch_keywords()`, `branch_operators()`,
    /// and `line_comment_prefixes()` — plus a `?` ternary scan that avoids
    /// `?.` / `??`. Declarative markup families (YAML, JSON, TOML, XML,
    /// Markdown) short-circuit to the base complexity of `1` because they
    /// have no control flow.
    fn cyclomatic_complexity(&self, source: &str) -> usize {
        if matches!(
            self.family(),
            crate::family::LanguageFamily::DeclarativeMarkup
        ) {
            return 1;
        }
        crate::complexity::count_cyclomatic(
            source,
            self.branch_keywords(),
            self.branch_operators(),
            self.line_comment_prefixes(),
        )
    }

    /// Returns true when a symbol-id names a test symbol under this
    /// language's conventions (Rust's `tests.` parent scope, Python's
    /// `test_` prefix, Go's `TestXxx` etc.). Default false.
    ///
    /// Use the `SymbolIdRef` accessors (`leaf_name`, `has_scope`, `kind`)
    /// instead of reparsing the raw string — callers never have to know
    /// the serialized shape.
    fn is_test_symbol_id(&self, _id: dowser_core::SymbolIdRef<'_>) -> bool {
        false
    }

    /// Advisory note to surface when `inspect_structure view=class_hierarchy`
    /// is requested on a file whose language does not model inheritance the
    /// way C++/Java/Python do (Rust traits, Go interfaces, etc.). Default
    /// `None`. Languages that want to steer callers toward the right
    /// relationship graph override.
    fn class_hierarchy_note(&self) -> Option<&'static str> {
        None
    }

    /// Detect whether `symbol_name` (defined in `defining_source` at
    /// `defining_path`) participates in a trait/interface dispatch surface
    /// that ranges across the repo. Returns `(trait_name, impl_files)`
    /// where `impl_files` are the other files in `repo_files` whose method
    /// must rename in lockstep. Default `None` — most languages don't
    /// have this concept; Rust traits and Go interfaces opt in.
    ///
    /// `repo_files` is `&[(path, source)]` for the indexed repo; callers
    /// pass it once instead of forcing each language to query storage.
    fn detect_trait_impl_sites(
        &self,
        _defining_source: &str,
        _repo_files: &[(String, String)],
        _defining_path: &str,
        _symbol_name: &str,
    ) -> Option<(String, Vec<String>)> {
        None
    }

    /// Whole-word match for `name` inside `source` under this language's
    /// identifier boundary rules. Default: ASCII `[A-Za-z0-9_]` boundary
    /// via [`crate::text_match::word_match`]. Per-language overrides carry
    /// non-ASCII identifier semantics (Ruby's trailing `?`/`!`, CSS `-`,
    /// JS `$`) without every caller having to branch on language id.
    fn mentions_identifier(&self, source: &str, name: &str) -> bool {
        crate::text_match::word_match(source, name)
    }

    /// Case-insensitive variant of [`Self::mentions_identifier`]. Default
    /// lowercases both sides and delegates; languages whose identifier
    /// boundary is case-sensitive (most of them) inherit this for free.
    fn mentions_identifier_ignore_case(&self, source: &str, name: &str) -> bool {
        crate::text_match::ci_word_match(source, name)
    }

    /// Normalise a function body for structural duplicate detection. The
    /// returned bytes feed a stable hasher (blake3) — two bodies that
    /// should be considered duplicates must normalise identically.
    ///
    /// Default: strip blank lines and line-comment lines (using this
    /// language's `line_comment_prefixes()`), trim each remaining line, and
    /// join with `\n`. Per-language overrides preserve structure the default
    /// destroys (Python/YAML indentation, SQL keyword case, …).
    fn normalise_body_for_dedup(&self, source: &str) -> Vec<u8> {
        let prefixes = self.line_comment_prefixes();
        let mut out = String::with_capacity(source.len());
        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if prefixes.iter().any(|p| trimmed.starts_with(p)) {
                continue;
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(trimmed);
        }
        out.into_bytes()
    }

    // ── Frameworks (per-project providers this language contributes) ─────
    /// Static list of framework-provider factories this language ships with.
    /// Stored as `fn() -> Box<dyn FrameworkProvider>` rather than static refs
    /// so each discovery run gets a fresh loader state.
    fn framework_providers(&self) -> &'static [fn() -> Box<dyn FrameworkProvider>] {
        &[]
    }

    /// Return a simple `FrameworkProfile` — `ignore_patterns`,
    /// `entry_point_patterns`, `layer_definitions` — when this language
    /// recognizes a framework marker at `root`. Default `None`. Languages
    /// that own multiple frameworks implement internal priority here.
    fn simple_framework_profile(
        &self,
        _root: &Path,
    ) -> Option<crate::frameworks::profiles::FrameworkProfile> {
        None
    }

    // ── Package / build discovery ────────────────────────────────────────
    /// Directory names whose subtrees this language's ecosystem treats as
    /// generated / vendored / build-output, and that `packages::walker`
    /// should prune. VCS metadata (`.git`, `.hg`, `.svn`) and dot-prefixed
    /// directories are handled generically in the walker; per-language
    /// names (`node_modules`, `target`, `__pycache__`, `vendor`, …) live
    /// on the language that owns them.
    fn pruned_dirs(&self) -> &'static [&'static str] {
        &[]
    }

    /// Directory segments that hold build output, package caches, or vendored
    /// deps for this ecosystem. Used by retrieval/ranking layers to deprioritize
    /// search/reference hits under these paths. Default: empty. Example: Rust
    /// returns `["target"]`.
    ///
    /// Distinct from [`Self::pruned_dirs`] (which the package walker prunes
    /// at ingest time) only by call site — same segments can appear in both.
    fn vendor_segments(&self) -> &'static [&'static str] {
        &[]
    }

    /// Reserved identifiers for this language (keywords, built-in type names).
    /// Used by cross-language scans to avoid treating keywords as identifier
    /// matches. Default: empty.
    fn reserved_identifiers(&self) -> &'static [&'static str] {
        &[]
    }

    /// When true, hits in files of this language are weak reference targets:
    /// a mention of `fn foo` inside a YAML/TOML/JSON config is usually config
    /// data, not a code reference. Default: true (code languages are strong
    /// reference targets). Override to false for data/prose languages.
    fn is_reference_target(&self) -> bool {
        true
    }

    /// Repo-relative basenames whose entries are config / manifest noise in
    /// a code outline (e.g. `package.json`, `Cargo.toml`, `build.rs`). The
    /// owning language decides — storage / engine never branch on filename.
    /// Matched against the full path either as exact equality (`build.rs`)
    /// or as a `/<basename>` suffix (`crates/foo/build.rs`).
    ///
    /// Default: empty.
    fn outline_noise_basenames(&self) -> &'static [&'static str] {
        &[]
    }

    /// Symbol kinds that uniquely identify this language vs. its siblings
    /// that share a grammar (e.g. C++ vs. C share `tree_sitter_cpp`; only
    /// C++ emits `class` / `namespace`). When non-empty, the engine issues
    /// one batch query per language asking "which of these paths emit a
    /// symbol in this set?" and passes the boolean result to
    /// [`Language::reclassify_file_language`].
    ///
    /// Default: empty (no post-ingest reclassification).
    fn distinctive_symbol_kinds(&self) -> &'static [&'static str] {
        &[]
    }

    /// Post-ingest override for a file's stored language. Called for every
    /// file whose ingest-time language is `self`, with `has_distinctive_kinds`
    /// set to the result of probing [`Language::distinctive_symbol_kinds`].
    /// Return `Some("c")` etc. to relabel, or `None` to keep the original.
    ///
    /// Used today by Cpp to downgrade `.h` headers that contain no C++-only
    /// constructs to `c`. Default: never override.
    fn reclassify_file_language(
        &self,
        _path: &str,
        _has_distinctive_kinds: bool,
    ) -> Option<&'static str> {
        None
    }

    /// Method / function names whose repetition across files is idiomatic
    /// for the language and should not be scored as a uniqueness collision
    /// by `symbol_uniqueness`. Default: empty — every language opts in
    /// explicitly.
    ///
    /// Examples: Rust `new`/`default`/`from`, Go `New`/`String`/`Error`,
    /// Python `__init__`/`__repr__`, Java `equals`/`hashCode`/`toString`,
    /// JS/TS `constructor`/`render`, Ruby `initialize`/`to_s`,
    /// PHP `__construct`, Elixir `start_link`/`handle_call`.
    ///
    /// A reader of the language expects dozens of these per repo and
    /// disambiguates by owning type, not by name. Counting every `fn new`
    /// as an ambiguity would cap Rust codebases near 0.2 by construction.
    fn idiomatic_symbol_names(&self) -> &'static [&'static str] {
        &[]
    }

    /// Per-language module-path rename engine. When `Some`, callers (today
    /// `evaluate_impact analysis=path_rename`) dispatch through the engine to
    /// preview a `from → to` prefix rewrite across the workspace. Default
    /// `None` — languages without a native module-path convention (scripting
    /// languages, config files, etc.) don't offer this surface.
    fn path_rename_engine(&self) -> Option<&'static dyn crate::path_rename::PathRenameEngine> {
        None
    }
}

#[cfg(test)]
mod default_body_tests {
    use super::*;
    use crate::builtin_registry;
    use crate::fallback::FALLBACK_LANGUAGE;

    #[test]
    fn function_length_warn_covers_every_non_markup_family() {
        for lang in builtin_registry().all() {
            let expected = match lang.family() {
                LanguageFamily::BracedStatic => Some(75),
                LanguageFamily::ScriptedDynamic => Some(40),
                LanguageFamily::FunctionalExpression => Some(30),
                LanguageFamily::DeclarativeMarkup | LanguageFamily::Unknown => None,
            };
            assert_eq!(
                lang.function_length_warn(),
                expected,
                "{} mismatched function_length_warn",
                lang.id().as_str()
            );
        }
    }

    #[test]
    fn fallback_type_baseline_returns_valid_range() {
        for lang in builtin_registry().all() {
            match lang.fallback_type_baseline() {
                Some(v) => assert!(
                    (0.0..=1.0).contains(&v),
                    "{} baseline {v} outside [0,1]",
                    lang.id().as_str()
                ),
                None => assert!(matches!(
                    lang.family(),
                    LanguageFamily::DeclarativeMarkup | LanguageFamily::Unknown
                )),
            }
        }
    }

    #[test]
    fn branch_keywords_default_has_common_tokens() {
        let kws = (&FALLBACK_LANGUAGE as &dyn Language).branch_keywords();
        for expected in ["if", "else if", "for", "while", "case", "and", "or"] {
            assert!(kws.contains(&expected), "default missing {expected}");
        }
    }

    #[test]
    fn normalise_body_for_dedup_strips_blanks_and_line_comments() {
        let lang: &dyn Language = &FALLBACK_LANGUAGE;
        let src = "fn a() {\n  // gone\n\n  return 1;\n}\n";
        let out = lang.normalise_body_for_dedup(src);
        let out_str = std::str::from_utf8(&out).unwrap();
        assert!(!out_str.contains("// gone"));
        assert!(!out_str.contains("\n\n"));
        assert!(out_str.contains("return 1;"));
    }

    #[test]
    fn normalise_body_for_dedup_is_deterministic_across_whitespace() {
        let lang: &dyn Language = &FALLBACK_LANGUAGE;
        let a = lang.normalise_body_for_dedup("x = 1\n\ny = 2\n");
        let b = lang.normalise_body_for_dedup("  x = 1\ny = 2");
        assert_eq!(a, b);
    }
}
