//! Analysis-oriented aggregations and reference counts.

use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeMap;

use rusqlite::OptionalExtension;

use codesight_core::reachability::{EdgeKind, StructuralEdge};

use super::IndexStore;
use super::types::AnalysisSymbol;

impl IndexStore {
    pub fn fn_method_symbol_count(&self) -> anyhow::Result<usize> {
        let conn = self.open_read_only()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM symbols WHERE kind IN ('function', 'method')",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Average span in lines for function/method symbols (crude complexity proxy).
    pub fn avg_line_span_fn_methods(&self) -> anyhow::Result<f64> {
        let conn = self.open_read_only()?;
        let avg: Option<f64> = conn
            .query_row(
                "SELECT AVG(CAST((end_line - start_line + 1) AS REAL)) FROM symbols WHERE kind IN ('function', 'method')",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(avg.unwrap_or(0.0))
    }

    /// `kind` → symbol count (lowercase kind strings, same as DB).
    pub fn kind_distribution(&self) -> anyhow::Result<BTreeMap<String, usize>> {
        let conn = self.open_read_only()?;
        let mut stmt =
            conn.prepare_cached("SELECT kind, COUNT(*) FROM symbols GROUP BY kind ORDER BY kind")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut m = BTreeMap::new();
        for row in rows {
            let (k, c) = row?;
            m.insert(k, c as usize);
        }
        Ok(m)
    }

    /// Sample symbols for `query_suggestions` example queries, ordered by
    /// PageRank-relevant kinds first (functions, classes, methods, structs,
    /// traits) so operator-facing suggestions land on real API surface rather
    /// than top-of-tree constants. Returns `(name, kind, file_path)`.
    pub fn symbol_samples_for_suggestions(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<(String, String, String)>> {
        let conn = self.open_read_only()?;
        let lim = limit.clamp(1, 64) as i64;
        let mut stmt = conn.prepare_cached(
            r#"SELECT name, kind, file_path FROM symbols
               WHERE kind IN ('function','method','class','struct','trait','interface','enum')
               ORDER BY file_path, name
               LIMIT ?1"#,
        )?;
        let rows = stmt.query_map(rusqlite::params![lim], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut v = Vec::new();
        for row in rows {
            v.push(row?);
        }
        Ok(v)
    }

    /// Return all symbols with basic metadata needed for dead-code / hotspot analyses.
    pub fn all_symbols_for_analysis(&self) -> anyhow::Result<Vec<AnalysisSymbol>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare_cached(
            r#"SELECT id, kind, name, file_path, start_line, end_line, source_text
               FROM symbols
               ORDER BY file_path, start_line"#,
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(AnalysisSymbol {
                id: row.get(0)?,
                kind: row.get(1)?,
                name: row.get(2)?,
                file_path: row.get(3)?,
                start_line: row.get(4)?,
                end_line: row.get(5)?,
                source_text: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Cached, shared-Arc variant of [`IndexStore::all_symbols_for_analysis`].
    /// First call loads from SQL; subsequent calls on the same store instance
    /// (including clones) return a cheap `Arc::clone`. Cleared automatically
    /// after each `apply_index_with_deps` commit.
    pub fn all_symbols_cached(&self) -> anyhow::Result<std::sync::Arc<Vec<AnalysisSymbol>>> {
        let mut guard = self
            .cached_all_symbols
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = guard.as_ref() {
            return Ok(existing.clone());
        }
        let loaded = std::sync::Arc::new(self.all_symbols_for_analysis()?);
        *guard = Some(loaded.clone());
        Ok(loaded)
    }

    /// Count how many *other* symbols reference the given `needle` in their source text.
    /// Returns the count and the distinct set of files containing references.
    pub fn reference_count_and_files(
        &self,
        needle: &str,
        exclude_id: &str,
    ) -> anyhow::Result<(usize, Vec<String>)> {
        if needle.is_empty() || needle.len() < 2 {
            return Ok((0, Vec::new()));
        }
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare_cached(
            r#"SELECT DISTINCT file_path
               FROM symbols
               WHERE id != ?1 AND instr(source_text, ?2) > 0"#,
        )?;
        let rows = stmt.query_map(rusqlite::params![exclude_id, needle], |row| {
            row.get::<_, String>(0)
        })?;
        let files: Vec<String> = rows.collect::<Result<Vec<_>, _>>()?;
        let count = files.len();
        Ok((count, files))
    }

    /// Count how many symbols in the repo reference the given needle.
    pub fn symbol_reference_count(&self, needle: &str, exclude_id: &str) -> anyhow::Result<usize> {
        if needle.is_empty() || needle.len() < 2 {
            return Ok(0);
        }
        let conn = self.open_read_only()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(1) FROM symbols WHERE id != ?1 AND instr(source_text, ?2) > 0",
            rusqlite::params![exclude_id, needle],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Bulk version of [`symbol_reference_count`] that scans every symbol's
    /// `source_text` exactly once and returns the set of candidate names that
    /// were mentioned in some other symbol's body. Designed for the
    /// dead-code symbol pass, which previously ran one `instr()` query per
    /// candidate — quadratic in symbol count and pathologically slow on a
    /// few-thousand-symbol repo. Caller passes the candidates as a slice of
    /// `(symbol_id, name)`; the returned set contains every `name` for which
    /// at least one symbol with a different `id` mentions `name` in its
    /// `source_text`.
    ///
    /// Implementation: tokenizes each body once into the set of identifier
    /// tokens it contains, then intersects against the candidate-name set.
    /// O(total_body_bytes + candidates) instead of O(bodies × candidates).
    pub fn names_mentioned_outside_self(
        &self,
        candidates: &[(String, String)],
    ) -> anyhow::Result<FxHashSet<String>> {
        use rustc_hash::{FxHashMap, FxHashSet};
        let mut by_name: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();
        for (id, name) in candidates {
            if name.len() < 2 {
                continue;
            }
            by_name.entry(name.clone()).or_default().insert(id.clone());
        }
        if by_name.is_empty() {
            return Ok(FxHashSet::default());
        }
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare_cached(
            "SELECT id, source_text FROM symbols WHERE source_text IS NOT NULL AND source_text != ''",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut found: FxHashSet<String> = FxHashSet::default();
        for row in rows {
            let (sid, body) = row?;
            if body.is_empty() {
                continue;
            }
            // Identifier-token scan: yields each maximal `[A-Za-z_][A-Za-z0-9_]*`
            // run once per body, dedup'd via a per-body set so repeated calls
            // to the same name don't pay extra FxHashMap lookups.
            let mut seen_tokens: FxHashSet<&str> = FxHashSet::default();
            let bytes = body.as_bytes();
            let mut i = 0usize;
            while i < bytes.len() {
                let b = bytes[i];
                if !(b.is_ascii_alphabetic() || b == b'_') {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let tok = &body[start..i];
                if tok.len() >= 2 {
                    seen_tokens.insert(tok);
                }
            }
            for tok in seen_tokens {
                if found.contains(tok) {
                    continue;
                }
                if let Some(owners) = by_name.get(tok)
                    && !owners.contains(sid.as_str())
                {
                    found.insert(tok.to_string());
                }
            }
        }
        Ok(found)
    }

    /// Store package names in the meta table.
    pub fn set_package_names(&self, names: &[String]) -> anyhow::Result<()> {
        let conn = self.open_connection()?;
        let json = serde_json::to_string(names)?;
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('package_names', ?1)",
            [&json],
        )?;
        Ok(())
    }

    /// Retrieve package names from the meta table. Returns empty vec if absent.
    pub fn get_package_names(&self) -> anyhow::Result<Vec<String>> {
        let conn = self.open_read_only()?;
        let result: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'package_names'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match result {
            Some(json) => Ok(serde_json::from_str(&json).unwrap_or_default()),
            None => Ok(Vec::new()),
        }
    }

    /// Get or set a meta key-value pair.
    pub fn get_meta(&self, key: &str) -> anyhow::Result<Option<String>> {
        let conn = self.open_read_only()?;
        let result: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(result)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> anyhow::Result<()> {
        let conn = self.open_connection()?;
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    /// Get all raw file contents from the index (reading from disk via root_path).
    pub fn all_file_contents(&self) -> anyhow::Result<Vec<(String, String)>> {
        let root = self.root_path()?;
        let files = self.all_file_paths()?;
        let mut results = Vec::new();
        for file_path in files {
            let full_path = root.join(&file_path);
            if let Ok(content) = std::fs::read_to_string(&full_path) {
                results.push((file_path, content));
            }
        }
        Ok(results)
    }

    /// Read every reachability edge whose kind is in `kinds`. Empty
    /// `kinds` returns the whole table.
    pub fn reachability_edges(&self, kinds: &[EdgeKind]) -> anyhow::Result<Vec<StructuralEdge>> {
        let conn = self.open_read_only()?;
        super::reachability_edges::all_edges_for_kinds(&conn, kinds).map_err(Into::into)
    }

    pub fn file_max_end_lines(&self) -> anyhow::Result<FxHashMap<String, u64>> {
        let conn = self.open_read_only()?;
        let mut stmt =
            conn.prepare_cached("SELECT file_path, MAX(end_line) FROM symbols GROUP BY file_path")?;
        let rows = stmt.query_map([], |row| {
            let path: String = row.get(0)?;
            let end_line: i64 = row.get(1)?;
            Ok((path, end_line as u64))
        })?;
        let mut map = FxHashMap::default();
        for row in rows {
            let (path, end_line) = row?;
            map.insert(path, end_line);
        }
        Ok(map)
    }
}
