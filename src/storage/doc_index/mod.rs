//! SQLite storage layer for documentation sections.

mod ingest;

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use anyhow::Context;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::Value;

use crate::{
    config::{BYTES_PER_TOKEN, SCHEMA_VERSION},
    core::pipeline::FileFingerprint,
    db,
    sections::Section,
};

pub use ingest::{DocIndexSummary, index_local_docs};

const DOC_STORE_INIT_SQL_TAIL: &str = r#"
CREATE TABLE IF NOT EXISTS doc_files (
    file_path TEXT PRIMARY KEY,
    format TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    modified_ns INTEGER NOT NULL,
    content_hash TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sections (
    row_id INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    file_path TEXT NOT NULL,
    heading TEXT NOT NULL,
    heading_path TEXT NOT NULL,
    level INTEGER NOT NULL,
    summary TEXT NOT NULL,
    content_text TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    file_size_bytes INTEGER NOT NULL,
    FOREIGN KEY(file_path) REFERENCES doc_files(file_path) ON DELETE CASCADE
);

CREATE VIRTUAL TABLE IF NOT EXISTS section_fts USING fts5(
    heading,
    heading_path,
    summary,
    content_text,
    file_path,
    content='sections',
    content_rowid='row_id',
    tokenize='porter unicode61'
);

CREATE TRIGGER IF NOT EXISTS sections_ai AFTER INSERT ON sections BEGIN
    INSERT INTO section_fts(rowid, heading, heading_path, summary, content_text, file_path)
    VALUES (new.row_id, new.heading, new.heading_path, new.summary, new.content_text, new.file_path);
END;

CREATE TRIGGER IF NOT EXISTS sections_ad AFTER DELETE ON sections BEGIN
    INSERT INTO section_fts(section_fts, rowid, heading, heading_path, summary, content_text, file_path)
    VALUES ('delete', old.row_id, old.heading, old.heading_path, old.summary, old.content_text, old.file_path);
END;

CREATE TRIGGER IF NOT EXISTS sections_au AFTER UPDATE ON sections BEGIN
    INSERT INTO section_fts(section_fts, rowid, heading, heading_path, summary, content_text, file_path)
    VALUES ('delete', old.row_id, old.heading, old.heading_path, old.summary, old.content_text, old.file_path);
    INSERT INTO section_fts(rowid, heading, heading_path, summary, content_text, file_path)
    VALUES (new.row_id, new.heading, new.heading_path, new.summary, new.content_text, new.file_path);
END;
"#;

static DOC_STORE_INIT_SQL: LazyLock<String> =
    LazyLock::new(|| format!("{}{}", db::REPO_INFO_DDL, DOC_STORE_INIT_SQL_TAIL));

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

#[derive(Debug, Clone)]
pub struct DocStore {
    db_path: PathBuf,
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
        let store = Self { db_path };
        let conn = store.open_connection()?;
        conn.execute_batch(DOC_STORE_INIT_SQL.as_str())?;
        Ok(store)
    }

    pub fn open_existing(db_path: PathBuf) -> anyhow::Result<Option<Self>> {
        if !db_path.exists() {
            return Ok(None);
        }
        let store = Self { db_path };
        let conn = store.open_read_only()?;
        conn.query_row("SELECT COUNT(*) FROM repo_info", [], |_| Ok(()))
            .context("repo_info missing from existing doc index")?;
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

    pub fn file_fingerprints(&self) -> anyhow::Result<HashMap<String, FileFingerprint>> {
        let conn = self.open_connection()?;
        let mut stmt = conn.prepare(
            "SELECT file_path, format, size_bytes, modified_ns, content_hash FROM doc_files",
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
            let fp = row?;
            items.insert(fp.file_path.clone(), fp);
        }
        Ok(items)
    }

    pub fn apply_index(
        &self,
        repo: &str,
        root_path: &Path,
        indexed_at: &str,
        index_version: u32,
        updated_files: &[DocFileUpdate],
        removed_files: &[String],
    ) -> anyhow::Result<()> {
        let mut conn = self.open_connection()?;
        let tx = conn.transaction()?;

        for file_path in removed_files {
            tx.execute("DELETE FROM sections WHERE file_path = ?1", [file_path])?;
            tx.execute("DELETE FROM doc_files WHERE file_path = ?1", [file_path])?;
        }

        for file in updated_files {
            tx.execute(
                "DELETE FROM sections WHERE file_path = ?1",
                [&file.file_path],
            )?;
            tx.execute(
                "DELETE FROM doc_files WHERE file_path = ?1",
                [&file.file_path],
            )?;
            tx.execute(
                "INSERT INTO doc_files (file_path, format, size_bytes, modified_ns, content_hash) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![file.file_path, file.format, file.size_bytes as i64, file.modified_ns as i64, file.content_hash],
            )?;
            for section in &file.sections {
                Self::insert_section(&tx, section, &file.source)?;
            }
        }

        tx.execute("DELETE FROM repo_info", [])?;
        tx.execute(
            "INSERT INTO repo_info (repo, root_path, indexed_at, index_version, schema_version) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![repo, root_path.to_string_lossy().to_string(), indexed_at, index_version, SCHEMA_VERSION],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn insert_section(tx: &Transaction<'_>, section: &Section, source: &str) -> anyhow::Result<()> {
        let content_text = source
            .get(section.start_byte..section.end_byte)
            .unwrap_or("");
        tx.execute(
            r#"INSERT INTO sections (
                id, file_path, heading, heading_path, level, summary, content_text,
                start_line, end_line, start_byte, end_byte, content_hash, file_size_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"#,
            params![
                section.id,
                section.file_path,
                section.heading,
                section.heading_path,
                section.level as i64,
                section.summary,
                content_text,
                section.start_line,
                section.end_line,
                section.start_byte as i64,
                section.end_byte as i64,
                section.content_hash,
                source.len() as i64,
            ],
        )?;
        Ok(())
    }

    pub fn search_sections(
        &self,
        query: &str,
        scope: Option<&str>,
        max_results: usize,
    ) -> anyhow::Result<Vec<SearchSection>> {
        let query_tokens = db::tokenize(query);
        if query_tokens.is_empty() {
            return Ok(Vec::new());
        }

        let fts_query = query_tokens
            .iter()
            .map(|t| format!("{t}*"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let row_limit = (max_results.clamp(1, 25) * 6) as i64;

        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT
                s.id, s.heading, s.heading_path, s.level, s.file_path,
                s.start_line, s.summary,
                bm25(section_fts, 8.0, 6.0, 3.0, 1.0, 2.0) AS rank
            FROM section_fts
            JOIN sections s ON s.row_id = section_fts.rowid
            WHERE section_fts MATCH :fts
              AND (:scope IS NULL OR s.file_path LIKE :scope_prefix)
            ORDER BY rank ASC
            LIMIT :row_limit
        "#,
        )?;

        let scope_prefix = scope.map(|s| format!("{s}%"));
        let rows = stmt.query_map(
            rusqlite::named_params! {
                ":fts": &fts_query,
                ":scope": &scope,
                ":scope_prefix": &scope_prefix.as_deref(),
                ":row_limit": &row_limit,
            },
            |row| {
                Ok(SearchSection {
                    id: row.get(0)?,
                    heading: row.get(1)?,
                    heading_path: row.get(2)?,
                    level: row.get::<_, i64>(3)? as u8,
                    file: row.get(4)?,
                    line: row.get::<_, u32>(5)?,
                    summary: row.get(6)?,
                    score: 1.0 / (1.0 + row.get::<_, f64>(7)?.max(0.0)),
                })
            },
        )?;

        let mut results: Vec<SearchSection> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for row in rows {
            let s = row?;
            if seen.insert(s.id.clone()) {
                results.push(s);
            }
        }
        results.truncate(max_results.clamp(1, 25));
        Ok(results)
    }

    pub fn section_record(&self, id: &str) -> anyhow::Result<Option<SectionRecord>> {
        let conn = self.open_read_only()?;
        let root_path = self.root_path()?;
        let mut stmt = conn.prepare(
            r#"SELECT id, file_path, heading, level, start_line, end_line,
                      start_byte, end_byte, content_hash, content_text, file_size_bytes
               FROM sections WHERE id = ?1"#,
        )?;
        let record = stmt
            .query_row([id], |row| {
                Ok(SectionRecord {
                    id: row.get(0)?,
                    file_path: row.get(1)?,
                    heading: row.get(2)?,
                    level: row.get::<_, i64>(3)? as u8,
                    start_line: row.get::<_, u32>(4)?,
                    end_line: row.get::<_, u32>(5)?,
                    start_byte: row.get::<_, i64>(6)? as usize,
                    end_byte: row.get::<_, i64>(7)? as usize,
                    content_hash: row.get(8)?,
                    content_text: row.get(9)?,
                    file_size_bytes: row.get::<_, i64>(10)? as u64,
                    root_path,
                })
            })
            .optional()?;
        Ok(record)
    }

    pub fn section_file_size(&self, id: &str) -> anyhow::Result<Option<u64>> {
        let conn = self.open_read_only()?;
        let result: Option<i64> = conn
            .query_row(
                "SELECT file_size_bytes FROM sections WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(result.map(|v| v as u64))
    }

    pub fn root_path(&self) -> anyhow::Result<PathBuf> {
        let conn = self.open_read_only()?;
        let value: String =
            conn.query_row("SELECT root_path FROM repo_info LIMIT 1", [], |row| {
                row.get(0)
            })?;
        Ok(PathBuf::from(value))
    }

    pub fn repo_total_bytes(&self) -> anyhow::Result<u64> {
        let conn = self.open_read_only()?;
        let bytes = conn.query_row(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM doc_files",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(bytes as u64)
    }

    pub fn doc_summary(&self) -> anyhow::Result<Value> {
        let conn = self.open_read_only()?;
        let (repo, indexed_at, index_version): (String, String, u32) = conn.query_row(
            "SELECT repo, indexed_at, index_version FROM repo_info LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let file_count = conn.query_row("SELECT COUNT(*) FROM doc_files", [], |row| {
            row.get::<_, i64>(0)
        })? as usize;
        let section_count = conn.query_row("SELECT COUNT(*) FROM sections", [], |row| {
            row.get::<_, i64>(0)
        })? as usize;
        let total_bytes = self.repo_total_bytes()?;

        let mut formats: BTreeMap<String, usize> = BTreeMap::new();
        let mut stmt = conn.prepare("SELECT format, COUNT(*) FROM doc_files GROUP BY format")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (fmt, count) = row?;
            formats.insert(fmt, count as usize);
        }

        Ok(serde_json::json!({
            "source_id": repo,
            "indexed_at": indexed_at,
            "file_count": file_count,
            "section_count": section_count,
            "formats": formats,
            "estimated_naive_tokens": total_bytes / BYTES_PER_TOKEN,
            "index_version": index_version,
        }))
    }

    pub fn doc_file_exists(&self, file_path: &str) -> anyhow::Result<bool> {
        let conn = self.open_read_only()?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(1) FROM doc_files WHERE file_path = ?1",
            [file_path],
            |row| row.get(0),
        )?;
        Ok(n > 0)
    }

    pub fn distinct_doc_formats(&self) -> anyhow::Result<Vec<String>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare("SELECT DISTINCT format FROM doc_files ORDER BY format")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn sections_for_file(&self, file_path: &str) -> anyhow::Result<Vec<Value>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare(
            r#"SELECT id, heading, heading_path, level, start_line, end_line, summary
                FROM sections WHERE file_path = ?1 ORDER BY start_line"#,
        )?;
        let rows = stmt.query_map([file_path], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "heading": row.get::<_, String>(1)?,
                "heading_path": row.get::<_, String>(2)?,
                "level": row.get::<_, i64>(3)?,
                "line": row.get::<_, u32>(4)?,
                "end_line": row.get::<_, u32>(5)?,
                "summary": row.get::<_, String>(6)?,
            }))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn toc(&self) -> anyhow::Result<Vec<Value>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare(
            "SELECT id, file_path, heading, heading_path, level, start_line FROM sections ORDER BY file_path, start_line",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "file": row.get::<_, String>(1)?,
                "heading": row.get::<_, String>(2)?,
                "heading_path": row.get::<_, String>(3)?,
                "level": row.get::<_, i64>(4)?,
                "line": row.get::<_, u32>(5)?,
            }))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Return all section records (id, heading, heading_path, level, file_path, content_text).
    pub fn all_sections(&self) -> anyhow::Result<Vec<SectionRecord>> {
        let conn = self.open_read_only()?;
        let root_path = self.root_path()?;
        let mut stmt = conn.prepare(
            r#"SELECT id, file_path, heading, level, start_line, end_line,
                      start_byte, end_byte, content_hash, content_text, file_size_bytes
               FROM sections ORDER BY file_path, start_line"#,
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SectionRecord {
                id: row.get(0)?,
                file_path: row.get(1)?,
                heading: row.get(2)?,
                level: row.get::<_, i64>(3)? as u8,
                start_line: row.get::<_, u32>(4)?,
                end_line: row.get::<_, u32>(5)?,
                start_byte: row.get::<_, i64>(6)? as usize,
                end_byte: row.get::<_, i64>(7)? as usize,
                content_hash: row.get(8)?,
                content_text: row.get(9)?,
                file_size_bytes: row.get::<_, i64>(10)? as u64,
                root_path: root_path.clone(),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Return all indexed doc file paths.
    pub fn all_doc_files(&self) -> anyhow::Result<Vec<String>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare("SELECT file_path FROM doc_files ORDER BY file_path")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Return section count.
    pub fn section_count(&self) -> anyhow::Result<usize> {
        let conn = self.open_read_only()?;
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM sections", [], |row| row.get(0))?;
        Ok(n as usize)
    }

    /// Fast stat-based staleness check for doc files.
    pub fn is_stale(&self, root_path: &Path) -> anyhow::Result<bool> {
        let conn = self.open_read_only()?;
        db::check_staleness(&conn, root_path, "doc_files", |rel| {
            crate::sections::is_doc_file(rel)
        })
    }

    fn open_connection(&self) -> anyhow::Result<Connection> {
        db::open_rw(&self.db_path)
    }

    fn open_read_only(&self) -> anyhow::Result<Connection> {
        db::open_ro(&self.db_path)
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
