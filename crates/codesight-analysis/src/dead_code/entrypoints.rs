//! Dead-code entrypoint detection — thin wrapper over the per-language
//! liveness axis in `codesight-language-support`.
//!
//! LANGUAGES.md §4.4: entrypoint basenames, test-file detection, fixture
//! detection, Rust `FOREIGN_TRAIT_BASES`, Python `__main__` guards, and
//! Cargo target discovery all live in per-language folders now. This module
//! keeps the public shape (`entrypoint_root_notes`,
//! `detect_main_guard_files`, `detect_cargo_target_files`,
//! `detect_foreign_trait_impl_files`, `is_test_file`, `is_test_fixture`)
//! so callers don't change in the same patch.

use std::collections::HashSet;
use std::path::Path;

use codesight_language_support::builtin_registry;
use codesight_language_support::manifest::{PYTHON, RUST};
use codesight_storage::storage::AnalysisSymbol;

use crate::evidence::EvidenceNote;

pub(crate) use codesight_language_support::is_test_file;
pub(crate) use codesight_language_support::is_test_fixture;

pub(crate) fn entrypoint_root_notes(
    file_path: &str,
    entry_patterns: &[String],
) -> Vec<EvidenceNote> {
    let basename = Path::new(file_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let mut notes = Vec::new();

    if codesight_language_support::is_entrypoint_basename(basename) || basename.starts_with("main.")
    {
        notes.push(EvidenceNote::heuristic(
            "entrypoint_basename",
            format!("basename '{basename}' is treated as an entry root"),
        ));
    }

    for pattern in entry_patterns {
        if glob_match(pattern, file_path) {
            notes.push(EvidenceNote::heuristic(
                "entrypoint_pattern",
                format!("path matched configured entry_point_patterns value '{pattern}'"),
            ));
        }
    }

    notes
}

pub(crate) fn detect_main_guard_files(
    root_path: Option<&Path>,
    all_files: &[String],
    all_symbols: &[AnalysisSymbol],
) -> HashSet<String> {
    let mut files: HashSet<String> = builtin_registry()
        .get(PYTHON)
        .map(|lang| {
            lang.additional_live_root_files(root_path, all_files)
                .into_iter()
                .collect()
        })
        .unwrap_or_default();

    for symbol in all_symbols {
        if symbol.source_text.contains("__name__") && symbol.source_text.contains("__main__") {
            files.insert(symbol.file_path.clone());
        }
    }

    files
}

pub(crate) fn detect_cargo_target_files(
    root_path: Option<&Path>,
    all_files: &[String],
) -> HashSet<String> {
    rust_live_roots(root_path, all_files, Filter::Cargo)
}

pub(crate) fn detect_foreign_trait_impl_files(
    root_path: Option<&Path>,
    all_files: &[String],
) -> HashSet<String> {
    rust_live_roots(root_path, all_files, Filter::ForeignTraitImpl)
}

enum Filter {
    Cargo,
    ForeignTraitImpl,
}

fn rust_live_roots(
    root_path: Option<&Path>,
    all_files: &[String],
    which: Filter,
) -> HashSet<String> {
    // Rust's `additional_live_root_files` returns the union of foreign-trait
    // impl files and Cargo targets. We re-filter here so callers can still
    // ask for each subset independently (the evidence-note text differs).
    let Some(rust) = builtin_registry().get(RUST) else {
        return HashSet::new();
    };
    let all = rust.additional_live_root_files(root_path, all_files);
    match which {
        Filter::Cargo => all
            .into_iter()
            .filter(|f| is_cargo_target_path(f))
            .collect(),
        Filter::ForeignTraitImpl => all
            .into_iter()
            .filter(|f| !is_cargo_target_path(f))
            .collect(),
    }
}

fn is_cargo_target_path(path: &str) -> bool {
    // Cargo targets sit under tests/, examples/, benches/, or src/bin/. A
    // foreign-trait impl file lives elsewhere.
    path.starts_with("tests/")
        || path.contains("/tests/")
        || path.starts_with("examples/")
        || path.contains("/examples/")
        || path.starts_with("benches/")
        || path.contains("/benches/")
        || path.starts_with("src/bin/")
        || path.contains("/src/bin/")
}

fn glob_match(pattern: &str, path: &str) -> bool {
    let re_str = pattern.replace('.', r"\.").replace('*', "[^/]*");
    regex::Regex::new(&format!("(?:^|/){re_str}$"))
        .map(|re| re.is_match(path))
        .unwrap_or(false)
}

#[cfg(test)]
mod cargo_target_tests {
    use super::detect_cargo_target_files;
    use std::fs;

    #[test]
    fn auto_discovers_tests_examples_benches_without_indexed_cargo_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("tests")).unwrap();
        fs::write(root.join("tests/it.rs"), "").unwrap();
        fs::create_dir_all(root.join("examples")).unwrap();
        fs::write(root.join("examples/demo.rs"), "").unwrap();
        fs::create_dir_all(root.join("benches")).unwrap();
        fs::write(root.join("benches/bench.rs"), "").unwrap();
        fs::create_dir_all(root.join("crates/inner/tests")).unwrap();
        fs::write(
            root.join("crates/inner/Cargo.toml"),
            "[package]\nname = \"inner\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(root.join("crates/inner/tests/it.rs"), "").unwrap();

        let all_files: Vec<String> = vec![
            "tests/it.rs".into(),
            "examples/demo.rs".into(),
            "benches/bench.rs".into(),
            "crates/inner/tests/it.rs".into(),
        ];

        let targets = detect_cargo_target_files(Some(root), &all_files);
        assert!(targets.contains("tests/it.rs"), "{:?}", targets);
        assert!(targets.contains("examples/demo.rs"), "{:?}", targets);
        assert!(targets.contains("benches/bench.rs"), "{:?}", targets);
        assert!(
            targets.contains("crates/inner/tests/it.rs"),
            "{:?}",
            targets
        );
    }
}
