//! Monorepo-aware manifest discovery.
//!
//! Walks a workspace root, visiting each directory and aggregating package
//! names across every supported ecosystem via `Language::package_name_for_dir`.
//! Pruning is applied eagerly via [`walkdir::IntoIter::filter_entry`] so noisy
//! subtrees (dependencies, build outputs, VCS metadata) cost O(1) instead of
//! O(subtree size).

use rustc_hash::FxHashSet;
use std::path::Path;
use std::sync::OnceLock;

use walkdir::{DirEntry, WalkDir};

/// Maximum directory depth to scan.
pub const MAX_DEPTH: usize = 12;

/// Build a sorted, deduplicated snapshot of every `Language::pruned_dirs`
/// override registered in [`crate::builtin_registry`]. VCS metadata
/// (`.git`, `.hg`, `.svn`) and the dot-prefix rule stay generic in
/// [`is_pruned`] — those aren't language-owned.
fn pruned_dirs() -> &'static [&'static str] {
    static DIRS: OnceLock<Vec<&'static str>> = OnceLock::new();
    DIRS.get_or_init(|| {
        let mut out: Vec<&'static str> = crate::builtin_registry()
            .all()
            .flat_map(|l| l.pruned_dirs().iter().copied())
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    })
}

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
    pruned_dirs().contains(&name)
}

/// Walk `root` and return every package name declared by any supported
/// manifest, deduplicated in first-seen order.
pub fn discover_package_names(root: &Path) -> Vec<String> {
    if !root.is_dir() {
        return Vec::new();
    }

    let mut seen: FxHashSet<String> = FxHashSet::default();
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
