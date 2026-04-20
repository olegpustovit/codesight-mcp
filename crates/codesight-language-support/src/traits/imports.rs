//! Import resolution axis (LANGUAGES.md §4.2).
//!
//! Owns: whether the language supports import extraction, extraction mode
//! (tree-sitter first vs. fallback), specifier resolution to workspace
//! files, alias loading (tsconfig, pyproject), bundle display, and
//! import-edge fact-quality overrides.
//!
//! Phase D.5 lifts `ImportExtractionMode` out of `codesight-imports` so the
//! registry is the single source of truth for "does this language have
//! imports, and if so is extraction tree-sitter-first or fallback-only?".
//! Actual extraction code stays in `codesight-imports` for now and routes
//! through this trait via the facade.

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

pub trait ImportResolution: Send + Sync {
    /// Whether this language has any import-extraction behavior at all.
    fn supports_import_extraction(&self) -> bool {
        self.extraction_mode().is_some()
    }

    /// Extraction strategy for this language, or `None` when extraction is
    /// not implemented (the default for every language that doesn't
    /// explicitly opt in).
    fn extraction_mode(&self) -> Option<ImportExtractionMode> {
        None
    }

    /// Root package name for an import specifier (e.g. `"serde::de::T"` →
    /// `"serde"`, `"flask.blueprints"` → `"flask"`, `".utils"` → `""`).
    /// Default returns the specifier unchanged — languages override when
    /// they have package-path semantics.
    fn root_package_from_specifier(&self, specifier: &str) -> String {
        specifier.to_string()
    }
}

/// Default: no imports. Languages that don't model imports (config files,
/// plain text, CSS, etc.) rely on this implementation.
pub struct NoopImports;

impl ImportResolution for NoopImports {}
