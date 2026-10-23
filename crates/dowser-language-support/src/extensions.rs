//! Extension classification helpers — source of truth for "is this a code
//! file" / "is this a doc file" outside of any individual `Language` impl.
//!
//! Hooks, operator tooling, and any caller that needs to decide which MCP
//! surface to route at should use these helpers instead of hardcoding
//! extension lists.

/// Documentation extensions handled by the retrieval doc-index pipeline.
///
/// Markdown is the one language that appears in *both* the language
/// registry (for section-level symbol extraction) and the doc pipeline;
/// `code_extensions()` excludes it so callers that classify paths do not
/// route `.md` to the code domain.
pub const DOC_EXTENSIONS: &[&str] = &["md", "mdx", "rst", "adoc", "asciidoc", "txt", "org", "tex"];

fn ext_lower<'a>(s: &'a str) -> std::borrow::Cow<'a, str> {
    if s.bytes().all(|b| !b.is_ascii_uppercase()) {
        std::borrow::Cow::Borrowed(s)
    } else {
        std::borrow::Cow::Owned(s.to_ascii_lowercase())
    }
}

/// Every extension claimed by any language in the builtin registry, minus
/// the doc-routed set. Deduplicated, sorted — safe to call once and cache.
pub fn code_extensions() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = crate::builtin_registry()
        .all()
        .flat_map(|l| l.manifest().extensions.iter().copied())
        .filter(|ext| !DOC_EXTENSIONS.contains(ext))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// True when `ext` (bare, no leading dot) is claimed by a registered
/// language and is not doc-routed.
pub fn is_code_extension(ext: &str) -> bool {
    let lower = ext_lower(ext);
    let lower = lower.as_ref();
    if DOC_EXTENSIONS.contains(&lower) {
        return false;
    }
    crate::builtin_registry()
        .all()
        .any(|l| l.manifest().extensions.contains(&lower))
}

/// True when `ext` (bare, no leading dot) is a doc-indexed extension.
pub fn is_doc_extension(ext: &str) -> bool {
    let lower = ext_lower(ext);
    let lower = lower.as_ref();
    DOC_EXTENSIONS.contains(&lower)
}

/// True when hits in `path` are weak reference targets — data files, prose
/// files, or lockfiles where a mention of `fn foo` is config data, not a
/// code reference. `find_references` filters these unless
/// `include_non_code=true`.
///
/// Resolution: look up the language for the path's extension and consult
/// [`crate::Language::is_reference_target`]. Extensions not claimed by any
/// registered language fall back to the residual set (`lock`, `csv`, `tsv`)
/// — files whose contents are tabular or VCS metadata.
#[must_use]
pub fn is_non_code_reference_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let Some(dot) = lower.rfind('.') else {
        return false;
    };
    let ext = &lower[dot + 1..];

    // Registry first: a language that owns this extension and explicitly
    // marks itself as a non-reference-target wins.
    for lang in crate::builtin_registry().all() {
        if lang.manifest().extensions.contains(&ext) && !lang.is_reference_target() {
            return true;
        }
    }
    // Residual non-code extensions that no language owns.
    matches!(ext, "lock" | "csv" | "tsv")
}

/// True for paths that should be excluded by default from repo-wide
/// structural scans (rename_safety, dead_code, reference scans). Matches
/// vendored/asset/fixture directories and non-code data files.
///
/// `path` is expected to be repo-relative with forward slashes.
#[must_use]
pub fn is_vendored_or_fixture_path(path: &str) -> bool {
    is_non_code_reference_path(path) || crate::is_vendored_path(path)
}

#[cfg(test)]
mod tests_path_exclusion {
    use super::*;

    #[test]
    fn vendored_or_fixture_hits() {
        assert!(is_vendored_or_fixture_path(
            "crates/dowser-language-support/tests/fixtures/code_reference/csharp/sample.cs"
        ));
        assert!(is_vendored_or_fixture_path("node_modules/lodash/index.js"));
        assert!(is_vendored_or_fixture_path("target/debug/build/x.rs"));
        assert!(is_vendored_or_fixture_path("assets/tokenizer.json"));
        assert!(is_vendored_or_fixture_path("fixtures/sample.rs"));
        assert!(is_vendored_or_fixture_path("testdata/x.go"));
    }

    #[test]
    fn real_source_is_not_excluded() {
        assert!(!is_vendored_or_fixture_path(
            "crates/dowser-analysis/src/impact.rs"
        ));
        assert!(!is_vendored_or_fixture_path("src/main.rs"));
        assert!(!is_vendored_or_fixture_path("tests/rename_plan.rs"));
    }
}
