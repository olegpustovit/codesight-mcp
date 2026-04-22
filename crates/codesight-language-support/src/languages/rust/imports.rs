//! Rust import extraction (tree-sitter primary + line-based fallback) plus
//! anchor-aware specifier → file resolution.

use std::collections::HashMap;

use tree_sitter::Node;

use crate::frameworks::parse_tree;
use crate::imports_types::{ImportEdge, source_slice};

pub fn extract(source: &str) -> Vec<ImportEdge> {
    extract_ts(source).unwrap_or_else(|| extract_fallback(source))
}

fn extract_ts(source: &str) -> Option<Vec<ImportEdge>> {
    let tree = parse_tree(tree_sitter_rust::LANGUAGE, source)?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }
    let mut edges = Vec::new();
    walk(root, source, &mut edges);
    Some(edges)
}

fn walk(node: Node<'_>, source: &str, edges: &mut Vec<ImportEdge>) {
    match node.kind() {
        "use_declaration" => {
            if let Some(arg) = node.child_by_field_name("argument") {
                collect_use(arg, source, "", edges);
            }
            return;
        }
        "extern_crate_declaration" => {
            if let Some(name_n) = node.child_by_field_name("name")
                && let Some(name) = source_slice(name_n, source)
            {
                edges.push(ImportEdge {
                    specifier: name.to_string(),
                    names: Vec::new(),
                });
            }
            return;
        }
        "mod_item" => {
            let has_body = node.child_by_field_name("body").is_some();
            if !has_body
                && let Some(name_n) = node.child_by_field_name("name")
                && let Some(name) = source_slice(name_n, source)
            {
                edges.push(ImportEdge {
                    specifier: format!("self::{name}"),
                    names: Vec::new(),
                });
                return;
            }
        }
        _ => {}
    }
    let mut i = 0u32;
    while let Some(c) = node.child(i) {
        walk(c, source, edges);
        i += 1;
    }
}

fn collect_use(node: Node<'_>, source: &str, prefix: &str, edges: &mut Vec<ImportEdge>) {
    match node.kind() {
        "scoped_use_list" => {
            let path_segment = node
                .child_by_field_name("path")
                .and_then(|p| path_text(p, source));
            let new_prefix = join_prefix(prefix, path_segment.as_deref());
            if let Some(list) = node.child_by_field_name("list") {
                collect_use(list, source, &new_prefix, edges);
            }
        }
        "use_list" => {
            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i as u32) {
                    collect_use(child, source, prefix, edges);
                }
            }
        }
        "identifier" | "crate" | "self" | "super" => {
            if let Some(name) = source_slice(node, source) {
                edges.push(ImportEdge {
                    specifier: prefix.to_string(),
                    names: vec![name.to_string()],
                });
            }
        }
        "scoped_identifier" => {
            let leaf = node
                .child_by_field_name("name")
                .and_then(|n| source_slice(n, source))
                .map(|s| s.to_string());
            let inner_prefix = node
                .child_by_field_name("path")
                .and_then(|p| path_text(p, source));
            let full_prefix = join_prefix(prefix, inner_prefix.as_deref());
            if let Some(leaf) = leaf {
                edges.push(ImportEdge {
                    specifier: full_prefix,
                    names: vec![leaf],
                });
            }
        }
        "use_as_clause" => {
            if let Some(path_n) = node.child_by_field_name("path") {
                collect_use(path_n, source, prefix, edges);
            }
        }
        "use_wildcard" => {
            let mut path = None;
            for i in 0..node.named_child_count() {
                if let Some(c) = node.named_child(i as u32)
                    && matches!(
                        c.kind(),
                        "identifier" | "scoped_identifier" | "crate" | "self" | "super"
                    )
                {
                    path = path_text(c, source);
                    break;
                }
            }
            let full = join_prefix(prefix, path.as_deref());
            edges.push(ImportEdge {
                specifier: full,
                names: Vec::new(),
            });
        }
        _ => {}
    }
}

fn path_text(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "crate" | "self" | "super" | "metavariable" => {
            source_slice(node, source).map(|s| s.to_string())
        }
        "scoped_identifier" => {
            let leaf = node
                .child_by_field_name("name")
                .and_then(|n| source_slice(n, source))
                .map(|s| s.to_string())?;
            let prefix = node
                .child_by_field_name("path")
                .and_then(|p| path_text(p, source));
            Some(join_prefix(prefix.as_deref().unwrap_or(""), Some(&leaf)))
        }
        _ => source_slice(node, source).map(|s| s.to_string()),
    }
}

