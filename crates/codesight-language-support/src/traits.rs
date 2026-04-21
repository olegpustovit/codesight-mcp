//! The consolidated `Language` trait — one trait per language, every method
//! default-bodied so overrides are opt-in.
//!
//! This is the single home every crate above L0.75 consumes: downstream
//! callers take a `&dyn Language` from [`crate::LanguageRegistry`] and call
//! methods directly. Legacy axis traits (`SymbolExtraction`, `ImportResolution`,
//! `LivenessPolicy`, `PackageManifest`, `VisibilityModel`, `ComplexityModel`)
//! are gone — their methods fold in here.

use std::collections::HashMap;
use std::path::Path;

use crate::family::{LanguageFamily, NestingMode};
use crate::fallback;
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

    /// SCM tags query for this language. Default reads from `grammar_spec()`.
    fn tags_query(&self) -> Option<&'static str> {
        self.grammar_spec().map(|spec| spec.tags_scm)
    }

    /// SCM injections query. Default reads from `grammar_spec()`.
    fn injections_query(&self) -> Option<&'static str> {
        self.grammar_spec().and_then(|spec| spec.injections_scm)
    }

    // ── Symbol extraction (placeholder — generic SCM pipeline is the fallback) ──
    /// Whether this language produces extracted symbols. Languages without a
    /// grammar or a manual extractor return `false`.
    fn supports_extraction(&self) -> bool {
        false
    }

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

    /// Path aliases for this language discovered in `root` (e.g. tsconfig
    /// `paths`, pyproject `[tool.…]`). Default empty.
    fn path_aliases(&self, _root: &Path) -> HashMap<String, String> {
        HashMap::new()
    }

    // ── Packages / workspaces ────────────────────────────────────────────
    fn package_name_for_dir(&self, _dir: &Path) -> Option<String> {
        None
    }

    fn workspace_members(&self, _root: &Path) -> Vec<String> {
        Vec::new()
    }

    // ── Liveness / entrypoints / tests ───────────────────────────────────
    /// Basenames always treated as entry-point roots for this language
    /// (`main.rs`, `lib.rs`, `build.rs`, `main.py`, `__main__.py`, etc.).
    fn entrypoint_basenames(&self) -> &'static [&'static str] {
        &[]
    }

    /// Whether this language considers `path` a test file.
    fn is_test_file(&self, _path: &str) -> bool {
        false
    }

    /// Whether `path` is a fixture / golden / snapshot file this language's
    /// reachability analysis should skip entirely.
    fn is_test_fixture(&self, _path: &str) -> bool {
        false
    }

    /// Whether `path` is a conventional non-production directory whose files
    /// should be default-excluded from hotspot ranking (scripts/, examples/,
    /// benches/, …).
    fn is_conventional_path(&self, _path: &str) -> bool {
        false
    }

    /// Hook for Rust's foreign-trait impl sweep, Python's
    /// `if __name__ == '__main__':` probe, Cargo targets, and other
    /// language-specific live-root discoveries that require file I/O.
    fn additional_live_root_files(
        &self,
        _root: Option<&Path>,
        _all_files: &[String],
    ) -> Vec<String> {
        Vec::new()
    }

    // ── Visibility ───────────────────────────────────────────────────────
    /// True when the symbol at `source_text` with the given `name` is
    /// restricted (non-public) under this language's visibility model.
    /// Default: underscore-prefix convention (Python/Ruby/Perl/Lua style).
    fn is_restricted(&self, source_text: &str, name: &str) -> bool {
        fallback::underscore_prefix_restricted(source_text, name)
    }

    // ── Complexity / type density ────────────────────────────────────────
    fn family(&self) -> LanguageFamily {
        LanguageFamily::Unknown
    }

    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }

    fn function_length_warn(&self) -> Option<u32> {
        fallback::function_length_warn(self.family())
    }

    fn fallback_type_baseline(&self) -> Option<f64> {
        fallback::fallback_type_baseline(self.family())
    }

    fn supports_measured_types(&self) -> bool {
        false
    }

    fn measured_type_density(&self, _source: &str) -> Option<f64> {
        None
    }

    // ── Frameworks (per-project providers this language contributes) ─────
    /// Static list of framework-provider factories this language ships with.
    /// Stored as `fn() -> Box<dyn FrameworkProvider>` rather than static refs
    /// so each discovery run gets a fresh loader state.
    fn framework_providers(&self) -> &'static [fn() -> Box<dyn FrameworkProvider>] {
        &[]
    }
}
