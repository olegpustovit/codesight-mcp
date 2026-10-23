//! Per-language path-rename preview engine.
//!
//! The trait + struct shapes here are what `dowser-analysis` consumes via
//! [`crate::traits::Language::path_rename_engine`]. The Rust implementation
//! lives at [`crate::languages::rust::path_rename`]; future languages plug in
//! by returning `Some(&MY_ENGINE)` from the trait default-override.

use std::path::Path;

use serde::Serialize;

/// Per-file preview row: path + ordered change list + the rewritten source
/// kept privately so callers can `fs::write` without re-running the scanner.
#[derive(Debug, Clone, Serialize)]
pub struct PathRenameFilePlan {
    pub path: String,
    pub changes: Vec<PathRenameChange>,
    #[serde(skip_serializing)]
    pub updated_source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathRenameChange {
    pub line: u32,
    pub col: u32,
    pub before: String,
    pub after: String,
    pub node_kind: String,
}

/// Location that the engine chose not to rewrite (e.g. partial grouped
/// imports) with a human-readable reason.
#[derive(Debug, Clone, Serialize)]
pub struct PathRenameUnhandled {
    pub path: String,
    pub line: u32,
    pub reason: String,
}

/// Post-rewrite collision (e.g. two identical `use` lines after the swap).
#[derive(Debug, Clone, Serialize)]
pub struct PathRenameCollision {
    pub path: String,
    pub line: u32,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct PathRenamePlan {
    pub files_affected: Vec<PathRenameFilePlan>,
    pub unhandled: Vec<PathRenameUnhandled>,
    pub collisions: Vec<PathRenameCollision>,
}

/// A `Language`-owned engine that previews a dotted/scoped path rewrite
/// (`from` → `to`) across the workspace. `scope` restricts the walk to a
/// repo-relative prefix; `partial_mode` is `"warn"` / `"skip"` / `"split"`
/// and controls handling of grouped imports that only partially match.
pub trait PathRenameEngine: Send + Sync {
    fn preview(
        &self,
        root: &Path,
        from: &str,
        to: &str,
        scope: Option<&str>,
        partial_mode: &str,
    ) -> anyhow::Result<PathRenamePlan>;
}

/// Derived summary: (files affected, total changes).
pub fn summarize(plan: &PathRenamePlan) -> (usize, usize) {
    let files = plan.files_affected.len();
    let changes = plan
        .files_affected
        .iter()
        .map(|f| f.changes.len())
        .sum::<usize>();
    (files, changes)
}

/// Sorted repo-relative paths touched by `plan`.
pub fn affected_paths(plan: &PathRenamePlan) -> std::collections::BTreeSet<String> {
    plan.files_affected.iter().map(|f| f.path.clone()).collect()
}
