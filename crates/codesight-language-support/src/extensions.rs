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

/// File extensions treated as data/docs for reference-search purposes. Hits
/// in these almost never mean "who references fn X" — `find_references`
/// filters them unless `include_non_code=true`.
///
/// Distinct from [`DOC_EXTENSIONS`]: a few of these (`json`, `yaml`, `toml`)
/// are claimed by their own `Language` impls for outline indexing, but
/// still shouldn't participate in identifier-usage search.
const NON_CODE_REFERENCE_EXTENSIONS: &[&str] = &[
    "json", "yaml", "yml", "toml", "xml", "md", "mdx", "csv", "tsv", "lock",
];

/// True when `path` ends in one of the non-code reference extensions above.
/// Pure function — callers decide the policy (`include_non_code` flag etc.).
#[must_use]
pub fn is_non_code_reference_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let Some(dot) = lower.rfind('.') else {
        return false;
    };
    NON_CODE_REFERENCE_EXTENSIONS
        .iter()
        .any(|ext| &lower[dot + 1..] == *ext)
}
