//! Rust import extraction (tree-sitter primary + line-based fallback).

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
