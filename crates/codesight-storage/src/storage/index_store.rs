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
        let store = Self { db_path };
        let conn = db::open_rw(&store.db_path)?;
        migrations::apply_code_index_migrations(&conn)?;
        Ok(store)
    }

    pub fn open_existing(db_path: PathBuf) -> anyhow::Result<Option<Self>> {
        if !db_path.exists() {
            return Ok(None);
        }
        let store = Self { db_path };
        let conn = store.open_read_only()?;
        conn.query_row("SELECT COUNT(*) FROM repo_info", [], |_| Ok(()))
            .context("repo_info missing from existing index")?;
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

    pub fn file_fingerprints(
        &self,
    ) -> anyhow::Result<std::collections::HashMap<String, FileFingerprint>> {
        let conn = self.open_connection()?;
        let mut stmt = conn.prepare(
            "SELECT file_path, language, size_bytes, modified_ns, content_hash FROM files",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(FileFingerprint {
                file_path: row.get(0)?,
                language: row.get(1)?,
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
    pub fn apply_index(
        &self,
        repo: &str,
        root_path: &std::path::Path,
        indexed_at: &str,
        index_version: u32,
        touched_files: &[FileFingerprint],
        updated_files: &[IndexedFileUpdate],
        removed_files: &[String],
    ) -> anyhow::Result<()> {
        self.apply_index_with_deps(
            repo,
            root_path,
            indexed_at,
            index_version,
            touched_files,
            updated_files,
            removed_files,
            &[],
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
        dep_edges: &[(String, String, FactQuality)],
    ) -> anyhow::Result<()> {
        let mut conn = self.open_connection()?;
        let tx = conn.transaction()?;
        Self::delete_removed_files(&tx, removed_files)?;
        Self::touch_files(&tx, touched_files)?;
        for file in updated_files {
            Self::replace_file(&tx, file)?;
        }

        // Replace dependency edges for updated files
        for file in updated_files {
            tx.execute(
                "DELETE FROM dependency_edges WHERE source_file = ?1",
                [&file.file_path],
            )?;
        }
        for (source, target, quality) in dep_edges {
            let q = quality.as_str();
            tx.execute(
                "INSERT INTO dependency_edges (source_file, target_file, import_fact_quality) VALUES (?1, ?2, ?3)
                 ON CONFLICT(source_file, target_file) DO UPDATE SET import_fact_quality = excluded.import_fact_quality",
                params![source, target, q],
            )?;
        }

        tx.execute("DELETE FROM repo_info", [])?;
        tx.execute(
            "INSERT INTO repo_info (repo, root_path, indexed_at, index_version, schema_version) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![repo, root_path.to_string_lossy().to_string(), indexed_at, index_version, SCHEMA_VERSION],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn repo_summary(&self) -> anyhow::Result<RepoMetadata> {
        let conn = self.open_read_only()?;
        let (repo, indexed_at, index_version): (String, String, u32) = conn.query_row(
            "SELECT repo, indexed_at, index_version FROM repo_info LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        Ok(RepoMetadata {
            repo,
            indexed_at,
            symbol_count: self.symbol_count_with(&conn)?,
            file_count: self.file_count_with(&conn)?,
            languages: self.languages_with(&conn)?,
            index_version,
            estimated_naive_tokens: 0,
            index_size_bytes: 0,
            is_stale: None,
        })
    }

    pub fn repo_metadata(&self) -> anyhow::Result<RepoMetadata> {
        let mut meta = self.repo_summary()?;
        meta.estimated_naive_tokens = self.repo_total_bytes()? / crate::config::BYTES_PER_TOKEN;
        meta.index_size_bytes = fs::metadata(&self.db_path)
            .map(|m| m.len())
            .unwrap_or_default();
        Ok(meta)
    }

    pub fn repo_total_bytes(&self) -> anyhow::Result<u64> {
        let conn = self.open_read_only()?;
        let bytes = conn.query_row(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM files",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(bytes as u64)
    }

    pub fn root_path(&self) -> anyhow::Result<PathBuf> {
        let conn = self.open_read_only()?;
        let value: String =
            conn.query_row("SELECT root_path FROM repo_info LIMIT 1", [], |row| {
                row.get(0)
            })?;
        Ok(PathBuf::from(value))
    }

    pub(crate) fn open_connection(&self) -> anyhow::Result<Connection> {
        let conn = db::open_rw(&self.db_path)?;
        migrations::apply_code_index_migrations(&conn)?;
        Ok(conn)
    }

    pub(crate) fn open_read_only(&self) -> anyhow::Result<Connection> {
        Ok(db::open_ro(&self.db_path)?)
    }

    pub(crate) fn delete_removed_files(
        tx: &Transaction<'_>,
        removed_files: &[String],
    ) -> anyhow::Result<()> {
        for file_path in removed_files {
            tx.execute("DELETE FROM symbols WHERE file_path = ?1", [file_path])?;
            tx.execute(
                "DELETE FROM dependency_edges WHERE source_file = ?1 OR target_file = ?1",
                [file_path],
            )?;
            tx.execute("DELETE FROM files WHERE file_path = ?1", [file_path])?;
        }
        Ok(())
    }

    pub(crate) fn touch_files(
        tx: &Transaction<'_>,
        touched_files: &[FileFingerprint],
    ) -> anyhow::Result<()> {
        for file in touched_files {
            tx.execute(
                "UPDATE files SET language = ?1, size_bytes = ?2, modified_ns = ?3, content_hash = ?4 WHERE file_path = ?5",
                params![
                    file.language,
                    to_i64_u64(file.size_bytes)?,
                    to_i64_u64(file.modified_ns)?,
                    file.content_hash,
                    file.file_path
                ],
            )?;
        }
        Ok(())
    }

    pub(crate) fn replace_file(
        tx: &Transaction<'_>,
        file: &IndexedFileUpdate,
    ) -> anyhow::Result<()> {
        tx.execute(
            "DELETE FROM symbols WHERE file_path = ?1",
            [&file.file_path],
        )?;
        tx.execute("DELETE FROM files WHERE file_path = ?1", [&file.file_path])?;
        tx.execute(
            "INSERT INTO files (file_path, language, size_bytes, modified_ns, content_hash) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                file.file_path,
                file.language,
                to_i64_u64(file.size_bytes)?,
                to_i64_u64(file.modified_ns)?,
                file.content_hash
            ],
        )?;
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

    pub(crate) fn insert_symbol(tx: &Transaction<'_>, symbol: &StoredSymbol) -> anyhow::Result<()> {
        tx.execute(
            r#"
            INSERT INTO symbols (
                id, file_path, language, extraction_tier, kind, qualified_name, name, signature,
                summary, source_text, start_line, end_line, start_byte, end_byte, source_hash,
                file_size_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
        "#,
            params![
                symbol.id,
                symbol.file_path,
                symbol.language,
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
            ],
        )?;
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
        let mut stmt = conn.prepare(
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
