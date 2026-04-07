//! Line- and scanner-based import extraction (fallback when tree-sitter parse is unusable).

use std::collections::{HashMap, HashSet};

use super::ImportEdge;

pub(super) fn deduplicate_edges(edges: Vec<ImportEdge>, _file_path: &str) -> Vec<ImportEdge> {
    let mut seen: HashMap<String, Vec<String>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for edge in edges {
        let entry = seen.entry(edge.specifier.clone()).or_insert_with(|| {
            order.push(edge.specifier.clone());
            Vec::new()
        });
        for name in &edge.names {
            if !entry.contains(name) {
                entry.push(name.clone());
            }
        }
    }
    order
        .into_iter()
        .map(|spec| ImportEdge {
            names: seen.remove(&spec).unwrap_or_default(),
            specifier: spec,
        })
        .collect()
}

pub(crate) fn extract_imports_python(content: &str) -> Vec<ImportEdge> {
    let mut edges = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with('#') || t.is_empty() {
            continue;
        }
        if t.starts_with("from ") {
            if let Some(rest) = t.strip_prefix("from ") {
                let parts: Vec<&str> = rest.splitn(2, " import ").collect();
                if parts.len() == 2 {
                    let module = parts[0].trim();
                    if module == "__future__" {
                        continue;
                    }
                    let imports_str = parts[1].trim();
                    let names: Vec<String> = if imports_str == "*" {
                        Vec::new()
                    } else {
                        imports_str
                            .split(',')
                            .map(|s| {
                                let s = s.trim();
                                s.split_whitespace().next().unwrap_or(s).to_string()
                            })
                            .filter(|s| !s.is_empty())
                            .collect()
                    };
                    edges.push(ImportEdge {
                        specifier: module.to_string(),
                        names,
                    });
                }
            }
        } else if t.starts_with("import ") {
            if let Some(rest) = t.strip_prefix("import ") {
                let module = rest
                    .split_whitespace()
                    .next()
                    .unwrap_or(rest)
                    .trim_end_matches(',');
                if !module.is_empty() {
                    edges.push(ImportEdge {
                        specifier: module.to_string(),
                        names: Vec::new(),
                    });
                }
            }
        }
    }
    edges
}

pub(crate) fn extract_imports_js(content: &str) -> Vec<ImportEdge> {
    let mut edges = Vec::new();
    let mut seen_specifiers: HashSet<String> = HashSet::new();

    for line in content.lines() {
        let t = line.trim();

        if t.starts_with("import ") {
            if let Some(spec) = extract_js_static_import(t) {
                if seen_specifiers.insert(spec.specifier.clone()) {
                    edges.push(spec);
                }
                continue;
            }
        }

        if t.starts_with("export ") && t.contains(" from ") {
            if let Some(spec) = extract_js_reexport(t) {
                if seen_specifiers.insert(spec.specifier.clone()) {
                    edges.push(spec);
                }
                continue;
            }
        }

        if t.contains("require(") {
            if let Some(spec) = extract_js_require(t) {
                if seen_specifiers.insert(spec.specifier.clone()) {
                    edges.push(spec);
                }
            }
        }

        if t.contains("import(") {
            for spec in extract_js_dynamic_imports(t) {
                if seen_specifiers.insert(spec.specifier.clone()) {
                    edges.push(spec);
                }
            }
        }
    }
    edges
}

fn extract_quoted_string(s: &str) -> Option<&str> {
    let start_single = s.find('\'');
    let start_double = s.find('"');
    let (start, quote) = match (start_single, start_double) {
        (Some(a), Some(b)) => {
            if a < b {
                (a, '\'')
            } else {
                (b, '"')
            }
        }
        (Some(a), None) => (a, '\''),
        (None, Some(b)) => (b, '"'),
        (None, None) => return None,
    };
    let rest = &s[start + 1..];
    let end = rest.find(quote)?;
    Some(&rest[..end])
}