fn join_prefix(outer: &str, inner: Option<&str>) -> String {
    match (outer.is_empty(), inner) {
        (true, Some(i)) => i.to_string(),
        (false, Some(i)) => format!("{outer}::{i}"),
        (_, None) => outer.to_string(),
    }
}

fn extract_fallback(content: &str) -> Vec<ImportEdge> {
    let mut edges = Vec::new();
    for raw in content.lines() {
        let line = strip_inline_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let Some(body) = line_body(line) else {
            continue;
        };
        if let Some(rest) = body.strip_prefix("extern crate ") {
            if let Some(name) = first_ident(rest.trim()) {
                edges.push(ImportEdge {
                    specifier: name.to_string(),
                    names: Vec::new(),
                });
            }
            continue;
        }
        if let Some(rest) = body.strip_prefix("use ") {
            parse_use_line(rest, &mut edges);
            continue;
        }
        if let Some(rest) = body.strip_prefix("mod ")
            && !rest.contains('{')
            && let Some(name) = first_ident(rest.trim())
        {
            edges.push(ImportEdge {
                specifier: format!("self::{name}"),
                names: Vec::new(),
            });
        }
    }
    edges
}

fn strip_inline_comment(line: &str) -> &str {
    match line.find("//") {
        Some(idx) => &line[..idx],
        None => line,
    }
}

fn line_body(line: &str) -> Option<&str> {
    let stripped = line.strip_suffix(';').unwrap_or(line).trim();
    let after_vis = if let Some(rest) = stripped.strip_prefix("pub") {
        let rest = rest.trim_start();
        if let Some(rest) = rest.strip_prefix('(') {
            let close = rest.find(')')?;
            rest[close + 1..].trim_start()
        } else {
            rest
        }
    } else {
        stripped
    };
    Some(after_vis)
}

fn parse_use_line(rest: &str, edges: &mut Vec<ImportEdge>) {
    let rest = rest.trim();
    let path_end = match rest.find(" as ") {
        Some(i) => &rest[..i],
        None => rest,
    };
    if let Some(brace_open) = path_end.rfind("::{") {
        let prefix = &path_end[..brace_open];
        let after = &path_end[brace_open + 3..];
        let brace_close = after.rfind('}').unwrap_or(after.len());
        let inner = &after[..brace_close];
        let mut any = false;
        for part in inner.split(',') {
            let leaf = part.trim();
            if leaf.is_empty() || leaf.contains('{') {
                continue;
            }
            let (_, last) = split_last_segment(leaf);
            edges.push(ImportEdge {
                specifier: prefix.to_string(),
                names: vec![last.to_string()],
            });
            any = true;
        }
        if any {
            return;
        }
        edges.push(ImportEdge {
            specifier: prefix.to_string(),
            names: Vec::new(),
        });
        return;
    }
    if let Some(prefix) = path_end.strip_suffix("::*") {
        edges.push(ImportEdge {
            specifier: prefix.to_string(),
            names: Vec::new(),
        });
        return;
    }
    let (prefix, last) = split_last_segment(path_end);
    if prefix.is_empty() {
        edges.push(ImportEdge {
            specifier: last.to_string(),
            names: Vec::new(),
        });
    } else {
        edges.push(ImportEdge {
            specifier: prefix.to_string(),
            names: vec![last.to_string()],
        });
    }
}

fn split_last_segment(path: &str) -> (&str, &str) {
    match path.rfind("::") {
        Some(i) => (&path[..i], &path[i + 2..]),
        None => ("", path),
    }
}

fn first_ident(s: &str) -> Option<&str> {
    let end = s
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    if end == 0 { None } else { Some(&s[..end]) }
}

// ── Specifier resolution ─────────────────────────────────────────────────
//
// Rust overrides `Language::resolve_import_edge` because its import graph
// has semantics external resolvers can't mimic: `crate::` / `self::` /
// `super::` anchors, nested `::` paths, workspace-member crate names.

/// Dispatch entry used by `Rust::resolve_import_edge`. Builds the full
/// module path from `(specifier, names[0])` the way the Rust `use` walker
/// would, then runs the anchored resolver.
pub fn resolve_edge(
    edge: &ImportEdge,
    from_file: &str,
    known_files_vec: &[String],
    workspace_crates: Option<&HashMap<String, String>>,
) -> Option<String> {
    let full_path = if let Some(first) = edge.names.first() {
        if edge.specifier.is_empty() {
            first.clone()
        } else {
            format!("{}::{}", edge.specifier, first)
        }
    } else {
        edge.specifier.clone()
    };
    resolve_rust_specifier(&full_path, from_file, known_files_vec, workspace_crates)
}

