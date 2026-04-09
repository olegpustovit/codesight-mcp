//! Top-level Python module name stems for repo-wide import heuristics (e.g. "most imported" files).
//! This remains local to the storage crate so `IndexStore` analysis helpers do not
//! depend on the root indexing module shape.

use tree_sitter::Node;

/// For each import in `content`, return the first path segment of the module path
/// (for example `foo` from `foo.bar`). Relative imports are skipped because they
/// do not participate in filename-stem matching.
pub fn python_top_level_import_stems(content: &str, _file_path: &str) -> Vec<String> {
    extract_stems_with_tree_sitter(content).unwrap_or_else(|| extract_stems_fallback(content))
}

fn extract_stems_with_tree_sitter(content: &str) -> Option<Vec<String>> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(content, None)?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }

    let mut stems = Vec::new();
    walk_module(root, content, &mut stems);
    Some(stems)
}

fn walk_module(node: Node<'_>, source: &str, stems: &mut Vec<String>) {
    match node.kind() {
        "import_statement" => {
            collect_import_statement(node, source, stems);
            return;
        }
        "import_from_statement" => {
            collect_import_from_statement(node, source, stems);
            return;
        }
        "future_import_statement" => return,
        _ => {}
    }

    let mut index = 0u32;
    while let Some(child) = node.child(index) {
        walk_module(child, source, stems);
        index += 1;
    }
}

fn collect_import_statement(node: Node<'_>, source: &str, stems: &mut Vec<String>) {
    let mut index = 0u32;
    while let Some(child) = node.child(index) {
        match child.kind() {
            "dotted_name" => push_dotted_name_stem(child, source, stems),
            "aliased_import" => {
                if let Some(name_node) = child.child_by_field_name("name")
                    && name_node.kind() == "dotted_name"
                {
                    push_dotted_name_stem(name_node, source, stems);
                }
            }
            _ => {}
        }
        index += 1;
    }
}

fn collect_import_from_statement(node: Node<'_>, source: &str, stems: &mut Vec<String>) {
    let Some(module_node) = node.child_by_field_name("module_name") else {
        return;
    };

    match module_node.kind() {
        "dotted_name" => push_dotted_name_stem(module_node, source, stems),
        "relative_import" => {}
        _ => {}
    }
}

fn push_dotted_name_stem(node: Node<'_>, source: &str, stems: &mut Vec<String>) {
    let Ok(text) = node.utf8_text(source.as_bytes()) else {
        return;
    };
    let dotted = text.trim();
    if dotted.is_empty() || dotted == "__future__" {
        return;
    }
    if let Some(stem) = dotted.split('.').next()
        && !stem.is_empty()
    {
        stems.push(stem.to_string());
    }
}

fn extract_stems_fallback(content: &str) -> Vec<String> {
    let mut stems = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("from ") {
            let parts: Vec<&str> = rest.splitn(2, " import ").collect();
            if let Some(module) = parts.first().map(|part| part.trim()) {
                if module.is_empty() || module.starts_with('.') || module == "__future__" {
                    continue;
                }
                if let Some(stem) = module.split('.').next()
                    && !stem.is_empty()
                {
                    stems.push(stem.to_string());
                }
            }
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("import ") {
            for module in rest.split(',') {
                let module = module.split_whitespace().next().unwrap_or_default().trim();
                if module.is_empty() || module.starts_with('.') {
                    continue;
                }
                if let Some(stem) = module.split('.').next()
                    && !stem.is_empty()
                {
                    stems.push(stem.to_string());
                }
            }
        }
    }

    stems
}

#[cfg(test)]
mod tests {
    use super::python_top_level_import_stems;

    #[test]
    fn skips_relative_imports() {
        let stems = python_top_level_import_stems("from .utils import thing\n", "pkg/mod.py");
        assert!(stems.is_empty());
    }

    #[test]
    fn extracts_multiple_import_stems() {
        let stems =
            python_top_level_import_stems("from foo.bar import baz\nimport os, sys\n", "mod.py");
        assert!(stems.contains(&"foo".to_string()));
        assert!(stems.contains(&"os".to_string()));
        assert!(stems.contains(&"sys".to_string()));
    }
}
