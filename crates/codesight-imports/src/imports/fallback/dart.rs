use crate::imports::ImportEdge;

use super::helpers::extract_quoted_string;

pub(crate) fn extract_imports(content: &str) -> Vec<ImportEdge> {
    let mut edges = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("import ") || trimmed.starts_with("export ") {
            let after = &trimmed[7..];
            let after = after.trim().trim_end_matches(';').trim();
            if let Some(specifier) = extract_quoted_string(after) {
                edges.push(ImportEdge {
                    specifier: specifier.to_string(),
                    names: Vec::new(),
                });
            }
        } else if let Some(after) = trimmed.strip_prefix("part ") {
            let after = after.trim().trim_end_matches(';').trim();
            if !after.starts_with("of ")
                && let Some(specifier) = extract_quoted_string(after)
            {
                edges.push(ImportEdge {
                    specifier: specifier.to_string(),
                    names: Vec::new(),
                });
            }
        }
    }
    edges
}
