//! Read-side queries for the doc index: FTS search, section lookup, summaries.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

use rusqlite::OptionalExtension;
use serde_json::Value;

use crate::{config::BYTES_PER_TOKEN, core::pipeline::FileFingerprint, db};

use super::{DocStore, SearchSection, SectionRecord};

impl DocStore {
    pub fn file_fingerprints(&self) -> anyhow::Result<HashMap<String, FileFingerprint>> {
        let conn = self.open_connection()?;
        let mut stmt = conn.prepare_cached(
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
        let mut stmt = conn.prepare_cached(
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
        let mut stmt = conn.prepare_cached(
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
        if let Some(&bytes) = self.cached_total_bytes.get() {
            return Ok(bytes);
        }
        let conn = self.open_read_only()?;
        self.repo_total_bytes_with(&conn)
    }

    pub(super) fn repo_total_bytes_with(&self, conn: &rusqlite::Connection) -> anyhow::Result<u64> {
        if let Some(&bytes) = self.cached_total_bytes.get() {
            return Ok(bytes);
        }
        let bytes = conn.query_row(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM doc_files",
            [],
            |row| row.get::<_, i64>(0),
        )? as u64;
        let _ = self.cached_total_bytes.set(bytes);
        Ok(bytes)
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
        let total_bytes = self.repo_total_bytes_with(&conn)?;

        let mut formats: BTreeMap<String, usize> = BTreeMap::new();
        let mut stmt =
            conn.prepare_cached("SELECT format, COUNT(*) FROM doc_files GROUP BY format")?;
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
        let mut stmt =
            conn.prepare_cached("SELECT DISTINCT format FROM doc_files ORDER BY format")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn sections_for_file(&self, file_path: &str) -> anyhow::Result<Vec<Value>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare_cached(
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
        let mut stmt = conn.prepare_cached(
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

    pub fn all_sections(&self) -> anyhow::Result<Vec<SectionRecord>> {
        let conn = self.open_read_only()?;
        let root_path = self.root_path()?;
        let mut stmt = conn.prepare_cached(
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

    pub fn all_doc_files(&self) -> anyhow::Result<Vec<String>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare_cached("SELECT file_path FROM doc_files ORDER BY file_path")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn section_count(&self) -> anyhow::Result<usize> {
        let conn = self.open_read_only()?;
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM sections", [], |row| row.get(0))?;
        Ok(n as usize)
    }

    /// Fast stat-based staleness check for doc files.
    pub fn is_stale(&self, root_path: &Path) -> anyhow::Result<bool> {
        let conn = self.open_read_only()?;
        Ok(db::check_staleness(&conn, root_path, "doc_files", |rel| {
            crate::retrieval::sections::is_doc_file(rel)
        })?)
    }
}
