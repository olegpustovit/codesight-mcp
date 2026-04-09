use std::{collections::HashSet, fs, path::Path};

use crate::{analysis::evidence::EvidenceNote, storage::AnalysisSymbol};

pub(crate) const ENTRYPOINT_BASENAMES: &[&str] = &[
    "main.py",
    "__main__.py",
    "__init__.py",
    "app.py",
    "application.py",
    "wsgi.py",
    "asgi.py",
    "manage.py",
    "setup.py",
    "conftest.py",
    "main.js",
    "index.js",
    "app.js",
    "server.js",
    "main.ts",
    "index.ts",
    "app.ts",
    "server.ts",
    "main.go",
    "main.rs",
    "main.c",
    "main.cpp",
];

pub(crate) fn is_test_file(file_path: &str) -> bool {
    let basename = Path::new(file_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    basename.starts_with("test_")
        || basename.starts_with("tests")
        || file_path.contains("/tests/")
        || file_path.contains("/test/")
}

pub(crate) fn entrypoint_root_notes(
    file_path: &str,
    entry_patterns: &[String],
) -> Vec<EvidenceNote> {
    let basename = Path::new(file_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let mut notes = Vec::new();

    if ENTRYPOINT_BASENAMES.contains(&basename) || basename.starts_with("main.") {
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
    let mut files = HashSet::new();

    if let Some(root_path) = root_path {
        for file in all_files {
            let absolute_path = root_path.join(file);
            if let Ok(content) = fs::read_to_string(&absolute_path)
                && content.contains("__name__")
                && content.contains("__main__")
            {
                files.insert(file.clone());
            }
        }
    }

    for symbol in all_symbols {
        if symbol.source_text.contains("__name__") && symbol.source_text.contains("__main__") {
            files.insert(symbol.file_path.clone());
        }
    }

    files
}

fn glob_match(pattern: &str, path: &str) -> bool {
    let re_str = pattern.replace('.', r"\.").replace('*', "[^/]*");
    regex::Regex::new(&format!("(?:^|/){re_str}$"))
        .map(|re| re.is_match(path))
        .unwrap_or(false)
}
