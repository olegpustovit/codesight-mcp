//! Symbol- and file-row reads (outlines, metadata, substring search).

use rusqlite::OptionalExtension;
use rusqlite::params;

use serde_json::Value;

use crate::class_hierarchy::ClassIndexRow;
use crate::config::BYTES_PER_TOKEN;
use crate::db::SKIP_DIRS;
use crate::storage::{SymbolMetadata, SymbolRecord};

use super::IndexStore;
use super::row_utils::symbol_record_from_row;

/// Tier 1.4: drop function-local symbols (e.g. `let` constants inside a
/// test function) from an outline. Treats `function` and `method` symbols
/// in the same file as scopes; any symbol whose dotted-path id has one of
/// those as an ancestor is filtered out. Operates on the JSON shape emitted
/// by [`IndexStore::file_outline`].
///
/// Symbol id format: `<file_path>::<segment>(.<segment>)*#<kind>`. The
/// segments encode nesting; we walk parent prefixes and drop the row when
/// any ancestor matches a known function/method in this file.
fn filter_function_local(rows: Vec<Value>) -> Vec<Value> {
    use std::collections::HashSet;

    let function_paths: HashSet<String> = rows
        .iter()
        .filter_map(|s| {
            let kind = s.get("kind").and_then(Value::as_str)?;
            if kind != "function" && kind != "method" {
                return None;
            }
            let id = s.get("id").and_then(Value::as_str)?;
            // strip `<file>::` and `#<kind>` to get the dotted path
            let after_file = id.split_once("::").map(|(_, t)| t).unwrap_or(id);
            let dotted = after_file
                .rsplit_once('#')
                .map(|(d, _)| d)
                .unwrap_or(after_file);
            Some(dotted.to_string())
        })
        .collect();

    if function_paths.is_empty() {
        return rows;
    }

    rows.into_iter()
        .filter(|s| {
            let Some(id) = s.get("id").and_then(Value::as_str) else {
                return true;
            };
            let after_file = id.split_once("::").map(|(_, t)| t).unwrap_or(id);
            let dotted = after_file
                .rsplit_once('#')
                .map(|(d, _)| d)
                .unwrap_or(after_file);
            // Walk *strict* ancestors only (skip the symbol's own path).
            let parts: Vec<&str> = dotted.split('.').collect();
            for i in 0..parts.len().saturating_sub(1) {
                let prefix = parts[..=i].join(".");
                if function_paths.contains(&prefix) {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// Paths under a `SKIP_DIRS` top-level directory (e.g. `target/`, `.git/`).
/// Storage-only filter: belt-and-suspenders layer on top of the walker.
/// Per-language manifest / build-script noise (`build.rs`, `package.json`, …)
/// lives on the `Language` trait and is filtered by the engine layer.
pub fn is_in_skip_dir(path: &str) -> bool {
    path.split('/')
        .next()
        .is_some_and(|first| SKIP_DIRS.contains(&first))
}

impl IndexStore {
    pub fn class_index_rows(&self) -> anyhow::Result<Vec<ClassIndexRow>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare_cached(
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
        let mut stmt = conn.prepare_cached(
            r#"
            SELECT id, kind, extraction_tier, file_path, start_line, end_line, start_byte, end_byte, source_hash, source_text, file_size_bytes
            FROM symbols
            WHERE id = ?1
        "#,
        )?;

        let record = stmt
            .query_row([id], |row| symbol_record_from_row(row, &root_path))
            .optional()?;
        Ok(record)
    }

    /// Resolve a `(file_path, start_line)` pair to the indexed symbol id.
    ///
    /// Ranks exact `start_line` hits first, then the innermost enclosing
    /// symbol (most recent start_line ≤ request, smallest span). Used by
    /// `read_content` so callers that only know `path:line` can avoid the
    /// stable_id lookup step.
    pub fn symbol_id_at_location(
        &self,
        file_path: &str,
        start_line: u32,
    ) -> anyhow::Result<Option<String>> {
        let conn = self.open_read_only()?;
        let exact: Option<String> = conn
            .query_row(
                "SELECT id FROM symbols
                 WHERE file_path = ?1 AND start_line = ?2
                 ORDER BY (end_line - start_line) ASC LIMIT 1",
                params![file_path, start_line],
                |row| row.get(0),
            )
            .optional()?;
        if exact.is_some() {
            return Ok(exact);
        }
        let enclosing: Option<String> = conn
            .query_row(
                "SELECT id FROM symbols
                 WHERE file_path = ?1 AND start_line <= ?2 AND end_line >= ?2
                 ORDER BY (end_line - start_line) ASC LIMIT 1",
                params![file_path, start_line],
                |row| row.get(0),
            )
            .optional()?;
        Ok(enclosing)
    }

    /// Resolve a `(file_path, name)` pair to an indexed symbol id.
    ///
    /// Agents often know where a symbol lives and what it's called but
    /// don't have the stable id or its start line. Prefers the outermost
    /// match (smallest depth, largest span) so overloaded / nested symbols
    /// surface the top-level one.
    pub fn symbol_id_at_name(&self, file_path: &str, name: &str) -> anyhow::Result<Option<String>> {
        let conn = self.open_read_only()?;
        let picked: Option<String> = conn
            .query_row(
                "SELECT id FROM symbols
                 WHERE file_path = ?1 AND name = ?2
                 ORDER BY (end_line - start_line) DESC, start_line ASC LIMIT 1",
                params![file_path, name],
                |row| row.get(0),
            )
            .optional()?;
        Ok(picked)
    }

    /// Resolve a bare `name` to an indexed symbol id when unique across the
    /// whole index. Returns `None` when the name matches zero or ≥2 rows so
    /// callers can surface an ambiguity error instead of guessing.
    pub fn symbol_id_by_unique_name(&self, name: &str) -> anyhow::Result<Option<String>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare_cached("SELECT id FROM symbols WHERE name = ?1 LIMIT 2")?;
        let mut rows = stmt.query([name])?;
        let Some(first) = rows.next()? else {
            return Ok(None);
        };
        let id: String = first.get(0)?;
        if rows.next()?.is_some() {
            return Ok(None);
        }
        Ok(Some(id))
    }

    pub fn symbol_metadata(&self, id: &str) -> anyhow::Result<Option<SymbolMetadata>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare_cached(
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
        let mut stmt = conn.prepare_cached(
            "SELECT file_path, language, size_bytes FROM files ORDER BY file_path",
        )?;
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
        let mut stmt = conn.prepare_cached(
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

    pub fn repo_outline(&self) -> anyhow::Result<Vec<Value>> {
        let conn = self.open_read_only()?;
        // `extraction_tier` is implementation detail (curated SCM vs
        // heuristic) — the agent only needs to know the symbol exists. Drop
        // from per-row output; surface repo-level only if needed.
        let mut stmt = conn.prepare_cached(
            "SELECT file_path, id, kind, name, signature, start_line FROM symbols ORDER BY file_path, start_line",
        )?;
        let rows = stmt.query_map([], |row| {
            let file_path: String = row.get(0)?;
            Ok((
                file_path,
                serde_json::json!({
                    "file": row.get::<_, String>(0)?,
                    "id": row.get::<_, String>(1)?,
                    "kind": row.get::<_, String>(2)?,
                    "name": row.get::<_, String>(3)?,
                    "signature": row.get::<_, String>(4)?,
                    "line": row.get::<_, u32>(5)?,
                }),
            ))
        })?;
        let out: Vec<Value> = rows
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|(path, _)| !is_in_skip_dir(path))
            .map(|(_, v)| v)
            .collect();
        Ok(out)
    }

    pub fn file_outline(&self, file_path: &str) -> anyhow::Result<Vec<Value>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare_cached(
            "SELECT id, kind, name, signature, start_line, end_line FROM symbols WHERE file_path = ?1 ORDER BY start_line",
        )?;
        let rows: Vec<Value> = stmt
            .query_map([file_path], |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, String>(0)?,
                    "kind": row.get::<_, String>(1)?,
                    "name": row.get::<_, String>(2)?,
                    "signature": row.get::<_, String>(3)?,
                    "line": row.get::<_, u32>(4)?,
                    "end_line": row.get::<_, u32>(5)?,
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(filter_function_local(rows))
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
        let mut stmt = conn.prepare_cached(
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
        let mut stmt =
            conn.prepare_cached("SELECT name, qualified_name FROM symbols WHERE id = ?1")?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn ensure_file_row(conn: &rusqlite::Connection, file_path: &str) {
        conn.execute(
            "INSERT OR IGNORE INTO files (file_path, language, size_bytes, modified_ns, content_hash) \
             VALUES (?1, 'rust', 0, 0, 'h')",
            [file_path],
        )
        .unwrap();
    }

    fn insert_raw_symbol(
        store: &IndexStore,
        id: &str,
        name: &str,
        file_path: &str,
        start_line: u32,
        end_line: u32,
    ) {
        let conn = store.open_connection().unwrap();
        ensure_file_row(&conn, file_path);
        conn.execute(
            "INSERT INTO symbols (id, file_path, language, extraction_tier, kind, \
             qualified_name, name, signature, summary, source_text, start_line, end_line, \
             start_byte, end_byte, source_hash, file_size_bytes) \
             VALUES (?1, ?2, 'rust', 'curated_query_pack', 'function', '', ?3, '', '', '', ?4, ?5, 0, 0, 'h', 0)",
            params![id, file_path, name, start_line as i64, end_line as i64],
        )
        .unwrap();
    }

    fn new_store() -> (tempfile::TempDir, IndexStore) {
        let dir = tempdir().unwrap();
        let store = IndexStore::open(dir.path().join("x.sqlite")).unwrap();
        (dir, store)
    }

    #[test]
    fn symbol_id_at_name_returns_match_for_path_and_name() {
        let (_d, store) = new_store();
        insert_raw_symbol(&store, "f.rs::open_rw#function", "open_rw", "f.rs", 10, 20);
        let got = store.symbol_id_at_name("f.rs", "open_rw").unwrap();
        assert_eq!(got.as_deref(), Some("f.rs::open_rw#function"));
    }

    #[test]
    fn symbol_id_at_name_returns_none_when_absent() {
        let (_d, store) = new_store();
        assert!(
            store
                .symbol_id_at_name("missing.rs", "no_such")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn symbol_id_at_name_prefers_largest_span() {
        // Nested method inside a struct impl: both share (file, name) in some
        // extractors. We want the outer one (larger span) to win.
        let (_d, store) = new_store();
        insert_raw_symbol(&store, "f.rs::inner#function", "work", "f.rs", 5, 7);
        insert_raw_symbol(&store, "f.rs::outer#function", "work", "f.rs", 1, 100);
        let got = store.symbol_id_at_name("f.rs", "work").unwrap();
        assert_eq!(got.as_deref(), Some("f.rs::outer#function"));
    }

    #[test]
    fn symbol_id_by_unique_name_returns_match_when_single() {
        let (_d, store) = new_store();
        insert_raw_symbol(&store, "f.rs::unique#function", "unique", "f.rs", 1, 2);
        let got = store.symbol_id_by_unique_name("unique").unwrap();
        assert_eq!(got.as_deref(), Some("f.rs::unique#function"));
    }

    #[test]
    fn symbol_id_by_unique_name_none_when_ambiguous() {
        let (_d, store) = new_store();
        insert_raw_symbol(&store, "a.rs::dup#function", "dup", "a.rs", 1, 2);
        insert_raw_symbol(&store, "b.rs::dup#function", "dup", "b.rs", 1, 2);
        assert!(store.symbol_id_by_unique_name("dup").unwrap().is_none());
    }

    #[test]
    fn symbol_id_by_unique_name_none_when_absent() {
        let (_d, store) = new_store();
        assert!(store.symbol_id_by_unique_name("no_such").unwrap().is_none());
    }
}
