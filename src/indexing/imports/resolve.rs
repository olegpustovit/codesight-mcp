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
    // Rust has `::`-path imports that overlap with external crate names.
    // Route every Rust edge through the Rust-aware resolver, which only
    // accepts local candidates when the specifier starts with
    // `crate`/`self`/`super`. Bare names like `regex` / `serde` / `tokio`
    // are correctly treated as external (return None) rather than falling
    // through to the generic stem-match fallback — which would false-match
    // any local file sharing the name.
    if lang.eq_ignore_ascii_case("rust") || lang.eq_ignore_ascii_case("rs") {
        return resolve_rust_specifier(&edge.specifier, from_file, known_files_vec);
    }
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

/// Map a Rust `::`-path specifier to a local file, stripping `crate::`,
/// `self::`, `super::` prefixes and attempting both `stem.rs` and
/// `stem/mod.rs` candidates against the known-files list.
fn resolve_rust_specifier(
    specifier: &str,
    from_file: &str,
    known_files: &[String],
) -> Option<String> {
    let mut segments: Vec<&str> = specifier.split("::").filter(|s| !s.is_empty()).collect();
    let first = segments.first().copied()?;
    let is_anchored = matches!(first, "crate" | "self" | "super");
    if !is_anchored {
        // Bare identifier — could be an external crate OR a submodule that
        // the current file declared via `mod <name>;`. We can't inspect the
        // from-file's AST here, so use a safe file-system heuristic: only
        // accept a local match if a sibling file or directory with that
        // stem exists in the **from-file's own directory** (not anywhere
        // in the project — that was the old stem-match false-positive).
        return resolve_rust_bare_sibling(&segments, from_file, known_files);
    }

    // Walk off leading anchor keywords, accumulating how many `super::`s we
    // saw so we can step up the module tree the corresponding number of
    // times. `self::` and `crate::` each consume one segment without
    // changing depth; `super::` consumes one segment and also pops one
    // module level off the from-file path.
    let mut super_steps = 0usize;
    let mut has_crate = false;
    let mut has_self = false;
    while let Some(seg) = segments.first().copied() {
        match seg {
            "crate" => {
                has_crate = true;
                segments.remove(0);
            }
            "self" => {
                has_self = true;
                segments.remove(0);
            }
            "super" => {
                super_steps += 1;
                segments.remove(0);
            }
            _ => break,
        }
    }
    if segments.is_empty() {
        return None;
    }

    // Compute the module directory (inside the crate source tree) from
    // which to resolve the remaining path.
    let base_dir = if has_crate {
        crate_root_of(from_file)?
    } else if has_self {
        // `self::X` from `foo.rs` or `foo/mod.rs` looks in the module's own
        // submodule directory. For `foo/mod.rs` that's `foo/`. For
        // `foo/bar.rs` (where bar is already a submodule) it's `foo/bar/`.
        self_module_dir(from_file)?
    } else if super_steps > 0 {
        // `super::X` from `foo/bar.rs` looks at the parent module `foo`,
        // whose siblings live in `foo/`. From `foo/mod.rs` it looks at
        // `foo`'s parent module instead.
        let mut dir = parent_module_dir(from_file)?;
        for _ in 1..super_steps {
            dir = parent_dir_string(&dir)?;
        }
        dir
    } else {
        return None;
    };

    // Try longest → shortest match so `a::b::c` prefers `a/b/c.rs` over
    // `a.rs` when both exist. Each candidate is anchored at `base_dir`.
    for take in (1..=segments.len()).rev() {
        let sub: Vec<&str> = segments[..take].to_vec();
        let joined = sub.join("/");
        let candidates = if base_dir.is_empty() {
            [format!("{joined}.rs"), format!("{joined}/mod.rs")]
        } else {
            [
                format!("{base_dir}/{joined}.rs"),
                format!("{base_dir}/{joined}/mod.rs"),
            ]
        };
        for cand in &candidates {
            if known_files.iter().any(|f| f == cand) && cand.as_str() != from_file {
                return Some(cand.clone());
            }
        }
    }
    None
}