pub(crate) fn extract_quoted_string_with_rest(s: &str) -> Option<(&str, &str)> {
    let start_single = s.find('\'');
    let start_double = s.find('"');
    let (start, quote) = match (start_single, start_double) {
        (Some(a), Some(b)) => {
            if a < b {
                (a, '\'')
            } else {
                (b, '"')
            }
        }
        (Some(a), None) => (a, '\''),
        (None, Some(b)) => (b, '"'),
        (None, None) => return None,
    };
    let rest = &s[start + 1..];
    let end = rest.find(quote)?;
    Some((&rest[..end], &rest[end + 1..]))
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
        let mut p = Vec::new();
        if !before.is_empty() && !before.starts_with('*') {
            p.push(before);
        }
        for item in inside.split(',') {
            let item = item.trim();
            if !item.is_empty() {
                p.push(item);
            }
        }
        p
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
        .map(|s| s.trim().split_whitespace().next().unwrap_or("").to_string())
        .filter(|s| !s.is_empty())
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

const VUE_BUILTINS: &[&str] = &[
    "Transition",
    "TransitionGroup",
    "KeepAlive",
    "Teleport",
    "Suspense",
    "Component",
    "Slot",
];

const HTML_ELEMENTS: &[&str] = &[
    "a",
    "abbr",
    "address",
    "area",
    "article",
    "aside",
    "audio",
    "b",
    "base",
    "bdi",
    "bdo",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "cite",
    "code",
    "col",
    "colgroup",
    "data",
    "datalist",
    "dd",
    "del",
    "details",
    "dfn",
    "dialog",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "i",
    "iframe",
    "img",
    "input",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "link",
    "main",
    "map",
    "mark",
    "menu",
    "meta",
    "meter",
    "nav",
    "noscript",
    "object",
    "ol",
    "optgroup",
    "option",
    "output",
    "p",
    "param",
    "picture",
    "pre",
    "progress",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "script",
    "section",
    "select",
    "slot",
    "small",
    "source",
    "span",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "time",
    "title",
    "tr",
    "track",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
    "svg",
    "path",
    "circle",
    "rect",
    "line",
    "polyline",
    "polygon",
    "text",
    "g",
    "defs",
    "use",
    "clipPath",
    "mask",
    "component",
    "transition",
    "transition-group",
    "keep-alive",
    "teleport",
    "suspense",
    "slot",
];

pub(crate) fn extract_imports_vue(content: &str) -> Vec<ImportEdge> {
    let mut edges = Vec::new();

    let script_content = extract_vue_script_block(content);
    let mut script_edges = Vec::new();
    if let Some(ref script) = script_content {
        script_edges = extract_imports_js(script);
    }

    let mut imported_names: HashSet<String> = HashSet::new();
    for edge in &script_edges {
        for name in &edge.names {
            imported_names.insert(name.clone());
        }
        if let Some(stem) = edge.specifier.rsplit('/').next() {
            let stem = stem.split('.').next().unwrap_or(stem);
            imported_names.insert(stem.to_string());
        }
    }
    edges.extend(script_edges);

    if let Some(template) = extract_vue_template_block(content) {
        let components = extract_template_components(&template);
        for comp in components {
            if !imported_names.contains(&comp) {
                edges.push(ImportEdge {
                    specifier: comp,
                    names: Vec::new(),
                });
            }
        }
    }

    edges
}

fn extract_vue_script_block(content: &str) -> Option<String> {
    let lower = content.to_ascii_lowercase();
    let start = lower.find("<script")?;
    let tag_end = content[start..].find('>')? + start + 1;
    let end = lower[tag_end..].find("</script>")?;
    Some(content[tag_end..tag_end + end].to_string())
}

fn extract_vue_template_block(content: &str) -> Option<String> {
    let lower = content.to_ascii_lowercase();
    let start = lower.find("<template")?;
    let tag_end = content[start..].find('>')? + start + 1;
    let end = lower[tag_end..].find("</template>")?;
    Some(content[tag_end..tag_end + end].to_string())
}

fn kebab_to_pascal(s: &str) -> String {
    s.split('-')
        .map(|part| {
            let mut c = part.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().chain(c).collect(),
            }
        })
        .collect()
}

