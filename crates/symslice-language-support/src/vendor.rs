//! Vendored / asset / fixture path exclusions, unioned across every
//! registered [`Language`].
//!
//! Single source of truth for "this path's contents shouldn't leak into
//! retrieval ranking or reference scans even though it lives in the index".
//! Callers at the search/rank layer (search_catalog ranker, find_references,
//! rename_safety, dead_code, blast_radius) consult this to deprioritize or
//! skip matches — ingestion is unaffected.
//!
//! Patterns are simple `/<segment>/` contains-checks rather than glob
//! evaluation: the excluded prefixes are all plain directory segments and
//! a contains-check runs in O(len) without pulling a glob crate in.
//!
//! The opt-in is two-fold:
//! 1. Caller passes `include_vendored=true` — every path is admitted.
//! 2. Caller passes an explicit `scope=` that starts with one of the
//!    excluded segments (e.g. `scope="crates/symslice-retrieval/assets/"`) —
//!    the scope signal overrides the default penalty via [`scope_opts_in`].
//!
//! The segment universe is the union of every registered language's
//! [`crate::Language::vendor_segments`] plus the [`UNIVERSAL_VENDOR_SEGMENTS`]
//! cross-language conventions.

use rustc_hash::FxHashSet;

use symslice_core::path_contains_segment;

use crate::builtin_registry;

/// Cross-language conventions: VCS metadata, generic build outputs, fixture
/// trees, and symslice's own index. **No language-specific entries** —
/// `node_modules`, `target`, `__pycache__` etc. live on the per-language
/// trait method [`Language::vendor_segments`].
const UNIVERSAL_VENDOR_SEGMENTS: &[&str] = &[
    // VCS
    ".git",
    ".hg",
    ".svn",
    // Cross-language build/dep conventions
    "vendor",
    "build",
    "dist",
    "out",
    "coverage",
    // Generic test data
    "fixtures",
    "testdata",
    "__fixtures__",
    // Generic assets
    "assets",
    // Symslice's own index
    ".symslice",
    ".symslice-index",
];

/// Size cutoff at which a `.json` file is treated as a vendor dump
/// (tokenizer vocab, model-weight metadata, etc.). 64 KiB.
pub const VENDOR_JSON_MAX_BYTES: u64 = 64 * 1024;

/// Build the unioned list of vendor segments — every registered language's
/// [`Language::vendor_segments`] plus [`UNIVERSAL_VENDOR_SEGMENTS`]. Tens of
/// segments total; safe to call per-request.
fn all_vendor_segments() -> FxHashSet<&'static str> {
    let mut set: FxHashSet<&'static str> = UNIVERSAL_VENDOR_SEGMENTS.iter().copied().collect();
    for lang in builtin_registry().all() {
        set.extend(lang.vendor_segments().iter().copied());
    }
    set
}

/// True when `path` lives under one of the registered vendor segments.
/// Matches whole path segments: `src/assets/foo` hits (segment match),
/// `src/fix` does not (`fix` != `fixtures`).
#[must_use]
pub fn is_vendored_path(path: &str) -> bool {
    all_vendor_segments()
        .iter()
        .any(|seg| path_contains_segment(path, seg))
}

/// True when `path` is a `.json` file bigger than [`VENDOR_JSON_MAX_BYTES`].
/// Tokenizer dumps and similar large JSON blobs pollute FTS/BM25 with
/// high-frequency token shards; deprioritize them by default.
#[must_use]
pub fn is_vendor_json_dump(path: &str, size_bytes: u64) -> bool {
    size_bytes > VENDOR_JSON_MAX_BYTES
        && path
            .rsplit('.')
            .next()
            .map(|ext| ext.eq_ignore_ascii_case("json"))
            .unwrap_or(false)
}

/// Combined default predicate: is this path one we should deprioritize /
/// exclude by default when surfacing matches to the user? Takes optional
/// `size_bytes` so callers that already have it (storage `file_bytes()`)
/// can include the large-JSON rule in one call.
#[must_use]
pub fn is_default_excluded(path: &str, size_bytes: Option<u64>) -> bool {
    if is_vendored_path(path) {
        return true;
    }
    match size_bytes {
        Some(size) => is_vendor_json_dump(path, size),
        None => false,
    }
}

/// True when `scope` explicitly targets an excluded segment — in which
/// case the caller clearly wants to see those paths and the default
/// exclusion should *not* apply. Normalizes a leading `./` and trims
/// trailing slashes before the segment check.
#[must_use]
pub fn scope_opts_in(scope: &str) -> bool {
    let trimmed = scope.trim_start_matches("./").trim_end_matches('/');
    if trimmed.is_empty() {
        return false;
    }
    all_vendor_segments().iter().any(|seg| {
        trimmed == *seg
            || trimmed.starts_with(&format!("{seg}/"))
            || trimmed.contains(&format!("/{seg}/"))
            || trimmed.ends_with(&format!("/{seg}"))
    })
}

/// Unioned reserved-identifier set — every registered language's
/// [`Language::reserved_identifiers`] folded into one [`FxHashSet`]. Callers
/// (today: usage_edges scanner) build this once and reuse it across the scan.
#[must_use]
pub fn all_reserved_identifiers() -> FxHashSet<&'static str> {
    let mut set: FxHashSet<&'static str> = FxHashSet::default();
    for lang in builtin_registry().all() {
        set.extend(lang.reserved_identifiers().iter().copied());
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_vendored_path_matches_segments() {
        assert!(is_vendored_path(
            "crates/symslice-retrieval/assets/embedder/potion-base-8M/tokenizer.json"
        ));
        assert!(is_vendored_path(
            "crates/x/tests/fixtures/code_reference/csharp/sample.cs"
        ));
        assert!(is_vendored_path("node_modules/foo/index.js"));
        assert!(is_vendored_path("target/debug/build"));
        assert!(is_vendored_path("vendor/github.com/foo/bar.go"));
        assert!(is_vendored_path(".git/HEAD"));
        assert!(is_vendored_path("some/testdata/golden.txt"));
    }

    #[test]
    fn is_vendored_path_rejects_near_misses() {
        assert!(!is_vendored_path("src/main.rs"));
        assert!(!is_vendored_path("crates/foo/src/targets.rs"));
        assert!(!is_vendored_path("fixtureless.rs"));
        assert!(!is_vendored_path("src/vendors.rs"));
    }

    #[test]
    fn is_vendor_json_dump_gates_on_extension_and_size() {
        assert!(is_vendor_json_dump("tokenizer.json", 200_000));
        assert!(is_vendor_json_dump("path/TOKENIZER.JSON", 70_000));
        assert!(!is_vendor_json_dump("tokenizer.json", 1_024));
        assert!(!is_vendor_json_dump("big.rs", 200_000));
    }

    #[test]
    fn scope_opts_in_detects_segment_anchored_scope() {
        assert!(scope_opts_in("assets/"));
        assert!(scope_opts_in("crates/symslice-retrieval/assets/"));
        assert!(scope_opts_in("tests/fixtures"));
        assert!(scope_opts_in("./vendor"));
        assert!(!scope_opts_in("src/service/"));
        assert!(!scope_opts_in(""));
        assert!(!scope_opts_in("src/assetsish"));
    }

    #[test]
    fn is_default_excluded_covers_both_rules() {
        assert!(is_default_excluded("assets/x.rs", None));
        assert!(is_default_excluded("tokenizer.json", Some(200_000)));
        assert!(!is_default_excluded("src/main.rs", Some(100)));
        assert!(!is_default_excluded("tokenizer.json", Some(100)));
        assert!(!is_default_excluded("tokenizer.json", None));
    }
}
