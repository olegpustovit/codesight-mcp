use std::collections::HashSet;

use crate::indexing::imports::ImportEdge;

use super::helpers::extract_quoted_string;

pub(crate) fn extract_imports(content: &str) -> Vec<ImportEdge> {
    let mut edges = Vec::new();
    let mut seen_specifiers: HashSet<String> = HashSet::new();

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("import ")
            && let Some(spec) = extract_js_static_import(trimmed)
        {
            if seen_specifiers.insert(spec.specifier.clone()) {
                edges.push(spec);
            }
            continue;
        }

        if trimmed.starts_with("export ")
            && trimmed.contains(" from ")
            && let Some(spec) = extract_js_reexport(trimmed)
        {
            if seen_specifiers.insert(spec.specifier.clone()) {
                edges.push(spec);
            }
            continue;
        }

        if trimmed.contains("require(")
            && let Some(spec) = extract_js_require(trimmed)
            && seen_specifiers.insert(spec.specifier.clone())
        {
            edges.push(spec);
        }

        if trimmed.contains("import(") {
            for spec in extract_js_dynamic_imports(trimmed) {
                if seen_specifiers.insert(spec.specifier.clone()) {
                    edges.push(spec);
                }
            }
        }
    }
    edges
}

fn extract_js_static_import(line: &str) -> Option<ImportEdge> {
    let after_import = line.strip_prefix("import ")?.trim();

    if after_import.starts_with('\'') || after_import.starts_with('"') {
        let specifier = extract_quoted_string(after_import)?;
        return Some(ImportEdge {
            specifier: specifier.to_string(),
            names: Vec::new(),
        });
    }

    let from_idx = line.find(" from ")?;
    let after_from = &line[from_idx + 6..];
    let specifier = extract_quoted_string(after_from)?;

    let between = &line["import ".len()..from_idx].trim();
    let names = extract_js_import_names(between);

    Some(ImportEdge {
        specifier: specifier.to_string(),
        names,
    })
}

fn extract_js_import_names(clause: &str) -> Vec<String> {
    let clause = clause.trim();
    let mut names = Vec::new();

    if clause.is_empty() {
        return names;
    }

    let parts: Vec<&str> = if clause.contains('{') {
        let brace_start = clause.find('{').unwrap();
        let brace_end = clause.find('}').unwrap_or(clause.len());
        let before = clause[..brace_start].trim().trim_end_matches(',').trim();
        let inside = &clause[brace_start + 1..brace_end];
        let mut items = Vec::new();
        if !before.is_empty() && !before.starts_with('*') {
            items.push(before);
        }
        for item in inside.split(',') {
            let item = item.trim();
            if !item.is_empty() {
                items.push(item);
            }
        }
        items
    } else if clause.starts_with("* as ") {
        let name = clause.strip_prefix("* as ").unwrap().trim();
        vec![name]
    } else {
        vec![clause]
    };

    for part in parts {
        let name = part.split_whitespace().next().unwrap_or(part);
        let name = name.trim_end_matches(',');
        if !name.is_empty() && name != "type" {
            names.push(name.to_string());
        }
    }
    names
}

fn extract_js_reexport(line: &str) -> Option<ImportEdge> {
    let from_idx = line.find(" from ")?;
    let after_from = &line[from_idx + 6..];
    let specifier = extract_quoted_string(after_from)?;

    let brace_start = line.find('{')?;
    let brace_end = line.find('}')?;
    let inside = &line[brace_start + 1..brace_end];
    let names: Vec<String> = inside
        .split(',')
        .map(|part| part.split_whitespace().next().unwrap_or("").to_string())
        .filter(|part| !part.is_empty())
        .collect();

    Some(ImportEdge {
        specifier: specifier.to_string(),
        names,
    })
}

fn extract_js_require(line: &str) -> Option<ImportEdge> {
    let req_idx = line.find("require(")?;
    let after = &line[req_idx + 8..];
    let specifier = extract_quoted_string(after)?;

    let mut names = Vec::new();
    let before = line[..req_idx].trim();
    if let Some(rest) = before
        .strip_prefix("const ")
        .or_else(|| before.strip_prefix("let "))
        .or_else(|| before.strip_prefix("var "))
    {
        let var_name = rest.trim().trim_end_matches('=').trim();
        if !var_name.is_empty() {
            names.push(var_name.to_string());
        }
    }

    Some(ImportEdge {
        specifier: specifier.to_string(),
        names,
    })
}

fn extract_js_dynamic_imports(line: &str) -> Vec<ImportEdge> {
    let mut edges = Vec::new();
    let mut search = line;
    while let Some(idx) = search.find("import(") {
        if idx > 0 {
            let prev = search.as_bytes()[idx - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                search = &search[idx + 7..];
                continue;
            }
        }
        let after = &search[idx + 7..];
        if let Some(specifier) = extract_quoted_string(after) {
            edges.push(ImportEdge {
                specifier: specifier.to_string(),
                names: Vec::new(),
            });
        }
        search = &search[idx + 7..];
    }
    edges
}
