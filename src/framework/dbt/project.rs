use std::path::{Path, PathBuf};

/// Detect `dbt_project.yml` at root or one level deep.
/// Returns the directory containing it, or `None`.
pub fn detect_dbt_project(root: &Path) -> Option<PathBuf> {
    // Root level
    if root.join("dbt_project.yml").exists() {
        return Some(root.to_path_buf());
    }

    // One level deep (immediate subdirectories, skip hidden)
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with('.') {
                    continue;
                }
                if path.join("dbt_project.yml").exists() {
                    return Some(path);
                }
            }
        }
    }

    None
}
