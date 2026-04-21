//! `FallbackLanguage` — the concrete zero-override reference impl, plus the
//! shared default heuristics every trait default delegates into.
//!
//! A language that overrides no method behaves exactly like `FallbackLanguage`
//! — this is how "every heuristic must be tied to a language or reuse the
//! fallback value" becomes a compile-time guarantee.

use crate::family::{LanguageFamily, NestingMode};
use crate::manifest::{LanguageId, LanguageManifest, UNKNOWN_LANGUAGE};
use crate::traits::Language;

/// Minimal manifest used by `FallbackLanguage`. Lives as a static so
/// `Language::manifest()` can return `&'static` without heap-allocating.
pub static FALLBACK_MANIFEST: LanguageManifest = LanguageManifest {
    display_name: "Unknown",
    aliases: &[],
    extensions: &[],
    filename_overrides: &[],
    compound_extensions: &[],
    grammar: None,
    profiles: &[],
    content_probe: None,
};

/// Zero-override reference language. The registry returns this (via
/// `Option::unwrap_or`) when asked about a path with no known identity.
pub struct FallbackLanguage;

impl Language for FallbackLanguage {
    fn id(&self) -> LanguageId {
        UNKNOWN_LANGUAGE
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &FALLBACK_MANIFEST
    }
}

/// Process-wide singleton. Use `&FALLBACK_LANGUAGE` when you need a
/// `&'static dyn Language`.
pub static FALLBACK_LANGUAGE: FallbackLanguage = FallbackLanguage;

// ── Default heuristics shared by trait method defaults ──────────────────────

/// Generic test-file heuristic applied when a concrete language's
/// `is_test_file` returns false. Matches `tests/` / `test/` / `__tests__/`
/// / `spec/` directories anywhere in the path.
pub fn generic_is_test_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.starts_with("tests/")
        || lower.starts_with("test/")
        || lower.contains("/tests/")
        || lower.contains("/test/")
        || lower.contains("/__tests__/")
        || lower.contains("/spec/")
}

/// Generic fixture/golden/snapshot detection. Used as the cross-language
/// fallback after each language's own `is_test_fixture` hook runs.
pub fn generic_is_test_fixture(path: &str) -> bool {
    path.contains("/fixtures/")
        || path.starts_with("fixtures/")
        || path.contains("/lang_samples/")
        || path.contains("/testdata/")
        || path.starts_with("testdata/")
        || path.contains("/snapshots/")
        || path.contains("/__snapshots__/")
        || path.contains("/golden/")
}

/// Underscore-prefix privacy convention (Python / Ruby / Perl / Lua / etc.).
/// Used as the trait default; languages without any visibility concept still
/// get reasonable "is restricted" answers.
pub fn underscore_prefix_restricted(_source_text: &str, name: &str) -> bool {
    name.starts_with('_')
}

/// Soft function-length threshold by family.
pub fn function_length_warn(family: LanguageFamily) -> Option<u32> {
    match family {
        LanguageFamily::BracedStatic => Some(75),
        LanguageFamily::ScriptedDynamic => Some(40),
        LanguageFamily::FunctionalExpression => Some(30),
        LanguageFamily::DeclarativeMarkup | LanguageFamily::Unknown => None,
    }
}

/// Baseline type-annotation score when the language doesn't support measured
/// types or measurement returns `None`.
pub fn fallback_type_baseline(family: LanguageFamily) -> Option<f64> {
    match family {
        LanguageFamily::BracedStatic => Some(1.0),
        LanguageFamily::ScriptedDynamic => Some(0.3),
        LanguageFamily::FunctionalExpression => Some(0.5),
        LanguageFamily::DeclarativeMarkup | LanguageFamily::Unknown => None,
    }
}

/// Default nesting mode for a family. Used by language impls that only
/// override `family()` without a custom `nesting_mode()`.
pub fn default_nesting_mode(family: LanguageFamily) -> NestingMode {
    match family {
        LanguageFamily::BracedStatic | LanguageFamily::ScriptedDynamic => NestingMode::Brace,
        LanguageFamily::FunctionalExpression
        | LanguageFamily::DeclarativeMarkup
        | LanguageFamily::Unknown => NestingMode::Indent,
    }
}
