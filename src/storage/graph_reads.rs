//! Repo-level graph reads, file listings, and staleness.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use super::IndexStore;

impl IndexStore {
    /// Return the file-level dependency adjacency list from stored edges.
    pub fn dependency_graph(
        &self,
    ) -> anyhow::Result<std::collections::HashMap<String, Vec<String>>> {
        let conn = self.open_read_only()?;

        // Start with all files as nodes (even those with no edges)
        let mut adj: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT file_path FROM files")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            for row in rows {
                adj.entry(row?).or_default();
            }
        }

        // Add edges
        {
            let mut stmt = conn.prepare("SELECT source_file, target_file FROM dependency_edges")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (src, tgt) = row?;
                adj.entry(src).or_default().push(tgt);
            }
        }

        Ok(adj)
    }

    /// Return all symbols with their source text, for call graph building.
    pub fn all_symbols_with_source(&self) -> anyhow::Result<Vec<crate::dep_graph::SymbolInfo>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare(
            "SELECT id, kind, extraction_tier, name, file_path, start_line, end_line, source_text FROM symbols ORDER BY file_path, start_line",
        )?;
        let rows = stmt.query_map([], |row| {
            let tier_str: String = row.get(2)?;
            let extraction_tier = crate::core::symbols::ExtractionTier::from_db(&tier_str)
                .unwrap_or(crate::core::symbols::ExtractionTier::ManualOverride);
            Ok(crate::dep_graph::SymbolInfo {
                id: row.get(0)?,
                kind: row.get(1)?,
                name: row.get(3)?,
                file: row.get(4)?,
                line: row.get::<_, u32>(5)?,
                end_line: row.get::<_, u32>(6)?,
                source: row.get(7)?,
                extraction_tier,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// List all indexed file paths.
    pub fn all_file_paths(&self) -> anyhow::Result<Vec<String>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare("SELECT file_path FROM files ORDER BY file_path")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Check if a file exists in the index.
    pub fn file_exists(&self, file_path: &str) -> anyhow::Result<bool> {
        let conn = self.open_read_only()?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(1) FROM files WHERE file_path = ?1",
            [file_path],
            |row| row.get(0),
        )?;
        Ok(n > 0)
    }

    /// Rollup stats for all indexed files (paths are repo-relative with `/`).
    pub fn repository_rollup(&self) -> anyhow::Result<Value> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare("SELECT file_path, language FROM files ORDER BY file_path")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut languages = BTreeMap::new();
        let mut directories = BTreeMap::new();
        let mut file_count = 0usize;
        for row in rows {
            let (path, lang) = row?;
            file_count += 1;
            *languages.entry(lang).or_insert(0) += 1;
            if let Some((first, rest)) = path.split_once('/') {
                let _ = rest;
                let key = format!("{first}/");
                *directories.entry(key).or_insert(0) += 1;
            } else {
                *directories.entry("./".to_string()).or_insert(0) += 1;
            }
        }
        Ok(serde_json::json!({
            "file_count": file_count,
            "languages": languages,
            "directories": directories,
        }))
    }

    /// Fast stat-based staleness check. Returns true if any indexed file has
    /// a different size or mtime on disk, or if new source files exist that
    /// aren't in the index. Does NOT read file contents or compute hashes.
    pub fn is_stale<F>(&self, root_path: &Path, is_source_file: F) -> anyhow::Result<bool>
    where
        F: Fn(&Path) -> bool,
    {
        let conn = self.open_read_only()?;
        crate::db::check_staleness(&conn, root_path, "files", |rel| {
            is_source_file(Path::new(rel))
        })
    }

    pub fn file_count(&self) -> anyhow::Result<usize> {
        let conn = self.open_read_only()?;
        Ok(conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get::<_, i64>(0))? as usize)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}
