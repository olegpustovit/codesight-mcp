//! `AppConfig`: the root settings struct handed down from `main`.

use std::path::PathBuf;

use crate::path_map::PathPair;

use super::env;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub(super) index_root: PathBuf,
    /// Whitelist of folders allowed to be indexed. When empty, all non-broad
    /// paths are allowed.
    pub trusted_folders: Vec<PathBuf>,
    /// Path-prefix remapping pairs from `CODESIGHT_PATH_MAP`.
    pub path_map: Vec<PathPair>,
}

fn default_index_root() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".code-index").join("codesight"))
        .unwrap_or_else(|| PathBuf::from(".code-index").join("codesight"))
}

impl AppConfig {
    pub fn new(index_root: PathBuf) -> Self {
        Self {
            index_root,
            trusted_folders: Vec::new(),
            path_map: Vec::new(),
        }
    }

    pub fn load() -> Self {
        let index_root = env::opt_path(env::INDEX_DIR).unwrap_or_else(default_index_root);
        let mut config = Self::new(index_root);
        config.path_map = crate::path_map::parse_path_map_from_env();
        config
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
