//! Path remapping for Docker / remote-dev workflows.
//!
//! When `SYMSLICE_PATH_MAP` is set, paths in MCP responses are remapped
//! (forward: stored prefix -> current-machine prefix) and incoming paths
//! are reverse-remapped (current-machine prefix -> stored prefix) so that
//! indexes built on one machine can be found from another.
//!
//! Format: `old=new,old2=new2` — comma-separated pairs, last `=` is the
//! separator (earlier `=` chars belong to the "old" side).

/// The environment variable name for the path map.
pub const ENV_VAR: &str = "SYMSLICE_PATH_MAP";

/// A single path-prefix mapping pair: (original, replacement).
pub type PathPair = (String, String);

/// Parse the `SYMSLICE_PATH_MAP` environment variable into a list of
/// `(original, replacement)` pairs.
///
/// Format: `old=new,old2=new2`
///  - Comma-separated entries.
///  - The **last** `=` in each entry is the separator; earlier `=` chars
///    belong to the original path (e.g. `/home/user/a=b=/new` parses as
///    `("/home/user/a=b", "/new")`).
///  - Whitespace around both sides is stripped.
///  - Entries without `=`, or with an empty original or replacement after
///    stripping, are skipped with a warning.
pub fn parse_path_map(env_value: &str) -> Vec<PathPair> {
    let trimmed = env_value.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let mut pairs = Vec::new();
    for entry in trimmed.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        // Last `=` is the separator.
        if let Some(pos) = entry.rfind('=') {
            let orig = entry[..pos].trim();
            let new = entry[pos + 1..].trim();
            if orig.is_empty() {
                tracing::warn!(
                    entry = %entry,
                    "SYMSLICE_PATH_MAP: skipping entry with empty original"
                );
                continue;
            }
            if new.is_empty() {
                tracing::warn!(
                    entry = %entry,
                    "SYMSLICE_PATH_MAP: skipping entry with empty replacement"
                );
                continue;
            }
            pairs.push((orig.to_owned(), new.to_owned()));
        } else {
            tracing::warn!(
                entry = %entry,
                "SYMSLICE_PATH_MAP: skipping malformed entry (no '=')"
            );
        }
    }
    pairs
}

/// Parse the path map from the environment variable directly.
pub fn parse_path_map_from_env() -> Vec<PathPair> {
    match std::env::var(ENV_VAR) {
        Ok(val) => parse_path_map(&val),
        Err(_) => Vec::new(),
    }
}

/// Normalize path separators: replace all backslashes with forward slashes.
fn normalize_sep(path: &str) -> String {
    path.replace('\\', "/")
}

/// Apply path prefix remapping.
///
/// In **forward** mode (`reverse = false`): the first element of each pair is
/// the prefix to match, replaced by the second element.
///
/// In **reverse** mode (`reverse = true`): the second element is the prefix to
/// match, replaced by the first element.
///
/// Matching:
///  - Both the input path and the prefix are normalized (backslash -> forward
///    slash) before comparison.
///  - A prefix matches only at a directory boundary: `/home/user` matches
///    `/home/user/file.py` and `/home/user` itself, but **not**
///    `/home/username/file.py`.
///  - If multiple pairs match, the **longest** prefix wins. On ties the
///    **first** pair in the list wins.
///  - The result is always returned with forward slashes.
pub fn remap(path: &str, pairs: &[PathPair], reverse: bool) -> String {
    if pairs.is_empty() {
        return path.to_owned();
    }

    let norm_path = normalize_sep(path);

    let mut best_match: Option<(usize, &str)> = None; // (prefix_len, replacement)

    for (orig, new) in pairs {
        let (from, to) = if reverse {
            (normalize_sep(new), orig.as_str())
        } else {
            (normalize_sep(orig), new.as_str())
        };

        let from_trimmed = from.trim_end_matches('/');

        if prefix_matches_at_boundary(&norm_path, from_trimmed) {
            let plen = from_trimmed.len();
            match best_match {
                Some((best_len, _)) if plen <= best_len => {
                    // Current best is longer or same length (first wins on tie) — keep it.
                }
                _ => {
                    best_match = Some((plen, to));
                }
            }
        }
    }

    if let Some((prefix_len, replacement)) = best_match {
        let rest = &norm_path[prefix_len..];
        let to_norm = normalize_sep(replacement);
        let to_trimmed = to_norm.trim_end_matches('/');
        format!("{to_trimmed}{rest}")
    } else {
        norm_path
    }
}

/// Check whether `path` starts with `prefix` at a directory boundary.
///
/// Returns true when:
///  - `path == prefix` (exact match), or
///  - `path` starts with `prefix` and the character immediately after is `/`.
fn prefix_matches_at_boundary(path: &str, prefix: &str) -> bool {
    if path == prefix {
        return true;
    }
    if !path.starts_with(prefix) {
        return false;
    }
    // The character immediately after the prefix must be '/'.
    path.as_bytes().get(prefix.len()) == Some(&b'/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty() {
        assert!(parse_path_map("").is_empty());
    }

    #[test]
    fn parse_whitespace() {
        assert!(parse_path_map("   ").is_empty());
    }

    #[test]
    fn parse_single() {
        assert_eq!(
            parse_path_map("/home/user=/mnt/user"),
            vec![("/home/user".to_owned(), "/mnt/user".to_owned())]
        );
    }

    #[test]
    fn parse_multiple() {
        assert_eq!(
            parse_path_map("/a=/b,/c=/d"),
            vec![
                ("/a".to_owned(), "/b".to_owned()),
                ("/c".to_owned(), "/d".to_owned()),
            ]
        );
    }

    #[test]
    fn parse_equals_in_path() {
        assert_eq!(
            parse_path_map("/home/user/a=b=/new/path"),
            vec![("/home/user/a=b".to_owned(), "/new/path".to_owned())]
        );
    }

    #[test]
    fn remap_boundary() {
        let pairs = vec![("/home/user".to_owned(), "/mnt/user".to_owned())];
        // Must NOT match /home/username
        assert_eq!(
            remap("/home/username/file.py", &pairs, false),
            "/home/username/file.py"
        );
        // Must match /home/user/file.py
        assert_eq!(
            remap("/home/user/file.py", &pairs, false),
            "/mnt/user/file.py"
        );
    }
}
