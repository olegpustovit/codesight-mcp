//! `scope` / `exclude` / `file_pattern` path filtering.

/// Test whether a repo-relative file path passes the combined
/// scope / exclude / file_pattern filter used by `inspect_graph` and
/// `find_references` (FEEDBACK.md §6b).
///
/// - `scope`: substring inclusion (path must contain it).
/// - `exclude`: substrings to skip (any match rejects).
/// - `file_pattern`: `*` / `**` / `?` glob — matches the full path.
///
/// All `None` means unfiltered (accept).
pub(in crate::service) fn path_matches_filter(
    path: &str,
    scope: Option<&str>,
    exclude: Option<&[String]>,
    file_pattern: Option<&str>,
) -> bool {
    if let Some(scope) = scope
        && !scope.is_empty()
        && !path.contains(scope)
    {
        return false;
    }
    if let Some(excludes) = exclude {
        for ex in excludes {
            if !ex.is_empty() && path.contains(ex) {
                return false;
            }
        }
    }
    if let Some(pat) = file_pattern
        && !pat.is_empty()
        && globset::Glob::new(pat)
            .map(|g| !g.compile_matcher().is_match(path))
            .unwrap_or(true)
    {
        return false;
    }
    true
}
