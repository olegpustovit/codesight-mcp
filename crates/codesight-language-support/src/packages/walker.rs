//! Monorepo-aware manifest discovery.
//!
//! Walks a workspace root, visiting each directory and aggregating package
//! names across every supported ecosystem via `Language::package_name_for_dir`.
//! Pruning is applied eagerly via [`walkdir::IntoIter::filter_entry`] so noisy
//! subtrees (dependencies, build outputs, VCS metadata) cost O(1) instead of
//! O(subtree size).

use std::collections::HashSet;
use std::path::Path;

use walkdir::{DirEntry, WalkDir};

/// Maximum directory depth to scan.
pub const MAX_DEPTH: usize = 12;

const PRUNED_DIR_NAMES: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "bin",
    "obj",
    "vendor",
    "__pycache__",
    "venv",
    ".venv",
    ".tox",
    ".git",
    ".hg",
    ".svn",
];

fn is_pruned(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return false;
    }
    if !entry.file_type().is_dir() {
        return false;
    }
    let Some(name) = entry.file_name().to_str() else {
        return true;
    };
    if name.starts_with('.') {
        return true;
    }
    PRUNED_DIR_NAMES.contains(&name)
}

/// Walk `root` and return every package name declared by any supported
/// manifest, deduplicated in first-seen order.
pub fn discover_package_names(root: &Path) -> Vec<String> {
    if !root.is_dir() {
        return Vec::new();
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut ordered: Vec<String> = Vec::new();

    let walker = WalkDir::new(root)
        .follow_links(false)
        .max_depth(MAX_DEPTH)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| !is_pruned(entry));

    let registry = crate::builtin_registry();
    for entry in walker.flatten() {
        if !entry.file_type().is_dir() {
            continue;
        }
        let dir = entry.path();
        for lang in registry.all() {
            if let Some(name) = lang.package_name_for_dir(dir)
                && seen.insert(name.clone())
            {
                ordered.push(name);
            }
        }
    }

    ordered
}
