use std::path::PathBuf;

use crate::path_map::PathPair;

pub const BYTES_PER_TOKEN: u64 = 4;
pub const RETRIEVAL_VERSION: &str = "agent-v1";
pub const RETRIEVAL_ENGINE: &str = "codesight";
pub const SCHEMA_VERSION: u32 = 1;

/// Threshold for emitting a "No .gitignore" warning during ingest.
pub const GITIGNORE_WARNING_FILE_THRESHOLD: usize = 500;

/// Maximum number of files to index from a local folder (0 = unlimited).
pub const DEFAULT_MAX_FOLDER_FILES: usize = 0;

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
    // Windows-style broad roots (lower-cased for comparison)
    "c:\\",
    "d:\\",
    "c:\\users",
    "d:\\users",
];

#[derive(Debug, Clone)]
pub struct AppConfig {
    index_root: PathBuf,
    pub auto_reindex: bool,
    /// Whitelist mode (default): only paths under these folders can be indexed.
    /// When empty and whitelist mode, all non-broad paths are allowed.
    pub trusted_folders: Vec<PathBuf>,
    /// When true (default), `trusted_folders` is a whitelist: only listed paths are trusted.
    /// When false, `trusted_folders` is a blacklist: listed paths are untrusted.
    pub trusted_folders_whitelist_mode: bool,
    /// Maximum number of files to discover from a local folder (0 = unlimited).
    pub max_folder_files: usize,
    /// Path-prefix remapping pairs from `CODESIGHT_PATH_MAP`.
    pub path_map: Vec<PathPair>,
    /// Default value for `cross_repo` parameter on inspect_graph modes.
    pub cross_repo_default: bool,
}

impl AppConfig {
    pub fn new(index_root: PathBuf) -> Self {
        Self {
            index_root,
            auto_reindex: true,
            trusted_folders: Vec::new(),
            trusted_folders_whitelist_mode: true,
            max_folder_files: DEFAULT_MAX_FOLDER_FILES,
            path_map: Vec::new(),
            cross_repo_default: false,
        }
    }

    pub fn load() -> Self {
        let auto_reindex = std::env::var("CODESIGHT_AUTO_REINDEX")
            .map(|v| v != "false" && v != "0")
            .unwrap_or(true);
        let index_root = std::env::var_os("CODESIGHT_INDEX_DIR")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|home| home.join(".code-index").join("codesight")))
            .unwrap_or_else(|| PathBuf::from(".code-index").join("codesight"));

        let max_folder_files = std::env::var("CODESIGHT_MAX_FOLDER_FILES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MAX_FOLDER_FILES);

        let path_map = crate::path_map::parse_path_map_from_env();

        let cross_repo_default = std::env::var("CODESIGHT_CROSS_REPO_DEFAULT")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let mut config = Self::new(index_root);
        config.auto_reindex = auto_reindex;
        config.max_folder_files = max_folder_files;
        config.path_map = path_map;
        config.cross_repo_default = cross_repo_default;
        config
    }

    pub fn ensure_layout(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.repos_dir())
    }

    pub fn repos_dir(&self) -> PathBuf {
        self.index_root.join("repos")
    }

    pub fn savings_file(&self) -> PathBuf {
        self.index_root.join("_savings.json")
    }

    pub fn repo_dir(&self, repo: &str) -> PathBuf {
        slug_dir(self.repos_dir(), repo)
    }

    pub fn repo_db_path(&self, repo: &str) -> PathBuf {
        self.repo_dir(repo).join("index.sqlite3")
    }

    pub fn docs_dir(&self) -> PathBuf {
        self.index_root.join("docs")
    }

    pub fn doc_dir(&self, repo: &str) -> PathBuf {
        slug_dir(self.docs_dir(), repo)
    }

    pub fn doc_db_path(&self, repo: &str) -> PathBuf {
        self.doc_dir(repo).join("doc_index.sqlite3")
    }

    pub fn embedding_path(&self, repo: &str) -> PathBuf {
        self.repo_dir(repo).join("embeddings.json")
    }
}

// ── Trust checking ──────────────────────────────────────────────────────────

