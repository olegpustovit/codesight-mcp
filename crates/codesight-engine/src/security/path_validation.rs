use std::fs;
use std::path::Path;

/// Returns `true` if `path` is logically contained within `root`.
///
/// Both paths are canonicalized (resolving `..`, symlinks, etc.) before
/// comparison, so `root/a/../../etc/passwd` is correctly rejected.
/// If either path cannot be canonicalized the result is `false`.
pub fn validate_path(root: &Path, path: &Path) -> bool {
    let Ok(canon_root) = fs::canonicalize(root) else {
        return false;
    };
    let Ok(canon_path) = fs::canonicalize(path) else {
        if path == root {
            return true;
        }
        if let Some(parent) = path.parent()
            && let Ok(canon_parent) = fs::canonicalize(parent)
        {
            let full = canon_parent.join(path.file_name().unwrap_or_default());
            return full.starts_with(&canon_root);
        }
        return false;
    };
    canon_path.starts_with(&canon_root)
}

/// Returns `true` if `path` is a symlink whose target resolves outside `root`.
///
/// Non-symlinks always return `false`.
pub fn is_symlink_escape(root: &Path, path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.file_type().is_symlink() {
        return false;
    }
    let Ok(canon_root) = fs::canonicalize(root) else {
        return true;
    };
    let Ok(canon_target) = fs::canonicalize(path) else {
        return true;
    };
    !canon_target.starts_with(&canon_root)
}
