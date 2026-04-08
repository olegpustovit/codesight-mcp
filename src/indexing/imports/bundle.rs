//! `context_bundle` import display strings (API output parity, not graph semantics).

use super::ImportEdge;
use super::line;
use super::tree_sitter;

/// Lines shown in `assemble_context` / `context_bundle` responses.
pub fn context_bundle_import_lines(content: &str, file_path: &str, language: &str) -> Vec<String> {
    let lang = language.to_ascii_lowercase();
    match lang.as_str() {
        "go" | "golang" => go_import_block_lines(content),
        "python" | "py" => python_import_lines_for_display(content),
        _ => format_import_edges_lines(&super::extract_imports(content, file_path, language)),
    }
}

/// Parity with legacy bundle behavior: verbatim trimmed `import` / `from` lines.
fn python_import_lines_for_display(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if t.starts_with("import ") || t.starts_with("from ") {
            out.push(t.to_string());
        }
    }
    out
}

/// Parity with `import_extract` Go helper: lines inside an `import (` block (header + specs + `)`).
fn go_import_block_lines(content: &str) -> Vec<String> {
    if let Some(edges) = tree_sitter::extract_go_imports_ts(content)
        && !edges.is_empty()
    {
        return format_go_edges_as_block_lines(content, &edges);
    }
    // Line fallback (malformed parse trees): preserve block shape when possible.
    let mut out = Vec::new();
    let mut in_block = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if !in_block && trimmed.starts_with("import (") {
            in_block = true;
            out.push(trimmed.to_string());
            continue;
        }
        if in_block {
            out.push(line.to_string());
            if trimmed == ")" {
                break;
            }
        }
    }
    if !out.is_empty() {
        return out;
    }
    // Single-line imports: one line per edge
    for e in line::extract_imports_go(content) {
        out.push(format!("import \"{}\"", e.specifier));
    }
    out
}

fn format_go_edges_as_block_lines(source: &str, edges: &[ImportEdge]) -> Vec<String> {
    if let Some(open) = source.lines().find(|l| l.trim().starts_with("import (")) {
        let mut out = vec![open.trim().to_string()];
        for e in edges {
            out.push(format!("\t\"{}\"", e.specifier));
        }
        out.push(")".to_string());
        return out;
    }
    edges
        .iter()
        .map(|e| format!("import \"{}\"", e.specifier))
        .collect()
}

fn format_import_edges_lines(edges: &[ImportEdge]) -> Vec<String> {
    let mut lines = Vec::new();
    for e in edges {
        if e.names.is_empty() {
            if e.specifier.starts_with(".") || e.specifier.starts_with("source:") {
                lines.push(format!("import {}", e.specifier));
            } else if e.specifier.contains('/') || e.specifier.starts_with('@') {
                lines.push(format!("import '{}'", e.specifier.replace('\'', "")));
            } else {
                lines.push(format!("import {}", e.specifier));
            }
        } else {
            lines.push(format!(
                "import {{ {} }} from '{}'",
                e.names.join(", "),
                e.specifier.replace('\'', "")
            ));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_three_import_lines() {
        let content = "import os\nfrom sys import argv\n\nx = 1\nfrom pathlib import Path\n";
        assert_eq!(python_import_lines_for_display(content).len(), 3);
    }

    #[test]
    fn go_block_four_lines() {
        let content = "package main\n\nimport (\n\t\"fmt\"\n\t\"os\"\n)\n\nfunc main() {}\n";
        let lines = go_import_block_lines(content);
        assert_eq!(lines.len(), 4, "{lines:?}");
    }
}
