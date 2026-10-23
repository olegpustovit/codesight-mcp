//! Dead-code entrypoint detection — thin dispatcher over the language
//! trait. Every per-language heuristic (Python `__main__` guards, Rust
//! Cargo targets, Rust foreign-trait impls, future Next.js pages, …)
//! lives on `Language::additional_live_roots`; this module just unions
//! the results.

use std::path::Path;

use symslice_language_support::builtin_registry;
use symslice_language_support::traits::LiveRoot;

use crate::evidence::EvidenceNote;

pub(crate) fn entrypoint_root_notes(
    file_path: &str,
    entry_patterns: &[String],
) -> Vec<EvidenceNote> {
    let basename = Path::new(file_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let mut notes = Vec::new();

    if symslice_language_support::is_entrypoint_basename(basename) {
        notes.push(EvidenceNote::heuristic(
            "entrypoint_basename",
            format!("basename '{basename}' is treated as an entry root"),
        ));
    }

    for pattern in entry_patterns {
        if glob_matches(pattern, file_path) {
            notes.push(EvidenceNote::heuristic(
                "entrypoint_pattern",
                format!("path matched configured entry_point_patterns value '{pattern}'"),
            ));
        }
    }

    notes
}

/// Union of every registered language's `additional_live_roots`. Callers
/// never branch on the language that produced a root; each `LiveRoot`
/// carries its own reason / message / confidence.
pub(crate) fn detect_language_live_roots(
    root_path: Option<&Path>,
    all_files: &[String],
) -> Vec<LiveRoot> {
    builtin_registry()
        .all()
        .flat_map(|lang| lang.additional_live_roots(root_path, all_files))
        .collect()
}

/// Glob match anchored at a path segment boundary. Matches either the full
/// path or a suffix starting after any `/`. Patterns support shell
/// wildcards (`*`, `?`, `[abc]`, `{a,b}`).
fn glob_matches(pattern: &str, path: &str) -> bool {
    let Ok(glob) = globset::Glob::new(pattern) else {
        return false;
    };
    let matcher = glob.compile_matcher();
    if matcher.is_match(path) {
        return true;
    }
    path.match_indices('/')
        .any(|(i, _)| matcher.is_match(&path[i + 1..]))
}

#[cfg(test)]
mod language_live_roots_tests {
    use super::detect_language_live_roots;
    use std::fs;

    #[test]
    fn collects_rust_cargo_targets_without_special_casing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("tests")).unwrap();
        fs::write(root.join("tests/it.rs"), "").unwrap();

        let all_files: Vec<String> = vec!["tests/it.rs".into()];
        let roots = detect_language_live_roots(Some(root), &all_files);
        assert!(
            roots
                .iter()
                .any(|r| r.path == "tests/it.rs" && r.reason == "cargo_target")
        );
    }
}
