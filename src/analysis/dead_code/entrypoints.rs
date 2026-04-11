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
    // Rust crate roots (not just binaries)
    "lib.rs",
    // Cargo build scripts — always live via cargo, never reachable via
    // normal import graphs.
    "build.rs",
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

/// Collect Cargo target files that are always live roots in practice:
///
/// - Auto-discovered targets: any `.rs` file under `tests/`, `examples/`, or
///   `benches/` next to a `Cargo.toml`.
/// - Explicit `[[test]]` / `[[example]]` / `[[bench]]` entries with a `path`
///   field.
///
/// Paths are returned relative to `root_path` (the indexed repo root), matching
/// the shape of `all_files`. When `root_path` is `None` this is a no-op — we
/// can't read `Cargo.toml` contents without a disk anchor.
pub(crate) fn detect_cargo_target_files(
    root_path: Option<&Path>,
    all_files: &[String],
) -> HashSet<String> {
    let mut targets = HashSet::new();
    let Some(root) = root_path else {
        return targets;
    };

    let file_set: HashSet<&str> = all_files.iter().map(String::as_str).collect();
    let cargo_dirs: Vec<String> = all_files
        .iter()
        .filter(|f| {
            Path::new(f)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n == "Cargo.toml")
                .unwrap_or(false)
        })
        .map(|f| {
            Path::new(f)
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .to_string()
        })
        .collect();

    let join_rel = |dir: &str, sub: &str| -> String {
        if dir.is_empty() {
            sub.to_string()
        } else {
            format!("{dir}/{sub}")
        }
    };

    for cargo_dir in &cargo_dirs {
        for subdir in ["tests", "examples", "benches"] {
            let prefix = join_rel(cargo_dir, subdir);
            for file in all_files {
                if !file.ends_with(".rs") {
                    continue;
                }
                let matches_subdir = file == &prefix
                    || file.starts_with(&format!("{prefix}/"))
                    || file.starts_with(&format!("{prefix}\\"));
                if matches_subdir {
                    targets.insert(file.clone());
                }
            }
        }

        let cargo_toml_rel = join_rel(cargo_dir, "Cargo.toml");
        let cargo_toml_abs = root.join(&cargo_toml_rel);
        let Ok(contents) = fs::read_to_string(&cargo_toml_abs) else {
            continue;
        };
        let Ok(parsed) = contents.parse::<toml::Value>() else {
            continue;
        };
        for key in ["test", "example", "bench", "bin"] {
            let Some(entries) = parsed.get(key).and_then(|v| v.as_array()) else {
                continue;
            };
            for entry in entries {
                let Some(path) = entry.get("path").and_then(|p| p.as_str()) else {
                    continue;
                };
                let rel = join_rel(cargo_dir, path);
                if file_set.contains(rel.as_str()) {
                    targets.insert(rel);
                }
            }
        }
    }

    targets
}
