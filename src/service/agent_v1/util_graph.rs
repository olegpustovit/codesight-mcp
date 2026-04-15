//! Graph / diff helpers for inspect_graph and impact.

use std::collections::HashSet;

use serde_json::{Value, json};

pub(in crate::service) fn graph_ref_symbol_id(v: Option<&Value>) -> anyhow::Result<String> {
    let Some(obj) = v.and_then(|x| x.as_object()) else {
        anyhow::bail!("ref object required");
    };
    let id = obj
        .get("stable_id")
        .or_else(|| obj.get("id"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    let Some(sym_id) = id else {
        anyhow::bail!("ref.stable_id or ref.id required");
    };
    Ok(sym_id)
}

/// Get HEAD SHA from a git repo.
pub(in crate::service) fn git_head_sha(root: &std::path::Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

/// Show a file at a specific git SHA.
pub(in crate::service) fn git_show_file(
    root: &std::path::Path,
    sha: &str,
    file_path: &str,
) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["show", &format!("{sha}:{file_path}")])
        .current_dir(root)
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        None
    }
}

/// Extract symbol names and their source text from raw content using the extractor registry.
pub(in crate::service) fn extract_symbol_names_from_content(
    content: &str,
    file_path: &str,
    extractors: &crate::extractors::ExtractorRegistry,
) -> Vec<(String, String)> {
    let path = std::path::Path::new(file_path);
    let extractor = match extractors.extractor_for_path(path) {
        Some(e) => e,
        None => return Vec::new(),
    };
    match extractor.extract(file_path, content) {
        Ok(symbols) => symbols
            .into_iter()
            .map(|s| (s.name, s.source_text))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Build a call chain from target_id to caller_id using BFS on call edges.
pub(in crate::service) fn build_call_chain(
    target_id: &str,
    caller_id: &str,
    edges: &[codesight_analysis::graph::CallEdge],
) -> Vec<String> {
    use std::collections::{HashMap, VecDeque};
    // Build reverse edge map: callee -> callers
    let mut callee_to_callers: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in edges {
        callee_to_callers
            .entry(edge.callee_id.as_str())
            .or_default()
            .push(edge.caller_id.as_str());
    }
    // BFS from target_id to caller_id through callers
    let mut queue: VecDeque<Vec<String>> = VecDeque::new();
    queue.push_back(vec![target_id.to_string()]);
    let mut visited = HashSet::new();
    visited.insert(target_id.to_string());
    while let Some(path) = queue.pop_front() {
        let Some(last) = path.last() else {
            continue;
        };
        let current = last.clone();
        if current == caller_id {
            return path;
        }
        if let Some(callers) = callee_to_callers.get(current.as_str()) {
            for &c in callers {
                if visited.insert(c.to_string()) {
                    let mut new_path = path.clone();
                    new_path.push(c.to_string());
                    queue.push_back(new_path);
                }
            }
        }
    }
    // Fallback: just [target, caller]
    vec![target_id.to_string(), caller_id.to_string()]
}

pub(in crate::service) fn simple_text_diff(a: &str, b: &str) -> Value {
    let la: Vec<_> = a.lines().collect();
    let lb: Vec<_> = b.lines().collect();
    let mut i = 0usize;
    while i < la.len() && i < lb.len() && la[i] == lb[i] {
        i += 1;
    }
    json!({
        "common_prefix_lines": i,
        "removed_tail": la.get(i..).unwrap_or(&[]),
        "added_tail": lb.get(i..).unwrap_or(&[]),
    })
}

pub(in crate::service) fn build_toc_tree(flat: &[Value]) -> Value {
    use std::collections::BTreeMap;
    let mut by_file: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for item in flat {
        let file = item
            .get("file")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        by_file.entry(file).or_default().push(item.clone());
    }
    serde_json::to_value(by_file).unwrap_or(json!({}))
}
