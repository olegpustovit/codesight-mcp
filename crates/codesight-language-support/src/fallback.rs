//! `FallbackLanguage` — the concrete zero-override reference impl plus the
//! two generic path-based test/fixture heuristics used by the path facade.
//!
//! A language that overrides no method behaves exactly like `FallbackLanguage`
//! — this is how "every heuristic must be tied to a language or reuse the
//! fallback value" becomes a compile-time guarantee.

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

/// Truly-universal test-directory heuristic, used only when the path has no
/// registered language (binary blobs, `.unknown`, …). Language-specific
/// conventions (`__tests__/`, `_spec.rb`, `_test.go`, …) belong on their
/// owning language's `is_test_file`.
pub fn generic_is_test_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.starts_with("tests/")
        || lower.starts_with("test/")
        || lower.contains("/tests/")
        || lower.contains("/test/")
}

/// Universal fixture/testdata directory fallback for files with no
/// registered language. Tool-specific snapshot shapes (`.snap`,
/// Jest's `__snapshots__/`, Go's `golden/`) live on the languages that
/// own those tools.
pub fn generic_is_test_fixture(path: &str) -> bool {
    path.contains("/fixtures/")
        || path.starts_with("fixtures/")
        || path.contains("/testdata/")
        || path.starts_with("testdata/")
}

