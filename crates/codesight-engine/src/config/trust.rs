//! Path-trust policy. Enforces that indexing cannot touch broad filesystem
//! roots unless an operator has explicitly trusted the path.
//!
//! `trusted_folders` is a whitelist: when non-empty, only paths under the
//! listed folders may be indexed; when empty, any non-broad path is allowed.

use std::path::{Path, PathBuf};

use super::app::AppConfig;

/// Path components that indicate a "broad" root (too high in the filesystem tree
/// to index safely without explicit trust).
const BROAD_ROOTS: &[&str] = &[
    "/",
    "/home",
    "/Users",
    "/root",
    "/var",
    "/tmp",
    "/etc",
    "/opt",
    "/usr",
    "/mnt",
    "/media",
    "/srv",
    "c:\\",
    "d:\\",
    "c:\\users",
    "d:\\users",
];

/// Result of checking whether a path is trusted for indexing.
#[derive(Debug, Clone)]
pub enum TrustCheckResult {
    Trusted,
    TrustedBroadRoot { warning: String },
    Untrusted { error: String },
}

pub fn is_broad_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let normalized = s.replace('\\', "/").to_lowercase();
    let trimmed = normalized.trim_end_matches('/');

    let components: Vec<&str> = trimmed.split('/').filter(|c| !c.is_empty()).collect();
    if components.len() <= 1 {
        return true;
    }

    for broad in BROAD_ROOTS {
        let b = broad.replace('\\', "/").to_lowercase();
        let b = b.trim_end_matches('/');
        if trimmed == b {
            return true;
        }
    }
    false
}

pub fn check_trust(config: &AppConfig, resolved_path: &Path) -> TrustCheckResult {
    let folders = &config.trusted_folders;

    if folders.is_empty() {
        if is_broad_path(resolved_path) {
            return TrustCheckResult::Untrusted {
                error: format!(
                    "Path '{}' is too broad to index safely. \
                     Configure trusted_folders or use a more specific path.",
                    resolved_path.display()
                ),
            };
        }
        return TrustCheckResult::Trusted;
    }

    let is_trusted = folders
        .iter()
        .any(|tf| resolved_path == tf.as_path() || resolved_path.starts_with(tf));

    if !is_trusted {
        return TrustCheckResult::Untrusted {
            error: format!(
                "Path '{}' is not under trusted_folders. \
                 Add it to trusted_folders to allow indexing.",
                resolved_path.display()
            ),
        };
    }

    if is_broad_path(resolved_path) {
        return TrustCheckResult::TrustedBroadRoot {
            warning: format!(
                "Path '{}' matched trusted_folders and was allowed, \
                 but would normally be rejected as too broad to index safely.",
                resolved_path.display()
            ),
        };
    }

    TrustCheckResult::Trusted
}

/// Validate project-config trusted_folders: reject any entry that escapes
/// the project root via `..`.
pub fn sanitize_project_trusted_folders(project_root: &Path, entries: &[PathBuf]) -> Vec<PathBuf> {
    entries
        .iter()
        .filter_map(|entry| {
            let resolved = if entry.is_relative() {
                project_root.join(entry)
            } else {
                entry.clone()
            };
            let s = resolved.to_string_lossy();
            if s.contains("..") {
                tracing::warn!(
                    path = %entry.display(),
                    "rejecting project config trusted_folder (path escape attempt)"
                );
                None
            } else {
                Some(resolved)
            }
        })
        .collect()
}
