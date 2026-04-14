//! `AppConfig`: the root settings struct handed down from `main`.

use std::path::PathBuf;

use crate::path_map::PathPair;

use super::defaults::{
    DEFAULT_DISPATCH_CONCURRENCY, DEFAULT_HANDLER_TIMEOUT_SECS, DEFAULT_MAX_FOLDER_FILES,
    DEFAULT_SHUTDOWN_DRAIN_SECS,
};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub(super) index_root: PathBuf,
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
    /// When true, read tools block (or warn) while a reindex is in flight on
    /// the same repo. When false (default), read tools proceed and attach a
    /// `stale_data_warning` to the response envelope.
    pub strict_freshness: bool,
    /// Per-handler wall-clock timeout. `0` disables the timeout.
    pub handler_timeout_secs: u64,
    /// How long a shutdown drain waits for an in-flight dispatch before
    /// checkpointing and exiting anyway.
    pub shutdown_drain_secs: u64,
    /// Maximum number of in-flight `tools/call` dispatches.
    pub dispatch_concurrency: usize,
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
            strict_freshness: false,
            handler_timeout_secs: DEFAULT_HANDLER_TIMEOUT_SECS,
            shutdown_drain_secs: DEFAULT_SHUTDOWN_DRAIN_SECS,
            dispatch_concurrency: DEFAULT_DISPATCH_CONCURRENCY,
        }
    }

    pub fn load() -> Self {
        super::loader::load_from_env_and_disk()
    }

    pub fn ensure_layout(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.repos_dir())
    }

    pub fn index_root(&self) -> &std::path::Path {
        &self.index_root
    }

    pub fn repos_dir(&self) -> PathBuf {
        self.index_root.join("repos")
    }

    pub fn savings_file(&self) -> PathBuf {
        self.index_root.join("_savings.json")
    }

    pub fn repo_dir(&self, repo: &str) -> PathBuf {
        super::paths::slug_dir(self.repos_dir(), repo)
    }

    pub fn repo_db_path(&self, repo: &str) -> PathBuf {
        self.repo_dir(repo).join("index.sqlite3")
    }

    pub fn docs_dir(&self) -> PathBuf {
        self.index_root.join("docs")
    }

    pub fn doc_dir(&self, repo: &str) -> PathBuf {
        super::paths::slug_dir(self.docs_dir(), repo)
    }

    pub fn doc_db_path(&self, repo: &str) -> PathBuf {
        self.doc_dir(repo).join("doc_index.sqlite3")
    }

    /// Passive-checkpoint every known code and doc database on disk. Used on
    /// graceful shutdown so the server does not leave fat WAL files behind.
    /// Errors are logged and swallowed — shutdown must not hang on a broken
    /// database file.
    pub fn checkpoint_all_wal(&self) {
        let roots = [self.repos_dir(), self.docs_dir()];
        for root in roots {
            let Ok(entries) = std::fs::read_dir(&root) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                for leaf in ["index.sqlite3", "doc_index.sqlite3"] {
                    let db = path.join(leaf);
                    if !db.exists() {
                        continue;
                    }
                    if let Err(error) = codesight_storage::db::wal_checkpoint_passive(&db) {
                        tracing::warn!(
                            path = %db.display(),
                            error = %error,
                            "shutdown wal checkpoint failed"
                        );
                    }
                }
            }
        }
    }
}
