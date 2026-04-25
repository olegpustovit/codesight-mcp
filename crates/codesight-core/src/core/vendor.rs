//! Path segment primitives. The aggregate vendor predicate
//! (`is_vendored_path`, `is_default_excluded`, `scope_opts_in`) lives in
//! `codesight-language-support::vendor` because it depends on the language
//! registry; this module keeps only the language-agnostic string check.

/// True when `path` contains `segment` as a whole `/`-delimited path
/// component. Equality (`path == segment`), prefix (`segment/…`), middle
/// (`…/segment/…`), or suffix (`…/segment`). Avoids matching `targets`
/// when the caller means `target`.
#[must_use]
pub fn path_contains_segment(path: &str, segment: &str) -> bool {
    if path == segment {
        return true;
    }
    let slash_prefix = format!("{segment}/");
    if path.starts_with(&slash_prefix) {
        return true;
    }
    let middle = format!("/{segment}/");
    if path.contains(&middle) {
        return true;
    }
    let suffix = format!("/{segment}");
    path.ends_with(&suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_match_anchors_on_slash_boundaries() {
        assert!(path_contains_segment("target/debug/build", "target"));
        assert!(path_contains_segment("a/target/b", "target"));
        assert!(path_contains_segment("a/target", "target"));
        assert!(path_contains_segment("target", "target"));
        assert!(!path_contains_segment("targets/foo", "target"));
        assert!(!path_contains_segment("a/targets", "target"));
    }
}
