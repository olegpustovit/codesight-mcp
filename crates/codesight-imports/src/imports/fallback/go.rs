use crate::imports::ImportEdge;

use super::helpers::extract_quoted_string;

/// Go import paths as edges when tree-sitter is unavailable.
pub(crate) fn extract_imports(content: &str) -> Vec<ImportEdge> {
    let mut edges = Vec::new();
    let mut in_block = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if !in_block && trimmed.starts_with("import ") {
            if trimmed.starts_with("import (") {
                in_block = true;
                continue;
            }
            let rest = trimmed[7..].trim();
            if let Some(specifier) = extract_quoted_string(rest) {
                edges.push(ImportEdge {
                    specifier: specifier.to_string(),
                    names: Vec::new(),
                });
            }
        } else if in_block {
            if trimmed == ")" {
                in_block = false;
                continue;
            }
            if let Some(specifier) = extract_quoted_string(trimmed) {
                edges.push(ImportEdge {
                    specifier: specifier.to_string(),
                    names: Vec::new(),
                });
            }
        }
    }
    edges
}
