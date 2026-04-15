//! Monorepo-aware manifest discovery.
//!
//! Walks a workspace root, visiting each directory and aggregating package
//! names across every supported ecosystem. Pruning is applied eagerly via
//! [`walkdir::IntoIter::filter_entry`] so noisy subtrees (dependencies, build
//! outputs, VCS metadata) cost O(1) instead of O(subtree size).
//!
//! ## Pruning policy
//!
//! An entry is pruned when any of the following holds at depth > 0:
//! - directory name matches [`PRUNED_DIR_NAMES`] (build outputs, vendored
//!   dependencies, virtualenvs, VCS metadata)
//! - directory name starts with `.` (covers `.git`, `.venv`, `.cache`,
//!   `.next`, `.nuxt`, `.output`, `.idea`, `.github`, …)
//! - entry depth would exceed [`MAX_DEPTH`]
//!
//! Symlinks are never followed, which makes cycles and escape-to-outside
//! symlinks inert.
//!
//! ## Ordering and deduplication
//!
//! Walkdir entries are sorted by filename so the scan order is deterministic
//! across platforms. Package names are deduplicated through a `HashSet`
//! while the accumulator preserves first-seen order.

use std::collections::HashSet;
use std::path::Path;

use walkdir::{DirEntry, WalkDir};

/// Maximum directory depth to scan. Real-world monorepo manifests almost
/// never sit below 4–6 levels; this cap protects against adversarial or
/// cyclic trees without constraining legitimate layouts.
pub const MAX_DEPTH: usize = 12;

/// Directories that never contain first-party manifests and are always
/// pruned during discovery. Kept local to the walker because this policy is
/// discovery-specific and deliberately more aggressive than the broader
/// file-indexing skip list in `db::SKIP_DIRS`.
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
///
/// Returns an empty vector if `root` does not exist or is not a directory.
/// Walk errors (permission denied, unreadable entries) are silently dropped
/// so discovery degrades gracefully on partially accessible trees.
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

    for entry in walker.flatten() {
        if !entry.file_type().is_dir() {
            continue;
        }
        let dir = entry.path();
        let names = [
            super::python::read_package_names(dir),
            super::javascript::read_package_names(dir),
            super::go::read_package_names(dir),
            super::rust::read_package_names(dir),
            super::dotnet::read_package_names(dir),
        ]
        .concat();
        for name in names {
            if seen.insert(name.clone()) {
                ordered.push(name);
            }
        }
    }

    ordered
}
