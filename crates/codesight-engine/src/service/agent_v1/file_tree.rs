//! File tree building for inspect_structure `file_tree` view.

use std::collections::BTreeMap;

use serde_json::{Value, json};

#[derive(Debug)]
pub(in crate::service) enum FileTreeBuildNode {
    Dir(BTreeMap<String, FileTreeBuildNode>),
    File(Value),
}

pub(in crate::service) fn normalize_file_tree_path_prefix(path: &Option<String>) -> Option<String> {
    let p = path.as_ref()?.trim();
    if p.is_empty() {
        return None;
    }
    let mut s = p.replace('\\', "/");
    if !s.ends_with('/') {
        s.push('/');
    }
    Some(s)
}

pub(in crate::service) fn file_tree_insert(
    root: &mut BTreeMap<String, FileTreeBuildNode>,
    rel_path: &str,
    file_json: Value,
) {
    let segments: Vec<&str> = rel_path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return;
    }
    let (dir_parts, file_seg) = segments.split_at(segments.len() - 1);
    let file_name = file_seg[0];
    let mut map = root;
    for seg in dir_parts {
        let entry = map
            .entry((*seg).to_string())
            .or_insert_with(|| FileTreeBuildNode::Dir(BTreeMap::new()));
        match entry {
            FileTreeBuildNode::Dir(inner) => map = inner,
            FileTreeBuildNode::File(_) => return,
        }
    }
    map.insert(file_name.to_string(), FileTreeBuildNode::File(file_json));
}

pub(in crate::service) fn file_tree_to_json(
    nodes: &BTreeMap<String, FileTreeBuildNode>,
    prefix: &str,
) -> Vec<Value> {
    let mut out = Vec::new();
    for (seg, node) in nodes {
        match node {
            FileTreeBuildNode::Dir(children) => {
                let path = if prefix.is_empty() {
                    seg.clone()
                } else {
                    format!("{prefix}/{seg}")
                };
                let ch = file_tree_to_json(children, &path);
                out.push(json!({
                    "type": "dir",
                    "path": path,
                    "children": ch,
                }));
            }
            FileTreeBuildNode::File(v) => out.push(v.clone()),
        }
    }
    out
}

pub(in crate::service) fn file_tree_hint(listed: usize, total: usize, max_files: usize) -> String {
    format!(
        "file tree lists {listed} of {total} files (max_files={max_files}); narrow with path (path_prefix) or raise max_files."
    )
}
