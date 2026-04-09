//! SQLite-backed code index (symbols, FTS, dependency edges).

mod analysis_reads;
mod convert;
mod graph_reads;
mod index_store;
mod repo_summary;
mod schema;
mod search;
mod symbol_reads;
mod symbol_record;
mod types;

use std::fs;
use std::path::{Path, PathBuf};

pub use repo_summary::RepoMetadata;
pub use symbol_record::{SymbolMetadata, SymbolRecord};
pub use types::{AnalysisSymbol, RetrievedSymbol, SearchSymbol, SymbolSourceRecord};

#[derive(Debug, Clone)]
pub struct IndexStore {
    pub(crate) db_path: PathBuf,
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
