//! Python module-stem resolver.
//!
//! Python's dotted-module imports (`foo.bar`) resolve to a `foo/bar.py`
//! file or a `foo/bar/__init__.py` package module. The helpers below live
//! on the Python language so the generic orchestration stays language-
//! agnostic.

use std::collections::{HashMap, HashSet};

/// Resolve dotted-module `import_targets` to project-relative file paths.
/// Each target is matched against every `.py` file's stem (or the package
/// directory name for `__init__.py`).
pub fn resolve_imports_to_files(
    import_targets: &[String],
    known_files: &[String],
    source_file: &str,
) -> Vec<String> {
    let mut resolved = Vec::new();
    let mut seen = HashSet::new();

    let mut stem_to_file: HashMap<String, Vec<String>> = HashMap::new();
    for f in known_files {
        if let Some(stem) = python_file_stem(f) {
            stem_to_file.entry(stem).or_default().push(f.clone());
        }
    }

    for target in import_targets {
        if let Some(matches) = stem_to_file.get(target.as_str()) {
            for m in matches {
                if m != source_file && seen.insert(m.clone()) {
                    resolved.push(m.clone());
                }
            }
        }
    }

    resolved
}

/// Strip `.py` and collapse `__init__.py` to its parent directory name.
fn python_file_stem(path: &str) -> Option<String> {
    let file_name = path.rsplit('/').next()?;
    let stem = file_name.strip_suffix(".py").unwrap_or(file_name);
    if stem == "__init__" {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() >= 2 {
            return Some(parts[parts.len() - 2].to_string());
        }
    }
    Some(stem.to_string())
}
