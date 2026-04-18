//! Analysis-oriented aggregations, reference counts, and Python import heuristics.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;

use rusqlite::OptionalExtension;

use crate::indexing::imports::python_top_level_import_stems;

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

    /// Python-only: files most often imported by other indexed `.py` files.
    /// Uses structural import extraction (`indexing::imports`) plus filename-stem resolution.
    pub fn python_most_imported_files(&self, limit: usize) -> anyhow::Result<Vec<(String, usize)>> {
        let root = self.root_path()?;
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare_cached("SELECT file_path FROM files")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;

        let mut stem_to_path: HashMap<String, String> = HashMap::new();
        let mut py_files: Vec<String> = Vec::new();
        for row in rows {
            let path = row?;
            if path.ends_with(".py") {
                py_files.push(path.clone());
            }
            if let Some(stem) = Path::new(&path).file_stem().and_then(|s| s.to_str()) {
                stem_to_path.insert(stem.to_string(), path);
            }
        }
        py_files.sort();
        let py_set: HashSet<String> = py_files.iter().cloned().collect();
        let mut by_target: HashMap<String, HashSet<String>> = HashMap::new();

        for importer in &py_files {
            let abs = root.join(importer);
            let Ok(content) = fs::read_to_string(&abs) else {
                continue;
            };
            for target in python_top_level_import_stems(&content, importer) {
                if let Some(resolved) = stem_to_path.get(&target) {
                    if resolved == importer || !py_set.contains(resolved) {
                        continue;
                    }
                    by_target
                        .entry(resolved.clone())
                        .or_default()
                        .insert(importer.clone());
                }
            }
        }

        let mut ranked: Vec<(String, usize)> =
            by_target.into_iter().map(|(k, v)| (k, v.len())).collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        ranked.truncate(limit.clamp(1, 50));
        Ok(ranked)
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

    pub fn file_max_end_lines(&self) -> anyhow::Result<HashMap<String, u64>> {
        let conn = self.open_read_only()?;
        let mut stmt =
            conn.prepare_cached("SELECT file_path, MAX(end_line) FROM symbols GROUP BY file_path")?;
        let rows = stmt.query_map([], |row| {
            let path: String = row.get(0)?;
            let end_line: i64 = row.get(1)?;
            Ok((path, end_line as u64))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (path, end_line) = row?;
            map.insert(path, end_line);
        }
        Ok(map)
    }
}