/// Map a Rust `::`-path specifier to a local file, stripping `crate::`,
/// `self::`, `super::` prefixes and attempting both `stem.rs` and
/// `stem/mod.rs` candidates against the known-files list.
pub fn resolve_rust_specifier(
    specifier: &str,
    from_file: &str,
    known_files: &[String],
    workspace_crates: Option<&HashMap<String, String>>,
) -> Option<String> {
    let mut segments: Vec<&str> = specifier.split("::").filter(|s| !s.is_empty()).collect();
    let first = segments.first().copied()?;
    let is_anchored = matches!(first, "crate" | "self" | "super");
    if !is_anchored {
        if let Some(map) = workspace_crates
            && let Some(lib_rs) = map.get(first)
            && lib_rs.as_str() != from_file
        {
            return Some(lib_rs.clone());
        }
        return resolve_rust_bare_sibling(&segments, from_file, known_files);
    }

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

    let base_dir = if has_crate {
        crate_root_of(from_file)?
    } else if has_self {
        self_module_dir(from_file)?
    } else if super_steps > 0 {
        let mut dir = parent_module_dir(from_file)?;
        for _ in 1..super_steps {
            dir = parent_dir_string(&dir)?;
        }
        dir
    } else {
        return None;
    };

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

fn resolve_rust_bare_sibling(
    segments: &[&str],
    from_file: &str,
    known_files: &[String],
) -> Option<String> {
    let first = *segments.first()?;
    let dir = parent_dir_string(from_file)?;
    let candidates = [format!("{dir}/{first}.rs"), format!("{dir}/{first}/mod.rs")];
    let first_hit = candidates
        .iter()
        .find(|c| known_files.iter().any(|f| f == *c))?;
    if segments.len() == 1 {
        return Some(first_hit.clone());
    }
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
    Some(first_hit.clone())
}

fn crate_root_of(from_file: &str) -> Option<String> {
    let parts: Vec<&str> = from_file.split('/').collect();
    let src_idx = parts.iter().rposition(|p| *p == "src")?;
    let prefix: Vec<&str> = parts[..=src_idx].to_vec();
    Some(prefix.join("/"))
}

fn parent_module_dir(from_file: &str) -> Option<String> {
    if let Some(parent) = from_file.strip_suffix("/mod.rs") {
        return parent_dir_string(parent);
    }
    parent_dir_string(from_file)
}

fn self_module_dir(from_file: &str) -> Option<String> {
    if let Some(parent) = from_file.strip_suffix("/mod.rs") {
        return Some(parent.to_string());
    }
    if from_file.ends_with("/lib.rs") || from_file.ends_with("/main.rs") {
        return parent_dir_string(from_file);
    }
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

    fn resolve(specifier: &str, from_file: &str, known: &[String]) -> Option<String> {
        resolve_rust_specifier(specifier, from_file, known, None)
    }

    #[test]
    fn crate_prefix_resolves_under_src_root() {
        let known = files(&["src/lib.rs", "src/models.rs"]);
        assert_eq!(
            resolve("crate::models", "src/lib.rs", &known).as_deref(),
            Some("src/models.rs"),
        );
    }

    #[test]
    fn nested_crate_path_prefers_longest_match() {
        let known = files(&["src/lib.rs", "src/a/b.rs"]);
        assert_eq!(
            resolve("crate::a::b", "src/lib.rs", &known).as_deref(),
            Some("src/a/b.rs"),
        );
    }

    #[test]
    fn crate_mod_rs_candidate_matches() {
        let known = files(&["src/lib.rs", "src/a/mod.rs"]);
        assert_eq!(
            resolve("crate::a", "src/lib.rs", &known).as_deref(),
            Some("src/a/mod.rs"),
        );
    }

    #[test]
    fn external_crate_returns_none() {
        let known = files(&["src/lib.rs", "src/models.rs"]);
        assert!(resolve("serde", "src/lib.rs", &known).is_none());
        assert!(resolve("std::collections", "src/lib.rs", &known).is_none());
    }

    #[test]
    fn bare_crate_name_does_not_false_match_local_file() {
        let known = files(&["src/framework/laravel/regex.rs", "src/lib.rs"]);
        assert!(resolve("regex", "src/lib.rs", &known).is_none());
    }

    #[test]
    fn super_resolves_to_sibling_in_same_dir() {
        let known = files(&[
            "src/framework/laravel/provider.rs",
            "src/framework/laravel/render.rs",
            "src/framework/nuxt/render.rs",
        ]);
        assert_eq!(
            resolve("super::render", "src/framework/laravel/provider.rs", &known).as_deref(),
            Some("src/framework/laravel/render.rs"),
        );
    }

    #[test]
    fn super_super_walks_two_levels_up() {
        let known = files(&[
            "src/framework/laravel/provider.rs",
            "src/framework/shared.rs",
        ]);
        assert_eq!(
            resolve(
                "super::super::shared",
                "src/framework/laravel/provider.rs",
                &known,
            )
            .as_deref(),
            Some("src/framework/shared.rs"),
        );
    }

    #[test]
    fn super_from_mod_rs_steps_out_of_module() {
        let known = files(&["src/framework/laravel/mod.rs", "src/framework/nuxt.rs"]);
        assert_eq!(
            resolve("super::nuxt", "src/framework/laravel/mod.rs", &known).as_deref(),
            Some("src/framework/nuxt.rs"),
        );
    }

    #[test]
    fn self_from_mod_rs_stays_in_module() {
        let known = files(&["src/a/mod.rs", "src/a/inner.rs"]);
        assert_eq!(
            resolve("self::inner", "src/a/mod.rs", &known).as_deref(),
            Some("src/a/inner.rs"),
        );
    }

    #[test]
    fn self_from_lib_rs_resolves_to_crate_root_sibling() {
        let known = files(&["src/lib.rs", "src/service.rs", "src/models/mod.rs"]);
        assert_eq!(
            resolve("self::service", "src/lib.rs", &known).as_deref(),
            Some("src/service.rs"),
        );
        assert_eq!(
            resolve("self::models", "src/lib.rs", &known).as_deref(),
            Some("src/models/mod.rs"),
        );
    }

    #[test]
    fn self_from_main_rs_resolves_to_crate_root_sibling() {
        let known = files(&["src/main.rs", "src/cli.rs"]);
        assert_eq!(
            resolve("self::cli", "src/main.rs", &known).as_deref(),
            Some("src/cli.rs"),
        );
    }

    #[test]
    fn bare_sibling_submodule_resolves_locally() {
        let known = files(&[
            "src/indexing/imports/mod.rs",
            "src/indexing/imports/resolve.rs",
            "src/framework/laravel/resolve.rs",
        ]);
        assert_eq!(
            resolve(
                "resolve::resolve_specifier",
                "src/indexing/imports/mod.rs",
                &known,
            )
            .as_deref(),
            Some("src/indexing/imports/resolve.rs"),
        );
    }

    #[test]
    fn bare_non_sibling_still_rejected() {
        let known = files(&[
            "src/framework/nextjs/middleware.rs",
            "src/framework/laravel/regex.rs",
        ]);
        assert!(resolve("regex", "src/framework/nextjs/middleware.rs", &known).is_none());
    }

    #[test]
    fn crate_root_from_workspace_crate_uses_its_own_src() {
        let known = files(&[
            "src/main.rs",
            "crates/cs-storage/src/lib.rs",
            "crates/cs-storage/src/db.rs",
        ]);
        assert_eq!(
            resolve("crate::db", "crates/cs-storage/src/lib.rs", &known).as_deref(),
            Some("crates/cs-storage/src/db.rs"),
        );
    }

    #[test]
    fn workspace_crate_bare_use_resolves_to_lib_rs() {
        let known = files(&[
            "src/main.rs",
            "crates/my-lib/src/lib.rs",
            "crates/my-lib/src/inner.rs",
        ]);
        let mut ws = HashMap::new();
        ws.insert(
            "my_lib".to_string(),
            "crates/my-lib/src/lib.rs".to_string(),
        );
        let got = resolve_rust_specifier("my_lib::do_thing", "src/main.rs", &known, Some(&ws));
        assert_eq!(got.as_deref(), Some("crates/my-lib/src/lib.rs"));
    }

    #[test]
    fn workspace_crate_wins_over_bare_sibling_even_when_sibling_exists() {
        let known = files(&[
            "src/main.rs",
            "src/utils.rs",
            "crates/utils/src/lib.rs",
        ]);
        let mut ws = HashMap::new();
        ws.insert("utils".to_string(), "crates/utils/src/lib.rs".to_string());
        let got = resolve_rust_specifier("utils::helper", "src/main.rs", &known, Some(&ws));
        assert_eq!(got.as_deref(), Some("crates/utils/src/lib.rs"));
    }

    #[test]
    fn workspace_crate_does_not_apply_to_anchored_paths() {
        let known = files(&[
            "src/main.rs",
            "src/utils.rs",
            "crates/utils/src/lib.rs",
        ]);
        let mut ws = HashMap::new();
        ws.insert("utils".to_string(), "crates/utils/src/lib.rs".to_string());
        let got = resolve_rust_specifier("self::utils", "src/main.rs", &known, Some(&ws));
        assert_eq!(got.as_deref(), Some("src/utils.rs"));
    }
}