fn extract_template_components(template: &str) -> Vec<String> {
    let mut components = HashSet::new();
    let html_set: HashSet<&str> = HTML_ELEMENTS.iter().copied().collect();
    let builtin_set: HashSet<&str> = VUE_BUILTINS.iter().copied().collect();

    let bytes = template.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'<' && i + 1 < len && bytes[i + 1] != b'/' {
            i += 1;
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let start = i;
            while i < len
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_')
            {
                i += 1;
            }
            if i > start {
                let tag = &template[start..i];
                let pascal = if tag.contains('-') {
                    kebab_to_pascal(tag)
                } else {
                    tag.to_string()
                };
                let tag_lower = tag.to_ascii_lowercase();
                if !html_set.contains(tag_lower.as_str())
                    && !builtin_set.contains(pascal.as_str())
                    && tag
                        .chars()
                        .next()
                        .map_or(false, |c| c.is_ascii_uppercase() || tag.contains('-'))
                {
                    components.insert(pascal);
                }
            }
        } else {
            i += 1;
        }
    }

    let mut result: Vec<String> = components.into_iter().collect();
    result.sort();
    result
}

pub(crate) fn extract_imports_sql(content: &str) -> Vec<ImportEdge> {
    let mut edges = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i + 1 < len {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let block_start = i + 2;
            if let Some(end_offset) = content[block_start..].find("}}") {
                let block = &content[block_start..block_start + end_offset];
                let block_trimmed = block
                    .trim()
                    .trim_start_matches('-')
                    .trim()
                    .trim_end_matches('-')
                    .trim();

                if block_trimmed.starts_with("ref(") {
                    let after_ref = &block_trimmed[4..];
                    if let Some(name) = extract_quoted_string(after_ref) {
                        let spec = name.to_string();
                        if !spec.is_empty() && seen.insert(spec.clone()) {
                            edges.push(ImportEdge {
                                specifier: spec,
                                names: Vec::new(),
                            });
                        }
                    }
                } else if block_trimmed.starts_with("source(") {
                    let after_source = &block_trimmed[7..];
                    if let Some((schema, rest)) = extract_quoted_string_with_rest(after_source) {
                        if let Some(table) = extract_quoted_string(rest) {
                            let spec = format!("source:{schema}.{table}");
                            if seen.insert(spec.clone()) {
                                edges.push(ImportEdge {
                                    specifier: spec,
                                    names: Vec::new(),
                                });
                            }
                        }
                    }
                }
                i = block_start + end_offset + 2;
            } else {
                i += 2;
            }
        } else {
            i += 1;
        }
    }

    edges
}

pub(crate) fn extract_imports_dart(content: &str) -> Vec<ImportEdge> {
    let mut edges = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with("import ") || t.starts_with("export ") {
            let after = &t[7..];
            let after = after.trim().trim_end_matches(';').trim();
            if let Some(specifier) = extract_quoted_string(after) {
                edges.push(ImportEdge {
                    specifier: specifier.to_string(),
                    names: Vec::new(),
                });
            }
        } else if t.starts_with("part ") {
            let after = t[5..].trim().trim_end_matches(';').trim();
            if !after.starts_with("of ") {
                if let Some(specifier) = extract_quoted_string(after) {
                    edges.push(ImportEdge {
                        specifier: specifier.to_string(),
                        names: Vec::new(),
                    });
                }
            }
        }
    }
    edges
}

/// Go import paths as edges (fallback when tree-sitter is unavailable).
pub(crate) fn extract_imports_go(content: &str) -> Vec<ImportEdge> {
    let mut edges = Vec::new();
    let mut in_block = false;

    for line in content.lines() {
        let t = line.trim();
        if !in_block && t.starts_with("import ") {
            if t.starts_with("import (") {
                in_block = true;
                continue;
            }
            let rest = t[7..].trim();
            if let Some(spec) = extract_quoted_string(rest) {
                edges.push(ImportEdge {
                    specifier: spec.to_string(),
                    names: Vec::new(),
                });
            }
        } else if in_block {
            if t == ")" {
                in_block = false;
                continue;
            }
            if let Some(spec) = extract_quoted_string(t) {
                edges.push(ImportEdge {
                    specifier: spec.to_string(),
                    names: Vec::new(),
                });
            }
        }
    }
    edges
}
