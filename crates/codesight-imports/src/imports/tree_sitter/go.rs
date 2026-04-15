//! Go import extraction via tree-sitter.

use tree_sitter::Node;

use codesight_extractors::extractors::common::{parse_tree, source_slice};

use super::super::ImportEdge;

pub fn extract_go_imports_ts(source: &str) -> Option<Vec<ImportEdge>> {
    let tree = parse_tree(tree_sitter_go::LANGUAGE, source).ok()?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }
    let mut edges = Vec::new();
    walk_go(root, source, &mut edges);
    Some(edges)
}

fn walk_go(node: Node<'_>, source: &str, edges: &mut Vec<ImportEdge>) {
    if node.kind() == "import_declaration" {
        collect_go_import_declaration(node, source, edges);
        return;
    }
    let mut i = 0u32;
    while let Some(c) = node.child(i) {
        walk_go(c, source, edges);
        i += 1;
    }
}

fn collect_go_import_declaration(node: Node<'_>, source: &str, edges: &mut Vec<ImportEdge>) {
    for i in 0..node.named_child_count() {
        let Some(ch) = node.named_child(i as u32) else {
            continue;
        };
        if ch.kind() == "import_spec_list" {
            collect_go_import_spec_list(ch, source, edges);
        } else if ch.kind() == "import_spec" {
            collect_go_import_spec(ch, source, edges);
        }
    }
}

fn collect_go_import_spec_list(node: Node<'_>, source: &str, edges: &mut Vec<ImportEdge>) {
    for i in 0..node.named_child_count() {
        let Some(spec) = node.named_child(i as u32) else {
            continue;
        };
        if spec.kind() == "import_spec" {
            collect_go_import_spec(spec, source, edges);
        }
    }
}

fn collect_go_import_spec(node: Node<'_>, source: &str, edges: &mut Vec<ImportEdge>) {
    for i in 0..node.named_child_count() {
        let Some(p) = node.named_child(i as u32) else {
            continue;
        };
        let kind = p.kind();
        if (kind == "interpreted_string_literal" || kind == "raw_string_literal")
            && let Some(s) = go_string(p, source)
        {
            edges.push(ImportEdge {
                specifier: s,
                names: Vec::new(),
            });
        }
    }
}

fn go_string(node: Node<'_>, source: &str) -> Option<String> {
    let t = source_slice(node, source).ok()?.trim();
    if (t.starts_with('"') && t.ends_with('"') && t.len() >= 2)
        || (t.starts_with('`') && t.ends_with('`') && t.len() >= 2)
    {
        if t.starts_with('`') {
            Some(t[1..t.len() - 1].to_string())
        } else {
            // interpreted string: strip quotes, ignore escapes for graph resolution
            Some(t[1..t.len() - 1].to_string())
        }
    } else {
        Some(t.to_string())
    }
}
