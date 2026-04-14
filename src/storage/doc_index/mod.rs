//! SQLite storage layer for documentation sections.

mod ingest;
mod migrations;
mod query;
mod store;

pub use ingest::{DocIndexSummary, index_local_docs, index_local_docs_with_section_vectors};
pub use migrations::{DOC_INDEX_TARGET_USER_VERSION, apply_doc_index_migrations};

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use rusqlite::Connection;
use serde_json::Value;

use crate::{db, retrieval::sections::Section};

#[derive(Debug, Clone)]
pub struct DocFileUpdate {
    pub file_path: String,
    pub format: String, // stored in the `format` DB column
    pub size_bytes: u64,
    pub modified_ns: u64,
    pub content_hash: String,
    pub sections: Vec<Section>,
    pub source: String,
}

#[derive(Clone)]
pub struct DocStore {
    pub(super) db_path: PathBuf,
    pub(super) cached_total_bytes: std::sync::Arc<std::sync::OnceLock<u64>>,
    pub(super) ro_conn: std::sync::Arc<db::RoConnCache>,
}

impl std::fmt::Debug for DocStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocStore")
            .field("db_path", &self.db_path)
            .finish()
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchSection {
    pub id: String,
    pub heading: String,
    pub heading_path: String,
    pub level: u8,
    pub file: String,
    pub line: u32,
    pub summary: String,
    pub score: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RetrievedSection {
    pub id: String,
    pub heading: String,
    pub file: String,
    pub level: u8,
    pub line: u32,
    pub end_line: u32,
    pub content: String,
    pub content_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_drift: Option<bool>,
    /// Present when the client passed `verify=true`; records client hash verification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_truncated: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct SectionRecord {
    pub id: String,
    pub file_path: String,
    pub heading: String,
    pub level: u8,
    pub start_line: u32,
    pub end_line: u32,
    pub start_byte: usize,
    pub end_byte: usize,
    pub content_hash: String,
    pub content_text: String,
    pub file_size_bytes: u64,
    pub root_path: PathBuf,
}

impl DocStore {
    pub fn open(db_path: PathBuf) -> anyhow::Result<Self> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let store = Self {
            db_path,
            cached_total_bytes: Default::default(),
            ro_conn: Default::default(),
        };
        let conn = store.open_connection()?;
        apply_doc_index_migrations(&conn)?;
        Ok(store)
    }

    pub fn open_existing(db_path: PathBuf) -> anyhow::Result<Option<Self>> {
        if !db_path.exists() {
            return Ok(None);
        }
        let store = Self {
            db_path,
            cached_total_bytes: Default::default(),
            ro_conn: Default::default(),
        };
        {
            let conn = store.open_read_only()?;
            conn.query_row("SELECT COUNT(*) FROM repo_info", [], |_| Ok(()))
                .context("repo_info missing from existing doc index")?;
        }
        Ok(Some(store))
    }

    pub fn current_index_version(&self) -> anyhow::Result<u32> {
        let conn = self.open_connection()?;
        Ok(conn
            .query_row("SELECT index_version FROM repo_info LIMIT 1", [], |row| {
                row.get::<_, u32>(0)
            })
            .unwrap_or(0))
    }

    pub(super) fn open_connection(&self) -> anyhow::Result<Connection> {
        Ok(db::open_rw(&self.db_path)?)
    }

    pub(super) fn open_read_only(&self) -> anyhow::Result<db::RoConn<'_>> {
        Ok(db::open_cached_ro(&self.ro_conn, &self.db_path)?)
    }

    /// KNN over section embeddings.
    pub fn query_section_knn(&self, query: &[f32], k: usize) -> anyhow::Result<Vec<(String, f64)>> {
        use codesight_storage::storage::vector_index::{Domain, query_knn};
        let conn = self.open_read_only()?;
        Ok(query_knn(&conn, Domain::Section, query, k)?)
    }
}

pub fn list_doc_summaries(docs_root: &Path) -> anyhow::Result<Vec<Value>> {
    let mut repos = Vec::new();
    if !docs_root.exists() {
        return Ok(repos);
    }
    for entry in fs::read_dir(docs_root)? {
        let entry = entry?;
        let db_path = entry.path().join("doc_index.sqlite3");
        if let Some(store) = DocStore::open_existing(db_path)?
            && let Ok(summary) = store.doc_summary()
        {
            repos.push(summary);
        }
    }
    Ok(repos)
}