/// Bare-identifier resolver for `use foo::X` style imports where `foo` is a
/// submodule declared in the current file (`mod foo;`). We don't have the
/// declaring file's AST here, so we check the from-file's own directory for
/// a sibling `foo.rs` or `foo/mod.rs` — if present, treat it as local.
/// Returns None for unambiguously external crate names.
fn resolve_rust_bare_sibling(
    segments: &[&str],
    from_file: &str,
    known_files: &[String],
) -> Option<String> {
    let first = *segments.first()?;
    let dir = parent_dir_string(from_file)?;
    // Candidate files the first segment could refer to, anchored at the
    // from-file's directory only (not the whole project — that was the old
    // false-positive trap).
    let candidates = [format!("{dir}/{first}.rs"), format!("{dir}/{first}/mod.rs")];
    let first_hit = candidates
        .iter()
        .find(|c| known_files.iter().any(|f| f == *c))?;
    if segments.len() == 1 {
        return Some(first_hit.clone());
    }
    // Extend the match with any remaining segments as nested path lookups
    // under the newly-found module directory.
    let submodule_dir = if first_hit.ends_with("/mod.rs") {
        first_hit.trim_end_matches("/mod.rs").to_string()
    } else {
        first_hit.trim_end_matches(".rs").to_string()
    };
    let rest = &segments[1..];
    for take in (1..=rest.len()).rev() {
        let joined = rest[..take].join("/");
        for cand in &[
            format!("{submodule_dir}/{joined}.rs"),
            format!("{submodule_dir}/{joined}/mod.rs"),
        ] {
            if known_files.iter().any(|f| f == cand) && cand.as_str() != from_file {
                return Some(cand.clone());
            }
        }
    }
    // Couldn't find a deeper match — return the first-segment hit so that
    // the edge at least points at the right sibling module.
    Some(first_hit.clone())
}

/// Return the "crate root" — the path prefix up to and including the last
/// `src/` component in `from_file`. For `crates/x/src/a/b.rs` this returns
/// `crates/x/src`. If no `src/` is found, returns the empty string so the
/// resolver falls back to repo-root-relative lookups.
fn crate_root_of(from_file: &str) -> Option<String> {
    let parts: Vec<&str> = from_file.split('/').collect();
    let src_idx = parts.iter().rposition(|p| *p == "src")?;
    let prefix: Vec<&str> = parts[..=src_idx].to_vec();
    Some(prefix.join("/"))
}

/// Return the directory where siblings of `from_file`'s module live.
/// - `foo/bar.rs`       → `foo`
/// - `foo/mod.rs`       → parent of `foo` (one level up)
/// - `src/foo.rs`       → `src`
fn parent_module_dir(from_file: &str) -> Option<String> {
    if let Some(parent) = from_file.strip_suffix("/mod.rs") {
        return parent_dir_string(parent);
    }
    parent_dir_string(from_file)
}

/// Return the directory where submodules of `from_file`'s module live.
/// - `src/lib.rs` / `src/main.rs` → `src` (crate roots: submodules live
///   alongside the root file, not under a `lib/`/`main/` directory)
/// - `foo/mod.rs` → `foo`
/// - `foo/bar.rs` → `foo/bar` (2018+ style: submodules of `bar` live there)
fn self_module_dir(from_file: &str) -> Option<String> {
    if let Some(parent) = from_file.strip_suffix("/mod.rs") {
        return Some(parent.to_string());
    }
    // Crate roots: `lib.rs` / `main.rs` live at `.../src/lib.rs` and their
    // submodules live in `.../src/`, never in `.../src/lib/`.
    if from_file.ends_with("/lib.rs") || from_file.ends_with("/main.rs") {
        return parent_dir_string(from_file);
    }
    // Bare `lib.rs` / `main.rs` at the project root (no enclosing dir).
    if from_file == "lib.rs" || from_file == "main.rs" {
        return Some(String::new());
    }
    let stripped = from_file.strip_suffix(".rs")?;
    Some(stripped.to_string())
}

fn parent_dir_string(path: &str) -> Option<String> {
    let idx = path.rfind('/')?;
    Some(path[..idx].to_string())
}

#[cfg(test)]
mod rust_resolve_tests {
    use super::*;

