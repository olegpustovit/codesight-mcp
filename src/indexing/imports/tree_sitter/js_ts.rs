//! JavaScript / TypeScript / TSX import extraction via tree-sitter.

use tree_sitter::Node;

use crate::extractors::common::{parse_tree, source_slice};

use super::super::ImportEdge;

pub fn extract_js_imports_ts(source: &str, language: &str) -> Option<Vec<ImportEdge>> {
    let lang: tree_sitter::Language = match language.to_ascii_lowercase().as_str() {
        "typescript" | "ts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        _ => tree_sitter_javascript::LANGUAGE.into(),
    };
    let tree = parse_tree(lang, source).ok()?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }
    let mut edges = Vec::new();
    walk_js(root, source, &mut edges);
    Some(edges)
}

fn walk_js(node: Node<'_>, source: &str, edges: &mut Vec<ImportEdge>) {
    match node.kind() {
        "import_statement" => collect_js_import_statement(node, source, edges),
        "export_statement" => collect_js_export_statement(node, source, edges),
        "call_expression" => {
            collect_js_call(node, source, edges);
            let mut i = 0u32;
            while let Some(c) = node.child(i) {
                walk_js(c, source, edges);
                i += 1;
            }
        }
        _ => {
            let mut i = 0u32;
            while let Some(c) = node.child(i) {
                walk_js(c, source, edges);
                i += 1;
            }
        }
    }
}

fn js_string_literal(node: Node<'_>, source: &str) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let t = source_slice(node, source).ok()?.trim();
    if (t.starts_with('"') && t.ends_with('"') && t.len() >= 2)
        || (t.starts_with('\'') && t.ends_with('\'') && t.len() >= 2)
    {
        Some(t[1..t.len() - 1].to_string())
    } else {
        Some(t.to_string())
    }
}

fn collect_js_import_statement(node: Node<'_>, source: &str, edges: &mut Vec<ImportEdge>) {
    let Some(spec_node) = node.child_by_field_name("source") else {
        return;
    };
    let Some(specifier) = js_string_literal(spec_node, source) else {
        return;
    };
    let mut names = Vec::new();
    for i in 0..node.named_child_count() {
        let Some(ch) = node.named_child(i as u32) else {
            continue;
        };
        if ch.kind() == "import_clause" {
            collect_import_clause(ch, source, &mut names);
        }
    }
    edges.push(ImportEdge { specifier, names });
}

fn collect_import_clause(node: Node<'_>, source: &str, names: &mut Vec<String>) {
    for i in 0..node.named_child_count() {
        let Some(c) = node.named_child(i as u32) else {
            continue;
        };
        match c.kind() {
            "identifier" => {
                if let Ok(s) = source_slice(c, source) {
                    names.push(s.to_string());
                }
            }
            "named_imports" => {
                for j in 0..c.named_child_count() {
                    let Some(spec) = c.named_child(j as u32) else {
                        continue;
                    };
                    if spec.kind() != "import_specifier" {
                        continue;
                    }
                    if let Some(n) = spec.child_by_field_name("name")
                        && n.kind() == "identifier"
                        && let Ok(s) = source_slice(n, source)
                    {
                        names.push(s.to_string());
                    }
                }
            }
            "namespace_import" => {
                for j in 0..c.named_child_count() {
                    let Some(id) = c.named_child(j as u32) else {
                        continue;
                    };
                    if id.kind() == "identifier"
                        && let Ok(s) = source_slice(id, source)
                    {
                        names.push(s.to_string());
                    }
                }
            }
            _ => {}
        }
    }
}

fn collect_js_export_statement(node: Node<'_>, source: &str, edges: &mut Vec<ImportEdge>) {
    let Some(spec_n) = node.child_by_field_name("source") else {
        return;
    };
    let Some(specifier) = js_string_literal(spec_n, source) else {
        return;
    };
    let mut names = Vec::new();
    for i in 0..node.named_child_count() {
        let Some(ch) = node.named_child(i as u32) else {
            continue;
        };
        if ch.kind() != "export_clause" {
            continue;
        }
        for j in 0..ch.named_child_count() {
            let Some(spec) = ch.named_child(j as u32) else {
                continue;
            };
            if spec.kind() != "export_specifier" {
                continue;
            }
            if let Some(n) = spec.child_by_field_name("name")
                && n.kind() == "identifier"
                && let Ok(s) = source_slice(n, source)
            {
                names.push(s.to_string());
            }
        }
    }
    edges.push(ImportEdge { specifier, names });
}

fn collect_js_call(node: Node<'_>, source: &str, edges: &mut Vec<ImportEdge>) {
    let Some(fn_n) = node.child_by_field_name("function") else {
        return;
    };
    let is_require =
        fn_n.kind() == "identifier" && source_slice(fn_n, source).ok() == Some("require");
    let is_dyn_import = fn_n.kind() == "import";
    if !is_require && !is_dyn_import {
        return;
    }
    let Some(args) = node.child_by_field_name("arguments") else {
        return;
    };
    for i in 0..args.named_child_count() {
        let Some(a) = args.named_child(i as u32) else {
            continue;
        };
        if a.kind() == "string" {
            if let Some(spec) = js_string_literal(a, source) {
                edges.push(ImportEdge {
                    specifier: spec,
                    names: Vec::new(),
                });
            }
            return;
        }
    }
}
