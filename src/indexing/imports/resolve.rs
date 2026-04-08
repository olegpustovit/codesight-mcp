//! Map import specifiers to indexed file paths and load tsconfig path aliases.

use std::collections::{HashMap, HashSet};

use super::ImportEdge;

/// Resolve an import specifier to a file path within the known source files.
///
/// `specifier`: the raw import string (e.g. `"./utils"`, `"react"`, `"dim_client"`)
/// `from_file`: the file that contains the import
/// `source_files`: set of known file paths in the project
/// `alias_map`: optional path alias mappings (e.g. `{"@/*": ["src/*"]}`)
///
/// Returns `Some(resolved_path)` if found, `None` otherwise.
pub fn resolve_specifier(
    specifier: &str,
    from_file: &str,
    source_files: &HashSet<String>,
    alias_map: Option<&HashMap<String, Vec<String>>>,
) -> Option<String> {
    if specifier.starts_with("source:") {
        return None;
    }

    if let Some(aliases) = alias_map
        && let Some(resolved) = resolve_with_aliases(specifier, source_files, aliases)
    {
        return Some(resolved);
    }

    if specifier.starts_with("./") || specifier.starts_with("../") {
        let dir = parent_dir(from_file);
        let resolved = normalize_path(&format!("{dir}/{specifier}"));
        return resolve_js_path(&resolved, source_files);
    }

    if specifier.starts_with('.') && !specifier.starts_with("./") && !specifier.starts_with("../") {
        return None;
    }

    if source_files.contains(specifier) {
        return Some(specifier.to_string());
    }

    let spec_lower = specifier.to_ascii_lowercase();
    if !specifier.contains('.') && !specifier.contains('/') {
        for file in source_files {
            let file_name = file.rsplit('/').next().unwrap_or(file);
            let stem = file_name
                .rsplit_once('.')
                .map(|(s, _)| s)
                .unwrap_or(file_name);
            if stem.to_ascii_lowercase() == spec_lower {
                return Some(file.clone());
            }
        }
    }

    None
}

fn resolve_with_aliases(
    specifier: &str,
    source_files: &HashSet<String>,
    alias_map: &HashMap<String, Vec<String>>,
) -> Option<String> {
    let mut aliases: Vec<(&String, &Vec<String>)> = alias_map.iter().collect();
    aliases.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

    for (pattern, replacements) in aliases {
        let prefix = pattern.trim_end_matches('*');
        if let Some(rest) = specifier.strip_prefix(prefix) {
            for replacement in replacements {
                let repl_prefix = replacement.trim_end_matches('*');
                let resolved_base = format!("{repl_prefix}{rest}");
                let resolved_base = resolved_base.trim_start_matches('/');
                if let Some(resolved) = resolve_js_path(resolved_base, source_files) {
                    return Some(resolved);
                }
            }
        }
    }
    None
}

fn resolve_js_path(base: &str, source_files: &HashSet<String>) -> Option<String> {
    if source_files.contains(base) {
        return Some(base.to_string());
    }

    let extensions = [".ts", ".tsx", ".js", ".jsx", ".vue", ".mjs", ".cjs"];
    for ext in &extensions {
        let with_ext = format!("{base}{ext}");
        if source_files.contains(&with_ext) {
            return Some(with_ext);
        }
    }

    let index_extensions = ["index.ts", "index.tsx", "index.js", "index.jsx"];
    for idx in &index_extensions {
        let with_index = format!("{base}/{idx}");
        if source_files.contains(&with_index) {
            return Some(with_index);
        }
    }

    if let Some(without) = base.strip_suffix(".js") {
        let ts_extensions = [".ts", ".tsx"];
        for ext in &ts_extensions {
            let mapped = format!("{without}{ext}");
            if source_files.contains(&mapped) {
                return Some(mapped);
            }
        }
    }

    None
}

fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(idx) => path[..idx].to_string(),
        None => ".".to_string(),
    }
}

fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// Load path aliases from tsconfig.json (supports JSONC with comments).
pub fn load_tsconfig_aliases(source_root: &str) -> HashMap<String, Vec<String>> {
    if source_root.is_empty() {
        return HashMap::new();
    }

    let tsconfig_path = format!("{source_root}/tsconfig.json");
    let content = match std::fs::read_to_string(&tsconfig_path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };

    parse_tsconfig_paths(&content)
}

/// Parse tsconfig JSON(C) content and extract compilerOptions.paths.
pub fn parse_tsconfig_paths(content: &str) -> HashMap<String, Vec<String>> {
    let clean = strip_jsonc_comments(content);
    let parsed: serde_json::Value = match serde_json::from_str(&clean) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };

    let paths = match parsed.get("compilerOptions").and_then(|co| co.get("paths")) {
        Some(p) => p,
        None => return HashMap::new(),
    };

    let mut result = HashMap::new();
    if let Some(obj) = paths.as_object() {
        for (key, value) in obj {
            if let Some(arr) = value.as_array() {
                let normalized: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| {
                        if s == "./*" {
                            "/*".to_string()
                        } else {
                            s.strip_prefix("./").unwrap_or(s).to_string()
                        }
                    })
                    .collect();
                result.insert(key.clone(), normalized);
            }
        }
    }
    result
}

fn strip_jsonc_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut in_string = false;

    while i < len {
        if in_string {
            result.push(chars[i]);
            if chars[i] == '\\' && i + 1 < len {
                i += 1;
                result.push(chars[i]);
            } else if chars[i] == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if chars[i] == '"' {
            in_string = true;
            result.push(chars[i]);
            i += 1;
            continue;
        }

        if chars[i] == '/' && i + 1 < len {
            if chars[i + 1] == '/' {
                i += 2;
                while i < len && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            } else if chars[i + 1] == '*' {
                i += 2;
                while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
                continue;
            }
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Resolve import targets to known file paths (Python stem → `foo.py`, package `__init__.py`, etc.).
pub fn resolve_imports_to_files(
    import_targets: &[String],
    known_files: &[String],
    source_file: &str,
) -> Vec<String> {
    let mut resolved = Vec::new();
    let mut seen = HashSet::new();

    let mut stem_to_file: HashMap<String, Vec<String>> = HashMap::new();
    for f in known_files {
        if let Some(stem) = file_stem(f) {
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

fn file_stem(path: &str) -> Option<String> {
    let file_name = path.rsplit('/').next()?;
    let stem = file_name
        .strip_suffix(".py")
        .or_else(|| file_name.strip_suffix(".js"))
        .or_else(|| file_name.strip_suffix(".ts"))
        .or_else(|| file_name.strip_suffix(".go"))
        .or_else(|| file_name.strip_suffix(".rs"))
        .or_else(|| file_name.strip_suffix(".rb"))
        .unwrap_or(file_name);
    if stem == "__init__" {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() >= 2 {
            return Some(parts[parts.len() - 2].to_string());
        }
    }
    Some(stem.to_string())
}

/// Resolve a single [`ImportEdge`] to a project file, including Python dotted-module fallback.
pub fn resolve_import_edge_for_graph(
    edge: &ImportEdge,
    from_file: &str,
    lang: &str,
    source_files: &HashSet<String>,
    known_files_vec: &[String],
    alias_map: Option<&HashMap<String, Vec<String>>>,
) -> Option<String> {
    if let Some(p) = resolve_specifier(&edge.specifier, from_file, source_files, alias_map) {
        return Some(p);
    }
    if lang.eq_ignore_ascii_case("python")
        && !edge.specifier.starts_with("./")
        && !edge.specifier.starts_with("../")
        && edge.specifier.contains('.')
    {
        let top = edge.specifier.split('.').next()?.to_string();
        if top.is_empty() {
            return None;
        }
        let resolved = resolve_imports_to_files(&[top], known_files_vec, from_file);
        return resolved.into_iter().next();
    }
    None
}
