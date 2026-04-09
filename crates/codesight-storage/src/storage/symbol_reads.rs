//! Symbol- and file-row reads (outlines, metadata, substring search).

use rusqlite::OptionalExtension;
use rusqlite::params;

use serde_json::Value;

use crate::class_hierarchy::ClassIndexRow;
use crate::config::BYTES_PER_TOKEN;
use crate::core::symbols::{ExtractionTier, SymbolKind};
use crate::storage::{SymbolMetadata, SymbolRecord};

use super::IndexStore;

impl IndexStore {
    pub fn class_index_rows(&self) -> anyhow::Result<Vec<ClassIndexRow>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, name, file_path, start_line, source_text
            FROM symbols
            WHERE kind = 'class'
            ORDER BY file_path, start_line, id
        "#,
        )?;
        let rows = stmt.query_map([], |row| {
            let source: String = row.get(4)?;
            Ok(ClassIndexRow {
                id: row.get(0)?,
                name: row.get(1)?,
                file_path: row.get(2)?,
                start_line: row.get::<_, u32>(3)?,
                header_line: crate::class_hierarchy::header_line_from_source(&source),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn symbol_record(&self, id: &str) -> anyhow::Result<Option<SymbolRecord>> {
        let conn = self.open_read_only()?;
        let root_path = self.root_path()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, kind, extraction_tier, file_path, start_line, end_line, start_byte, end_byte, source_hash, source_text, file_size_bytes
            FROM symbols
            WHERE id = ?1
        "#,
        )?;

        let record = stmt
            .query_row([id], |row| {
                let kind_str: String = row.get(1)?;
                let kind = SymbolKind::from_db(&kind_str).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::other(format!(
                            "unknown symbol kind {kind_str}"
                        ))),
                    )
                })?;

                let tier_str: String = row.get(2)?;
                let extraction_tier = ExtractionTier::from_db(&tier_str).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::other(format!(
                            "unknown extraction tier {tier_str}"
                        ))),
                    )
                })?;

                Ok(SymbolRecord {
                    id: row.get(0)?,
                    kind,
                    extraction_tier,
                    file_path: row.get(3)?,
                    start_line: row.get::<_, u32>(4)?,
                    end_line: row.get::<_, u32>(5)?,
                    start_byte: row.get::<_, i64>(6)? as usize,
                    end_byte: row.get::<_, i64>(7)? as usize,
                    source_hash: row.get(8)?,
                    stored_source: row.get(9)?,
                    file_size_bytes: row.get::<_, i64>(10)? as u64,
                    root_path,
                })
            })
            .optional()?;
        Ok(record)
    }

    pub fn symbol_metadata(&self, id: &str) -> anyhow::Result<Option<SymbolMetadata>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, file_path, file_size_bytes, LENGTH(source_text)
            FROM symbols
            WHERE id = ?1
        "#,
        )?;
        let record = stmt
            .query_row([id], |row| {
                let symbol_size_bytes = row.get::<_, i64>(3)? as u64;
                let file_size_bytes = row.get::<_, i64>(2)? as u64;
                Ok(SymbolMetadata {
                    id: row.get(0)?,
                    file: row.get(1)?,
                    file_size_bytes,
                    symbol_size_bytes,
                    estimated_file_tokens: file_size_bytes / BYTES_PER_TOKEN,
                    estimated_symbol_tokens: symbol_size_bytes / BYTES_PER_TOKEN,
                    estimated_savings: file_size_bytes.saturating_sub(symbol_size_bytes)
                        / BYTES_PER_TOKEN,
                })
            })
            .optional()?;
        Ok(record)
    }

    pub fn file_list(&self) -> anyhow::Result<Vec<Value>> {
        let conn = self.open_read_only()?;
        let mut stmt =
            conn.prepare("SELECT file_path, language, size_bytes FROM files ORDER BY file_path")?;
        let rows = stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "path": row.get::<_, String>(0)?,
                "language": row.get::<_, String>(1)?,
                "size_bytes": row.get::<_, i64>(2)?,
            }))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// File paths with language, size, and live symbol counts (for `inspect_structure` file_tree).
    pub fn file_tree_rows(&self) -> anyhow::Result<Vec<(String, String, i64, u64)>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT f.file_path, f.language, f.size_bytes,
                   (SELECT COUNT(*) FROM symbols s WHERE s.file_path = f.file_path)
            FROM files f
            ORDER BY f.file_path
        "#,
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)? as u64,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Check whether a file has any C++ specific symbol kinds (class, method, namespace).
    /// Used to distinguish C vs C++ for .h files.
    pub fn file_has_cpp_symbols(&self, file_path: &str) -> anyhow::Result<bool> {
        let conn = self.open_read_only()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM symbols WHERE file_path = ?1 AND kind IN ('class', 'method', 'namespace', 'type')",
            [file_path],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn repo_outline(&self) -> anyhow::Result<Vec<Value>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare(
            "SELECT file_path, id, kind, extraction_tier, name, signature, start_line FROM symbols ORDER BY file_path, start_line",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "file": row.get::<_, String>(0)?,
                "id": row.get::<_, String>(1)?,
                "kind": row.get::<_, String>(2)?,
                "extraction_tier": row.get::<_, String>(3)?,
                "name": row.get::<_, String>(4)?,
                "signature": row.get::<_, String>(5)?,
                "line": row.get::<_, u32>(6)?,
            }))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn file_outline(&self, file_path: &str) -> anyhow::Result<Vec<Value>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare(
            "SELECT id, kind, extraction_tier, name, signature, start_line, end_line FROM symbols WHERE file_path = ?1 ORDER BY start_line",
        )?;
        let rows = stmt.query_map([file_path], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "kind": row.get::<_, String>(1)?,
                "extraction_tier": row.get::<_, String>(2)?,
                "name": row.get::<_, String>(3)?,
                "signature": row.get::<_, String>(4)?,
                "line": row.get::<_, u32>(5)?,
                "end_line": row.get::<_, u32>(6)?,
            }))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Heuristic reference search: symbols whose stored `source_text` contains `needle`.
    pub fn symbols_source_substring(
        &self,
        needle: &str,
        exclude_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<Value>> {
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.open_read_only()?;
        let lim = limit.clamp(1, 50) as i64;
        let mut stmt = conn.prepare(
            r#"SELECT id, kind, name, file_path, start_line
                FROM symbols
                WHERE id != ?1 AND instr(source_text, ?2) > 0
                LIMIT ?3"#,
        )?;
        let rows = stmt.query_map(rusqlite::params![exclude_id, needle, lim], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "kind": row.get::<_, String>(1)?,
                "name": row.get::<_, String>(2)?,
                "file": row.get::<_, String>(3)?,
                "line": row.get::<_, u32>(4)?,
            }))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn symbol_names_by_id(&self, id: &str) -> anyhow::Result<Option<(String, String)>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare("SELECT name, qualified_name FROM symbols WHERE id = ?1")?;
        let row = stmt
            .query_row([id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .optional()?;
        Ok(row)
    }

    pub fn symbol_signature_by_id(&self, id: &str) -> anyhow::Result<Option<String>> {
        let conn = self.open_read_only()?;
        let row = conn
            .query_row(
                "SELECT signature FROM symbols WHERE id = ?1",
                params![id],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(row)
    }

    pub fn file_bytes(&self, file_path: &str) -> anyhow::Result<Option<u64>> {
        let conn = self.open_read_only()?;
        let result: Option<i64> = conn
            .query_row(
                "SELECT size_bytes FROM files WHERE file_path = ?1",
                [file_path],
                |row| row.get(0),
            )
            .optional()?;
        Ok(result.map(|v| v as u64))
    }

    pub fn file_language(&self, file_path: &str) -> anyhow::Result<Option<String>> {
        let conn = self.open_read_only()?;
        let result: Option<String> = conn
            .query_row(
                "SELECT language FROM files WHERE file_path = ?1",
                [file_path],
                |row| row.get(0),
            )
            .optional()?;
        Ok(result)
    }
}
