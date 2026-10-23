//! Helpers for GitHub-style repo URLs and sparse-checkout tree selection.

/// Parse a GitHub repo reference into `(owner, name)` for common URL shapes.
pub fn parse_github_url(input: &str) -> Option<(String, String)> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }

    if !s.contains("://") {
        let mut parts = s.splitn(2, '/');
        let owner = parts.next()?.trim();
        let repo = parts.next()?.trim();
        if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
            return None;
        }
        if repo.contains('/') {
            return None;
        }
        let repo = repo.trim_end_matches(".git").trim_end_matches('/');
        if repo.is_empty() {
            return None;
        }
        return Some((owner.to_string(), repo.to_string()));
    }

    let rest = s
        .strip_prefix("https://github.com/")
        .or_else(|| s.strip_prefix("http://github.com/"))?;
    let mut parts = rest.splitn(2, '/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    let repo = repo.trim_end_matches(".git").trim_end_matches('/');
    if repo.is_empty() {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

/// Whether a repo-relative path should be ignored when enumerating GitHub tree blobs (`test_should_skip_file`).
pub fn should_skip_file_path(path: &str) -> bool {
    let norm = path.replace('\\', "/");
    for seg in norm.split('/') {
        if seg == "node_modules" {
            return true;
        }
    }
    norm.contains("vendor/github.com/")
        || norm.starts_with("vendor/github.com")
        || norm == "vendor/github.com"
}

fn is_readme_md(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case("readme.md"))
}

fn code_extension(path: &str) -> Option<&str> {
    let (_, ext) = path.rsplit_once('.')?;
    if ext.is_empty() {
        return None;
    }
    Some(ext)
}

/// Whether a blob path is treated as an indexable source file in sparse GitHub trees (`test_discover_source_files`).
pub fn is_github_tree_source_blob(path: &str) -> bool {
    if should_skip_file_path(path) {
        return false;
    }
    if is_readme_md(path) {
        return false;
    }
    let Some(ext) = code_extension(path) else {
        return false;
    };
    symslice_language_support::is_code_extension(ext)
}

/// How [`discover_github_tree_source_files`] applies file caps (explicit overrides config when both are set).
#[derive(Clone, Debug, Default)]
pub struct DiscoverOpts {
    pub config_max_files: Option<usize>,
    pub explicit_max_files: Option<usize>,
}

pub fn effective_max_files(opts: &DiscoverOpts) -> Option<usize> {
    opts.explicit_max_files.or(opts.config_max_files)
}

/// Select source file paths from a flat list of GitHub `tree` blob paths (reference `discover_source_files` behavior).
pub fn discover_github_tree_source_files(
    paths: &[&str],
    opts: &DiscoverOpts,
) -> (Vec<String>, usize, bool) {
    let mut eligible: Vec<String> = paths
        .iter()
        .copied()
        .filter(|p| is_github_tree_source_blob(p))
        .map(|p| p.replace('\\', "/"))
        .collect();
    eligible.sort_by(|a, b| {
        let a_src = a.starts_with("src/");
        let b_src = b.starts_with("src/");
        b_src.cmp(&a_src).then_with(|| a.cmp(b))
    });
    let total = eligible.len();
    let cap = effective_max_files(opts);
    let truncated = match cap {
        Some(m) => total > m,
        None => false,
    };
    let take = cap.unwrap_or(total).min(total);
    eligible.truncate(take);
    (eligible, total, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_urls_and_shorthand() {
        assert_eq!(
            parse_github_url("https://github.com/owner/repo"),
            Some(("owner".into(), "repo".into()))
        );
        assert_eq!(
            parse_github_url("https://github.com/owner/repo.git"),
            Some(("owner".into(), "repo".into()))
        );
        assert_eq!(
            parse_github_url("owner/repo"),
            Some(("owner".into(), "repo".into()))
        );
    }

    #[test]
    fn skip_rules_match_reference() {
        assert!(should_skip_file_path("node_modules/foo.js"));
        assert!(should_skip_file_path("vendor/github.com/foo.go"));
        assert!(!should_skip_file_path("src/main.py"));
    }
}
