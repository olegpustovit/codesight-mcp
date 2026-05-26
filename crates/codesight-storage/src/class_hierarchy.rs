//! Class inheritance line parsing and hierarchy views.

use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

use serde::Serialize;

#[derive(Debug, Clone)]
pub struct ClassIndexRow {
    pub id: String,
    pub name: String,
    pub file_path: String,
    pub start_line: u32,
    /// First line of the indexed class span (declaration header).
    pub header_line: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HierarchyNode {
    pub name: String,
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct HierarchyView {
    pub class: HierarchyNode,
    pub ancestors: Vec<HierarchyNode>,
    pub descendants: Vec<HierarchyNode>,
    pub ancestor_count: usize,
    pub descendant_count: usize,
}

pub fn header_line_from_source(source_text: &str) -> String {
    source_text.lines().next().unwrap_or("").trim().to_string()
}

/// Parse a comma-separated Python bases clause (best-effort).
pub fn parse_bases(header_line: &str) -> Vec<String> {
    let line = header_line.split('#').next().unwrap_or(header_line).trim();
    if line.is_empty() {
        return Vec::new();
    }
    if line.contains(" extends ") || line.contains(" implements ") {
        return parse_java_bases(line);
    }
    parse_python_bases(line)
}

fn parse_python_bases(line: &str) -> Vec<String> {
    let Some(idx) = line.find("class ") else {
        return Vec::new();
    };
    let after = line[idx + 6..].trim_start();
    let Some(open) = after.find('(') else {
        return Vec::new();
    };
    let Some(rel) = after[open + 1..].find(')') else {
        return Vec::new();
    };
    let inside = after[open + 1..open + 1 + rel].trim();
    if inside.is_empty() {
        return Vec::new();
    }
    inside
        .split(',')
        .map(str::trim)
        .filter(|s| s.chars().next().is_some_and(|c| c.is_uppercase()))
        .map(|s| s.to_string())
        .collect()
}

fn parse_java_bases(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(pos) = line.find(" extends ") {
        let rest = line[pos + " extends ".len()..].trim_start();
        let end = rest
            .find(" implements ")
            .or_else(|| rest.find('{'))
            .unwrap_or(rest.len());
        let token = rest[..end].trim();
        if type_candidate(token) {
            out.push(token.to_string());
        }
    }
    if let Some(pos) = line.find(" implements ") {
        let tail = &line[pos + " implements ".len()..];
        let tail = tail.split('{').next().unwrap_or(tail).trim();
        for part in tail.split(',') {
            let p = part.trim();
            if type_candidate(p) {
                out.push(p.to_string());
            }
        }
    }
    out
}

fn type_candidate(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_uppercase())
}

pub fn compute_hierarchy_view(rows: &[ClassIndexRow], class_query: &str) -> Option<HierarchyView> {
    let q = class_query.trim();
    if q.is_empty() {
        return None;
    }
    let needle = q.to_ascii_lowercase();
    let target = rows
        .iter()
        .find(|r| r.name.to_ascii_lowercase() == needle)?;

    let by_lower: FxHashMap<String, &ClassIndexRow> = rows
        .iter()
        .map(|r| (r.name.to_ascii_lowercase(), r))
        .collect();

    let bases: Vec<Vec<String>> = rows.iter().map(|r| parse_bases(&r.header_line)).collect();

    let row_index: FxHashMap<String, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (r.name.to_ascii_lowercase(), i))
        .collect();

    let ancestors = ancestors_of(target, rows, &bases, &by_lower, &row_index);
    let descendants = descendants_of(target, rows, &bases);

    Some(HierarchyView {
        class: HierarchyNode {
            name: target.name.clone(),
            file: target.file_path.clone(),
            line: target.start_line,
        },
        ancestor_count: ancestors.len(),
        descendant_count: descendants.len(),
        ancestors,
        descendants,
    })
}

fn ancestors_of(
    target: &ClassIndexRow,
    _rows: &[ClassIndexRow],
    bases: &[Vec<String>],
    by_lower: &FxHashMap<String, &ClassIndexRow>,
    row_index: &FxHashMap<String, usize>,
) -> Vec<HierarchyNode> {
    let ti = row_index[&target.name.to_ascii_lowercase()];
    let mut out = Vec::new();
    let mut frontier: Vec<String> = bases[ti].clone();
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for base in frontier {
            if let Some(r) = by_lower.get(&base.to_ascii_lowercase()) {
                out.push(HierarchyNode {
                    name: r.name.clone(),
                    file: r.file_path.clone(),
                    line: r.start_line,
                });
                let bi = row_index[&r.name.to_ascii_lowercase()];
                next.extend(bases[bi].iter().cloned());
            } else {
                out.push(HierarchyNode {
                    name: base.clone(),
                    file: "(external)".to_string(),
                    line: 0,
                });
            }
        }
        frontier = next;
    }
    out
}

fn descendants_of(
    target: &ClassIndexRow,
    rows: &[ClassIndexRow],
    bases: &[Vec<String>],
) -> Vec<HierarchyNode> {
    let tkey = target.name.to_ascii_lowercase();
    let mut child_map: FxHashMap<String, Vec<usize>> = FxHashMap::default();
    for (i, rb) in bases.iter().enumerate() {
        for b in rb {
            child_map.entry(b.to_ascii_lowercase()).or_default().push(i);
        }
    }

    let mut out = Vec::new();
    let mut queued: FxHashSet<usize> = FxHashSet::default();
    let mut queue: VecDeque<usize> = VecDeque::new();
    if let Some(kids) = child_map.get(&tkey) {
        for &idx in kids {
            if queued.insert(idx) {
                queue.push_back(idx);
            }
        }
    }
    while let Some(i) = queue.pop_front() {
        let r = &rows[i];
        out.push(HierarchyNode {
            name: r.name.clone(),
            file: r.file_path.clone(),
            line: r.start_line,
        });
        let key = r.name.to_ascii_lowercase();
        if let Some(kids) = child_map.get(&key) {
            for &j in kids {
                if queued.insert(j) {
                    queue.push_back(j);
                }
            }
        }
    }
    out
}
