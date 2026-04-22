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

/// Extract symbol names and their source text from raw content using the
/// tree-sitter indexer. Used by git-diff-driven analyses that feed in content
/// from `git show`, so we can't rely on on-disk reads.
pub(in crate::service) fn extract_symbol_names_from_content(
    content: &str,
    file_path: &str,
    project_root: &std::path::Path,
    indexer: &std::sync::Arc<crate::indexing::Indexer>,
) -> Vec<(String, String)> {
    let abs = project_root.join(file_path);
    indexer
        .extract_symbols(project_root, &abs, content)
        .into_iter()
        .map(|s| (s.name, s.source_text))
        .collect()
}

/// BFS once from `target_id` across reverse call edges, returning a
/// `caller_id → parent_caller_id` map rooted at the target. Cheap chain
/// reconstruction for any caller is then an O(path_len) walk via
/// [`reconstruct_caller_chain`].
///
/// Replaces a per-caller BFS that rebuilt the reverse edge map and
/// cloned the entire path vector on every queue push — the combination
/// timed out `evaluate_impact analysis=impact_preview` for high-fan-in
/// targets (`search_catalog`, etc.) against the 2000 ms handler cap.
pub(in crate::service) fn build_caller_parents(
    target_id: &str,
    edges: &[codesight_analysis::graph::CallEdge],
) -> std::collections::HashMap<String, String> {
    use std::collections::{HashMap, VecDeque};
    let mut callee_to_callers: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in edges {
        callee_to_callers
            .entry(edge.callee_id.as_str())
            .or_default()
            .push(edge.caller_id.as_str());
    }
    let mut parents: HashMap<String, String> = HashMap::new();
    let mut visited: HashSet<&str> = HashSet::new();
    visited.insert(target_id);
    let mut queue: VecDeque<&str> = VecDeque::new();
    queue.push_back(target_id);
    while let Some(current) = queue.pop_front() {
        if let Some(callers) = callee_to_callers.get(current) {
            for &c in callers {
                if visited.insert(c) {
                    parents.insert(c.to_string(), current.to_string());
                    queue.push_back(c);
                }
            }
        }
    }
    parents
}

/// Walk the precomputed parent map back from `caller_id` to `target_id`
/// to reconstruct the call chain. Falls back to `[target, caller]` when
/// the caller is not reachable in the parent map (unchanged semantics).
pub(in crate::service) fn reconstruct_caller_chain(
    target_id: &str,
    caller_id: &str,
    parents: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    if caller_id == target_id {
        return vec![target_id.to_string()];
    }
    let mut chain = vec![caller_id.to_string()];
    let mut current = caller_id.to_string();
    while let Some(parent) = parents.get(&current) {
        chain.push(parent.clone());
        if parent == target_id {
            chain.reverse();
            return chain;
        }
        current = parent.clone();
    }
    vec![target_id.to_string(), caller_id.to_string()]
}

pub(in crate::service) fn simple_text_diff(a: &str, b: &str) -> Value {
    use similar::{ChangeTag, DiffOp, TextDiff};

    let diff = TextDiff::from_lines(a, b);
    let common_prefix_lines = match diff.ops().first() {
        Some(DiffOp::Equal { len, .. }) => *len,
        _ => 0,
    };
    let mut removed_tail: Vec<String> = Vec::new();
    let mut added_tail: Vec<String> = Vec::new();
    for change in diff.iter_all_changes() {
        let line = change.value().trim_end_matches('\n').to_string();
        match change.tag() {
            ChangeTag::Delete => removed_tail.push(line),
            ChangeTag::Insert => added_tail.push(line),
            ChangeTag::Equal => {}
        }
    }
    json!({
        "common_prefix_lines": common_prefix_lines,
        "removed_tail": removed_tail,
        "added_tail": added_tail,
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
