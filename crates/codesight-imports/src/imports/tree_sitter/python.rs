//! Python imports via tree-sitter (primary path).

use tree_sitter::Node;

use codesight_extractors::extractors::common::{parse_tree, source_slice};

use super::super::ImportEdge;

/// Returns `None` when parse fails or the tree has syntax errors (caller should use line fallback).
pub fn extract_python_imports_ts(source: &str) -> Option<Vec<ImportEdge>> {
    let tree = parse_tree(tree_sitter_python::LANGUAGE, source).ok()?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }
    let mut edges = Vec::new();
    walk_module(root, source, &mut edges);
    Some(edges)
}

fn walk_module(node: Node<'_>, source: &str, edges: &mut Vec<ImportEdge>) {
    if node.kind() == "import_statement" {
        collect_import_statement(node, source, edges);
        return;
    }
    if node.kind() == "import_from_statement" {
        collect_import_from_statement(node, source, edges);
        return;
    }
    if node.kind() == "future_import_statement" {
        return;
    }

    let mut i = 0u32;
    while let Some(child) = node.child(i) {
        walk_module(child, source, edges);
        i += 1;
    }
}

fn dotted_name_text(node: Node<'_>, source: &str) -> Option<String> {
    let mut parts = Vec::new();
    let mut i = 0u32;
    while let Some(c) = node.child(i) {
        if c.kind() == "identifier" {
            parts.push(source_slice(c, source).ok()?.to_string());
        }
        i += 1;
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("."))
    }
}

fn relative_module_text(node: Node<'_>, source: &str) -> Option<String> {
    let full = source_slice(node, source).ok()?.trim();
    if full.is_empty() {
        None
    } else {
        Some(full.to_string())
    }
}

fn collect_import_statement(node: Node<'_>, source: &str, edges: &mut Vec<ImportEdge>) {
    let mut i = 0u32;
    while let Some(child) = node.child(i) {
        match child.kind() {
            "dotted_name" => {
                if let Some(spec) = dotted_name_text(child, source) {
                    edges.push(ImportEdge {
                        specifier: spec,
                        names: Vec::new(),
                    });
                }
            }
            "aliased_import" => {
                if let Some(name_n) = child.child_by_field_name("name")
                    && name_n.kind() == "dotted_name"
                    && let Some(spec) = dotted_name_text(name_n, source)
                {
                    edges.push(ImportEdge {
                        specifier: spec,
                        names: Vec::new(),
                    });
                }
            }
            _ => {}
        }
        i += 1;
    }
}

fn collect_import_from_statement(node: Node<'_>, source: &str, edges: &mut Vec<ImportEdge>) {
    let module_n = match node.child_by_field_name("module_name") {
        Some(n) => n,
        None => return,
    };

    let (module, skip_future) = match module_n.kind() {
        "dotted_name" => {
            let t = dotted_name_text(module_n, source).unwrap_or_default();
            let skip = t == "__future__";
            (t, skip)
        }
        "relative_import" => (
            relative_module_text(module_n, source).unwrap_or_default(),
            false,
        ),
        _ => return,
    };

    if skip_future || module.is_empty() {
        return;
    }

    for i in 0..node.named_child_count() {
        let Some(c) = node.named_child(i as u32) else {
            continue;
        };
        if c.kind() == "wildcard_import" {
            edges.push(ImportEdge {
                specifier: module,
                names: Vec::new(),
            });
            return;
        }
    }

    let mut names = Vec::new();
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if cursor.field_name() == Some("name") {
                let c = cursor.node();
                match c.kind() {
                    "dotted_name" => {
                        if let Some(sym) = dotted_name_text(c, source) {
                            names.push(sym);
                        }
                    }
                    "aliased_import" => {
                        if let Some(name_n) = c.child_by_field_name("name")
                            && name_n.kind() == "dotted_name"
                            && let Some(sym) = dotted_name_text(name_n, source)
                        {
                            names.push(sym);
                        }
                    }
                    _ => {}
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    edges.push(ImportEdge {
        specifier: module,
        names,
    });
}
