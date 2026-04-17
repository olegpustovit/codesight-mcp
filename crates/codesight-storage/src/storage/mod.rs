//! SQLite-backed code index (symbols, FTS, dependency edges).

mod analysis_reads;
mod convert;
pub mod doc_schema;
mod graph_reads;
mod index_store;
pub mod migrations;
mod repo_summary;
mod schema;
mod search;
mod snapshot_store;
mod symbol_reads;
mod symbol_record;
mod types;
pub mod vector_index;

use std::fs;
use std::path::{Path, PathBuf};

pub use repo_summary::RepoMetadata;
pub use schema::EMBEDDING_DIM;
pub use snapshot_store::QualitySnapshot;
pub use symbol_reads::is_outline_noise;
pub use symbol_record::{SymbolMetadata, SymbolRecord};
pub use types::{AnalysisSymbol, RetrievedSymbol, SearchSymbol, SymbolSourceRecord};

#[derive(Clone)]
pub struct IndexStore {
    pub(crate) db_path: PathBuf,
    pub(crate) cached_total_bytes: std::sync::Arc<std::sync::OnceLock<u64>>,
    pub(crate) cached_root_path: std::sync::Arc<std::sync::OnceLock<PathBuf>>,
    pub(crate) ro_conn: std::sync::Arc<crate::db::RoConnCache>,
}

impl std::fmt::Debug for IndexStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexStore")
            .field("db_path", &self.db_path)
            .finish()
    }
}

pub fn list_repo_summaries(index_root: &Path) -> anyhow::Result<Vec<RepoMetadata>> {
    let mut repos = Vec::new();
    if !index_root.exists() {
        return Ok(repos);
    }

    for entry in fs::read_dir(index_root)? {
        let entry = entry?;
        let db_path = entry.path().join("index.sqlite3");
        if let Some(store) = IndexStore::open_existing(db_path)?
            && let Ok(summary) = store.repo_summary()
        {
            repos.push(summary);
        }
    }
    repos.sort_by(|left, right| left.repo.cmp(&right.repo));
    Ok(repos)
}