/// Result of checking whether a path is trusted for indexing.
#[derive(Debug, Clone)]
pub enum TrustCheckResult {
    /// Path is trusted and may be indexed.
    Trusted,
    /// Path is trusted but is a broad root — proceed with a warning.
    TrustedBroadRoot { warning: String },
    /// Path is not trusted — return an error.
    Untrusted { error: String },
}

/// Returns `true` if `path` looks like a broad filesystem root that should not
/// be indexed without explicit trust.
pub fn is_broad_path(path: &std::path::Path) -> bool {
    let s = path.to_string_lossy();
    let normalized = s.replace('\\', "/").to_lowercase();
    let trimmed = normalized.trim_end_matches('/');

    // A path with <= 2 components (like "/" or "/home" or "/Users") is broad.
    let components: Vec<&str> = trimmed.split('/').filter(|c| !c.is_empty()).collect();
    if components.len() <= 1 {
        return true;
    }

    // Check against known broad roots
    for broad in BROAD_ROOTS {
        let b = broad.replace('\\', "/").to_lowercase();
        let b = b.trim_end_matches('/');
        if trimmed == b {
            return true;
        }
    }
    false
}

/// Check whether `resolved_path` is trusted for indexing given the config.
///
/// This implements the same behavioral contract as the Python `_is_trusted` +
/// broad-path logic from historical trust-path tests.
pub fn check_trust(config: &AppConfig, resolved_path: &std::path::Path) -> TrustCheckResult {
    let folders = &config.trusted_folders;
    let whitelist = config.trusted_folders_whitelist_mode;

    if whitelist {
        // ── Whitelist mode ──
        if folders.is_empty() {
            // No trusted folders configured → allow non-broad paths.
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

        // Check if the path is exactly one of the trusted folders or a descendant.
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

        // Path is trusted. If it is also broad, emit a warning.
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
    } else {
        // ── Blacklist mode ──
        if folders.is_empty() {
            return TrustCheckResult::Untrusted {
                error: format!(
                    "trusted_folders_whitelist_mode is False (blacklist mode) \
                     but trusted_folders list is empty. \
                     In blacklist mode you must list at least one untrusted path."
                ),
            };
        }

        // In blacklist mode, listed paths are *untrusted*.
        let is_blacklisted = folders
            .iter()
            .any(|tf| resolved_path == tf.as_path() || resolved_path.starts_with(tf));

        if is_blacklisted {
            return TrustCheckResult::Untrusted {
                error: format!(
                    "Path '{}' is not under trusted_folders (blacklisted).",
                    resolved_path.display()
                ),
            };
        }

        // Not blacklisted — trusted. Check broad-path for warning.
        if is_broad_path(resolved_path) {
            return TrustCheckResult::TrustedBroadRoot {
                warning: format!(
                    "Path '{}' is not blacklisted and was allowed, \
                     but would normally be rejected as too broad to index safely.",
                    resolved_path.display()
                ),
            };
        }

        TrustCheckResult::Trusted
    }
}

/// Validate project-config trusted_folders: reject any entry that escapes
/// the project root via `..`.
pub fn sanitize_project_trusted_folders(
    project_root: &std::path::Path,
    entries: &[PathBuf],
) -> Vec<PathBuf> {
    entries
        .iter()
        .filter_map(|entry| {
            let resolved = if entry.is_relative() {
                project_root.join(entry)
            } else {
                entry.clone()
            };
            // Normalize by canonicalizing or at least checking for ..
            let s = resolved.to_string_lossy();
            if s.contains("..") {
                eprintln!(
                    "codesight-mcp: rejecting project config trusted_folder '{}' \
                     (path escape attempt)",
                    entry.display()
                );
                None
            } else {
                Some(resolved)
            }
        })
        .collect()
}

fn slug_dir(base: PathBuf, repo: &str) -> PathBuf {
    let sanitized = repo
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' => ch,
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned();
    let digest = blake3::hash(repo.as_bytes()).to_hex().to_string();
    base.join(format!("{sanitized}-{}", &digest[..12]))
}