    fn files(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn crate_prefix_resolves_under_src_root() {
        let known = files(&["src/lib.rs", "src/models.rs"]);
        let got = resolve_rust_specifier("crate::models", "src/lib.rs", &known);
        assert_eq!(got.as_deref(), Some("src/models.rs"));
    }

    #[test]
    fn nested_crate_path_prefers_longest_match() {
        let known = files(&["src/lib.rs", "src/a/b.rs"]);
        let got = resolve_rust_specifier("crate::a::b", "src/lib.rs", &known);
        assert_eq!(got.as_deref(), Some("src/a/b.rs"));
    }

    #[test]
    fn crate_mod_rs_candidate_matches() {
        let known = files(&["src/lib.rs", "src/a/mod.rs"]);
        let got = resolve_rust_specifier("crate::a", "src/lib.rs", &known);
        assert_eq!(got.as_deref(), Some("src/a/mod.rs"));
    }

    #[test]
    fn external_crate_returns_none() {
        let known = files(&["src/lib.rs", "src/models.rs"]);
        assert!(resolve_rust_specifier("serde", "src/lib.rs", &known).is_none());
        assert!(resolve_rust_specifier("std::collections", "src/lib.rs", &known).is_none());
    }

    #[test]
    fn bare_crate_name_does_not_false_match_local_file() {
        let known = files(&["src/framework/laravel/regex.rs", "src/lib.rs"]);
        assert!(resolve_rust_specifier("regex", "src/lib.rs", &known).is_none());
    }

    #[test]
    fn super_resolves_to_sibling_in_same_dir() {
        // Regression: `super::render` from `laravel/provider.rs` must NOT
        // match `nuxt/render.rs` — it must anchor at the current file's
        // module dir (`laravel/`).
        let known = files(&[
            "src/framework/laravel/provider.rs",
            "src/framework/laravel/render.rs",
            "src/framework/nuxt/render.rs",
        ]);
        let got =
            resolve_rust_specifier("super::render", "src/framework/laravel/provider.rs", &known);
        assert_eq!(got.as_deref(), Some("src/framework/laravel/render.rs"));
    }

    #[test]
    fn super_super_walks_two_levels_up() {
        let known = files(&[
            "src/framework/laravel/provider.rs",
            "src/framework/shared.rs",
        ]);
        let got = resolve_rust_specifier(
            "super::super::shared",
            "src/framework/laravel/provider.rs",
            &known,
        );
        assert_eq!(got.as_deref(), Some("src/framework/shared.rs"));
    }

    #[test]
    fn super_from_mod_rs_steps_out_of_module() {
        // `super::sibling` from `foo/mod.rs` should resolve in `foo/`'s
        // parent directory (one level above `foo/`).
        let known = files(&["src/framework/laravel/mod.rs", "src/framework/nuxt.rs"]);
        let got = resolve_rust_specifier("super::nuxt", "src/framework/laravel/mod.rs", &known);
        assert_eq!(got.as_deref(), Some("src/framework/nuxt.rs"));
    }

    #[test]
    fn self_from_mod_rs_stays_in_module() {
        let known = files(&["src/a/mod.rs", "src/a/inner.rs"]);
        let got = resolve_rust_specifier("self::inner", "src/a/mod.rs", &known);
        assert_eq!(got.as_deref(), Some("src/a/inner.rs"));
    }

    #[test]
    fn self_from_lib_rs_resolves_to_crate_root_sibling() {
        // `mod service;` in `src/lib.rs` must map to `src/service.rs`
        // (or `src/service/mod.rs`), NOT to `src/lib/service.rs`.
        // Regression for the 94%-dead-code bug where crate roots were
        // treated like ordinary 2018-style module files.
        let known = files(&["src/lib.rs", "src/service.rs", "src/models/mod.rs"]);
        let got = resolve_rust_specifier("self::service", "src/lib.rs", &known);
        assert_eq!(got.as_deref(), Some("src/service.rs"));
        let got2 = resolve_rust_specifier("self::models", "src/lib.rs", &known);
        assert_eq!(got2.as_deref(), Some("src/models/mod.rs"));
    }

    #[test]
    fn self_from_main_rs_resolves_to_crate_root_sibling() {
        let known = files(&["src/main.rs", "src/cli.rs"]);
        let got = resolve_rust_specifier("self::cli", "src/main.rs", &known);
        assert_eq!(got.as_deref(), Some("src/cli.rs"));
    }

    #[test]
    fn bare_sibling_submodule_resolves_locally() {
        // `use resolve::X` from `indexing/imports/mod.rs` where `resolve.rs`
        // is declared `mod resolve;` in that same file. The sibling file
        // exists in the from-file's own directory, so the edge resolves.
        let known = files(&[
            "src/indexing/imports/mod.rs",
            "src/indexing/imports/resolve.rs",
            "src/framework/laravel/resolve.rs", // decoy — should NOT be picked
        ]);
        let got = resolve_rust_specifier(
            "resolve::resolve_specifier",
            "src/indexing/imports/mod.rs",
            &known,
        );
        assert_eq!(got.as_deref(), Some("src/indexing/imports/resolve.rs"));
    }

    #[test]
    fn bare_non_sibling_still_rejected() {
        // `use regex::Regex` from `nextjs/middleware.rs`: there's no
        // `nextjs/regex.rs`, so even though `laravel/regex.rs` exists
        // elsewhere, the specifier stays external.
        let known = files(&[
            "src/framework/nextjs/middleware.rs",
            "src/framework/laravel/regex.rs",
        ]);
        assert!(
            resolve_rust_specifier("regex", "src/framework/nextjs/middleware.rs", &known).is_none()
        );
    }

    #[test]
    fn crate_root_from_workspace_crate_uses_its_own_src() {
        // A workspace-member file's `crate::` prefix must resolve inside
        // that member's `src/`, not the outer repo's `src/`.
        let known = files(&[
            "src/main.rs",
            "crates/cs-storage/src/lib.rs",
            "crates/cs-storage/src/db.rs",
        ]);
        let got = resolve_rust_specifier("crate::db", "crates/cs-storage/src/lib.rs", &known);
        assert_eq!(got.as_deref(), Some("crates/cs-storage/src/db.rs"));
    }
}
