//! Opening the index, applying writes, and repo-level metadata.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use rusqlite::{Connection, Transaction, params};
use tracing::instrument;

use super::RepoMetadata;
use crate::{
    config::SCHEMA_VERSION,
    core::pipeline::{FileFingerprint, IndexedFileUpdate, StoredSymbol},
    db,
};
use codesight_core::reachability::StructuralEdge;
use std::collections::HashMap;

use super::IndexStore;
use super::convert::{to_i64_u64, to_i64_usize};
use super::migrations;
use crate::core::symbols::FactQuality;

impl IndexStore {
    #[instrument(level = "debug", name = "index_open", skip_all, fields(path = %db_path.display(), kind = "code_index_open"))]
    pub fn open(db_path: PathBuf) -> anyhow::Result<Self> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let store = Self {
            db_path,
            cached_total_bytes: Default::default(),
            cached_root_path: Default::default(),
            ro_conn: Default::default(),
            cached_all_symbols: Default::default(),
        };
        let conn = db::open_rw(&store.db_path)?;
        migrations::apply_code_index_migrations(&conn)?;
        Ok(store)
    }

    pub fn open_existing(db_path: PathBuf) -> anyhow::Result<Option<Self>> {
        if !db_path.exists() {
            return Ok(None);
        }
        let store = Self {
            db_path,
            cached_total_bytes: Default::default(),
            cached_root_path: Default::default(),
            ro_conn: Default::default(),
            cached_all_symbols: Default::default(),
        };
        {
            let conn = store.open_read_only()?;
            conn.query_row("SELECT COUNT(*) FROM repo_info", [], |_| Ok(()))
                .context("repo_info missing from existing index")?;
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

    /// Returns the extractor-version stamp stored in `repo_info`, or an
    /// empty string if the index has not been written yet or predates the
    /// column. Callers compare this against the current build stamp to
    /// decide whether to invalidate cached file fingerprints.
    pub fn stored_extractor_version(&self) -> anyhow::Result<String> {
        let conn = self.open_connection()?;
        Ok(conn
            .query_row(
                "SELECT extractor_version FROM repo_info LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_default())
    }

    pub fn file_fingerprints(
        &self,
    ) -> anyhow::Result<std::collections::HashMap<String, FileFingerprint>> {
        let conn = self.open_connection()?;
        let mut stmt = conn.prepare_cached(
            "SELECT file_path, language, size_bytes, modified_ns, content_hash FROM files",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(FileFingerprint {
                file_path: row.get(0)?,
                language: codesight_core::LanguageId::intern(&row.get::<_, String>(1)?),
                size_bytes: row.get::<_, i64>(2)? as u64,
                modified_ns: row.get::<_, i64>(3)? as u64,
                content_hash: row.get(4)?,
            })
        })?;

        let mut items = HashMap::new();
        for row in rows {
            let fingerprint = row?;
            items.insert(fingerprint.file_path.clone(), fingerprint);
        }
        Ok(items)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_code_index(
        &self,
        repo: &str,
        root_path: &std::path::Path,
        indexed_at: &str,
        index_version: u32,
        touched_files: &[FileFingerprint],
        updated_files: &[IndexedFileUpdate],
        removed_files: &[String],
        symbol_vectors: &[super::vector_index::OwnedVectorRow],
    ) -> anyhow::Result<()> {
        self.apply_index_with_deps(
            repo,
            root_path,
            indexed_at,
            index_version,
            touched_files,
            updated_files,
            removed_files,
            symbol_vectors,
            &[],
            &[],
            "",
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_index_with_deps(
        &self,
        repo: &str,
        root_path: &std::path::Path,
        indexed_at: &str,
        index_version: u32,
        touched_files: &[FileFingerprint],
        updated_files: &[IndexedFileUpdate],
        removed_files: &[String],
        symbol_vectors: &[super::vector_index::OwnedVectorRow],
        dep_edges: &[(String, String, FactQuality)],
        reachability_edges: &[(String, Vec<StructuralEdge>)],
        extractor_version: &str,
    ) -> anyhow::Result<()> {
        let mut conn = self.open_connection()?;
        let tx = conn.transaction()?;
        Self::delete_removed_files(&tx, removed_files)?;
        Self::touch_files(&tx, touched_files)?;
        for file in updated_files {
            Self::replace_file(&tx, file)?;
        }
        Self::upsert_symbol_vectors(&tx, symbol_vectors)?;
        Self::replace_dep_edges_for_updated_files(&tx, updated_files, dep_edges)?;
        Self::replace_reachability_edges_for_updated_files(&tx, reachability_edges)?;
        Self::reset_repo_info(
            &tx,
            repo,
            root_path,
            indexed_at,
            index_version,
            extractor_version,
        )?;
        tx.commit()?;
        // Symbols may have shifted under us — drop the cached snapshot so the
        // next analytic tool rebuilds from the new commit.
        if let Ok(mut guard) = self.cached_all_symbols.lock() {
            *guard = None;
        }
        Ok(())
    }

    fn replace_reachability_edges_for_updated_files(
        tx: &Transaction<'_>,
        edges_per_file: &[(String, Vec<StructuralEdge>)],
    ) -> anyhow::Result<()> {
        for (source_file, edges) in edges_per_file {
            super::reachability_edges::replace_edges_for_file(tx, source_file, edges)?;
        }
        Ok(())
    }

    fn upsert_symbol_vectors(
        tx: &Transaction<'_>,
        symbol_vectors: &[super::vector_index::OwnedVectorRow],
    ) -> anyhow::Result<()> {
        if symbol_vectors.is_empty() {
            return Ok(());
        }
        let rows: Vec<_> = symbol_vectors
            .iter()
            .map(super::vector_index::OwnedVectorRow::borrowed)
            .collect();
        super::vector_index::upsert_vectors(tx, super::vector_index::Domain::Symbol, &rows)?;
        Ok(())
    }

    fn replace_dep_edges_for_updated_files(
        tx: &Transaction<'_>,
        updated_files: &[IndexedFileUpdate],
        dep_edges: &[(String, String, FactQuality)],
    ) -> anyhow::Result<()> {
        let mut del = tx.prepare_cached("DELETE FROM dependency_edges WHERE source_file = ?1")?;
        for file in updated_files {
            del.execute([&file.file_path])?;
        }
        drop(del);
        let mut ins = tx.prepare_cached(
            "INSERT INTO dependency_edges (source_file, target_file, import_fact_quality) VALUES (?1, ?2, ?3)
             ON CONFLICT(source_file, target_file) DO UPDATE SET import_fact_quality = excluded.import_fact_quality",
        )?;
        for (source, target, quality) in dep_edges {
            ins.execute(params![source, target, quality.as_str()])?;
        }
        Ok(())
    }

    fn reset_repo_info(
        tx: &Transaction<'_>,
        repo: &str,
        root_path: &std::path::Path,
        indexed_at: &str,
        index_version: u32,
        extractor_version: &str,
    ) -> anyhow::Result<()> {
        tx.execute("DELETE FROM repo_info", [])?;
        tx.execute(
            "INSERT INTO repo_info (repo, root_path, indexed_at, index_version, schema_version, extractor_version) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                repo,
                root_path.to_string_lossy().to_string(),
                indexed_at,
                index_version,
                SCHEMA_VERSION,
                extractor_version,
            ],
        )?;
        Ok(())
    }

    pub fn repo_summary(&self) -> anyhow::Result<RepoMetadata> {
        let conn = self.open_read_only()?;
        let (repo, indexed_at, index_version): (String, String, u32) = conn.query_row(
            "SELECT repo, indexed_at, index_version FROM repo_info LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let total_bytes = self.repo_total_bytes_with(&conn)?;
        Ok(RepoMetadata {
            repo,
            indexed_at,
            symbol_count: self.symbol_count_with(&conn)?,
            file_count: self.file_count_with(&conn)?,
            languages: self.languages_with(&conn)?,
            index_version,
            estimated_naive_tokens: total_bytes / crate::config::BYTES_PER_TOKEN,
            index_size_bytes: 0,
            is_stale: None,
        })
    }

    pub fn repo_metadata(&self) -> anyhow::Result<RepoMetadata> {
        let mut meta = self.repo_summary()?;
        meta.index_size_bytes = fs::metadata(&self.db_path)
            .map(|m| m.len())
            .unwrap_or_default();
        Ok(meta)
    }

    pub fn repo_total_bytes(&self) -> anyhow::Result<u64> {
        if let Some(&bytes) = self.cached_total_bytes.get() {
            return Ok(bytes);
        }
        let conn = self.open_read_only()?;
        self.repo_total_bytes_with(&conn)
    }

    pub(crate) fn repo_total_bytes_with(&self, conn: &Connection) -> anyhow::Result<u64> {
        if let Some(&bytes) = self.cached_total_bytes.get() {
            return Ok(bytes);
        }
        let bytes = conn.query_row(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM files",
            [],
            |row| row.get::<_, i64>(0),
        )? as u64;
        let _ = self.cached_total_bytes.set(bytes);
        Ok(bytes)
    }

    pub fn root_path(&self) -> anyhow::Result<PathBuf> {
        if let Some(cached) = self.cached_root_path.get() {
            return Ok(cached.clone());
        }
        let conn = self.open_read_only()?;
        let value: String =
            conn.query_row("SELECT root_path FROM repo_info LIMIT 1", [], |row| {
                row.get(0)
            })?;
        let path = PathBuf::from(value);
        let _ = self.cached_root_path.set(path.clone());
        Ok(path)
    }

    /// KNN over code-symbol embeddings. Returns `(stable_id, distance)` pairs.
    pub fn query_symbol_knn(&self, query: &[f32], k: usize) -> anyhow::Result<Vec<(String, f64)>> {
        let conn = self.open_read_only()?;
        Ok(super::vector_index::query_knn(
            &conn,
            super::vector_index::Domain::Symbol,
            query,
            k,
        )?)
    }

    pub(crate) fn open_connection(&self) -> anyhow::Result<Connection> {
        let conn = db::open_rw(&self.db_path)?;
        migrations::apply_code_index_migrations(&conn)?;
        Ok(conn)
    }

    pub(crate) fn open_read_only(&self) -> anyhow::Result<db::RoConn<'_>> {
        Ok(db::open_cached_ro(&self.ro_conn, &self.db_path)?)
    }

    pub(crate) fn delete_removed_files(
        tx: &Transaction<'_>,
        removed_files: &[String],
    ) -> anyhow::Result<()> {
        if removed_files.is_empty() {
            return Ok(());
        }
        let mut del_symbols = tx.prepare_cached("DELETE FROM symbols WHERE file_path = ?1")?;
        let mut del_deps = tx.prepare_cached(
            "DELETE FROM dependency_edges WHERE source_file = ?1 OR target_file = ?1",
        )?;
        let mut del_reach = tx.prepare_cached(
            "DELETE FROM reachability_edges WHERE source_file = ?1 OR target_file = ?1",
        )?;
        let mut del_files = tx.prepare_cached("DELETE FROM files WHERE file_path = ?1")?;
        for file_path in removed_files {
            Self::delete_symbol_vectors_for_file(tx, file_path)?;
            del_symbols.execute([file_path])?;
            del_deps.execute([file_path])?;
            del_reach.execute([file_path])?;
            del_files.execute([file_path])?;
        }
        Ok(())
    }

    pub(crate) fn touch_files(
        tx: &Transaction<'_>,
        touched_files: &[FileFingerprint],
    ) -> anyhow::Result<()> {
        if touched_files.is_empty() {
            return Ok(());
        }
        let mut stmt = tx.prepare_cached(
            "UPDATE files SET language = ?1, size_bytes = ?2, modified_ns = ?3, content_hash = ?4 WHERE file_path = ?5",
        )?;
        for file in touched_files {
            stmt.execute(params![
                file.language.as_str(),
                to_i64_u64(file.size_bytes)?,
                to_i64_u64(file.modified_ns)?,
                file.content_hash,
                file.file_path
            ])?;
        }
        Ok(())
    }

    pub(crate) fn replace_file(
        tx: &Transaction<'_>,
        file: &IndexedFileUpdate,
    ) -> anyhow::Result<()> {
        Self::delete_symbol_vectors_for_file(tx, &file.file_path)?;
        tx.prepare_cached("DELETE FROM symbols WHERE file_path = ?1")?
            .execute([&file.file_path])?;
        tx.prepare_cached("DELETE FROM files WHERE file_path = ?1")?
            .execute([&file.file_path])?;
        tx.prepare_cached(
            "INSERT INTO files (file_path, language, size_bytes, modified_ns, content_hash) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?
        .execute(params![
            file.file_path,
            file.language.as_str(),
            to_i64_u64(file.size_bytes)?,
            to_i64_u64(file.modified_ns)?,
            file.content_hash
        ])?;
        // Deduplicate IDs: if two symbols share an ID (e.g., inner class
        // re-defines __init__), keep the first one seen.
        let mut seen_ids = std::collections::HashSet::new();
        for symbol in &file.symbols {
            if seen_ids.insert(symbol.id.clone()) {
                Self::insert_symbol(tx, symbol)?;
            }
        }
        Ok(())
    }

    fn delete_symbol_vectors_for_file(tx: &Transaction<'_>, file_path: &str) -> anyhow::Result<()> {
        tx.execute(
            "DELETE FROM vec_symbol_embeddings
             WHERE symbol_rowid IN (SELECT row_id FROM symbols WHERE file_path = ?1)",
            [file_path],
        )?;
        Ok(())
    }

    pub(crate) fn insert_symbol(tx: &Transaction<'_>, symbol: &StoredSymbol) -> anyhow::Result<()> {
        tx.prepare_cached(
            r#"
            INSERT INTO symbols (
                id, file_path, language, extraction_tier, kind, qualified_name, name, signature,
                summary, source_text, start_line, end_line, start_byte, end_byte, source_hash,
                file_size_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
        "#,
        )?
        .execute(params![
            symbol.id,
            symbol.file_path,
            symbol.language.as_str(),
            symbol.extraction_tier.as_str(),
            symbol.kind.as_str(),
            symbol.qualified_name,
            symbol.name,
            symbol.signature,
            symbol.summary,
            symbol.source_text,
            symbol.start_line,
            symbol.end_line,
            to_i64_usize(symbol.start_byte)?,
            to_i64_usize(symbol.end_byte)?,
            symbol.source_hash,
            to_i64_u64(symbol.file_size_bytes)?
        ])?;
        Ok(())
    }

    pub(crate) fn file_count_with(&self, conn: &Connection) -> anyhow::Result<usize> {
        let count = conn.query_row("SELECT COUNT(*) FROM files", [], |row| row.get::<_, i64>(0))?;
        Ok(count as usize)
    }

    pub(crate) fn symbol_count_with(&self, conn: &Connection) -> anyhow::Result<usize> {
        let count = conn.query_row("SELECT COUNT(*) FROM symbols", [], |row| {
            row.get::<_, i64>(0)
        })?;
        Ok(count as usize)
    }

    pub(crate) fn languages_with(
        &self,
        conn: &Connection,
    ) -> anyhow::Result<BTreeMap<String, usize>> {
        let mut stmt = conn.prepare_cached(
            "SELECT language, COUNT(*) AS count FROM files GROUP BY language ORDER BY language",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut languages = BTreeMap::new();
        for row in rows {
            let (language, count) = row?;
            languages.insert(language, count as usize);
        }
        Ok(languages)
    }
}
