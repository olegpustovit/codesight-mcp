//! Repo-level graph reads, file listings, and staleness.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::core::symbols::FactQuality;

use super::IndexStore;

impl IndexStore {
    fn parse_import_edge_quality(value: &str) -> FactQuality {
        FactQuality::from_db(value).unwrap_or(FactQuality::BaselineStructural)
    }

    /// Return the file-level dependency adjacency list from stored edges (all import tiers).
    pub fn dependency_graph(
        &self,
    ) -> anyhow::Result<std::collections::HashMap<String, Vec<String>>> {
        Ok(self
            .dependency_graph_with_import_quality()?
            .into_iter()
            .map(|(src, targets)| (src, targets.into_iter().map(|(path, _)| path).collect()))
            .collect())
    }

    /// Stored dependency edges with [`FactQuality`] from import extraction (see REFACTOR_PLAN).
    pub fn dependency_graph_with_import_quality(
        &self,
    ) -> anyhow::Result<std::collections::HashMap<String, Vec<(String, FactQuality)>>> {
        // RW connection so schema migration runs before reads on older indexes.
        let conn = self.open_connection()?;

        let mut adj: std::collections::HashMap<String, Vec<(String, FactQuality)>> =
            std::collections::HashMap::new();
        {
            let mut stmt = conn.prepare_cached("SELECT file_path FROM files")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            for row in rows {
                adj.entry(row?).or_default();
            }
        }
        {
            let mut stmt = conn.prepare_cached(
                "SELECT source_file, target_file, import_fact_quality FROM dependency_edges",
            )?;
            let rows = stmt.query_map([], |row| {
                let src = row.get::<_, String>(0)?;
                let tgt = row.get::<_, String>(1)?;
                let q = row.get::<_, String>(2)?;
                Ok((src, tgt, Self::parse_import_edge_quality(&q)))
            })?;
            for row in rows {
                let (src, tgt, q) = row?;
                adj.entry(src).or_default().push((tgt, q));
            }
        }

        Ok(adj)
    }

    /// Adjacency built only from import edges marked suitable for strict graph analysis.
    pub fn dependency_graph_strict(
        &self,
    ) -> anyhow::Result<std::collections::HashMap<String, Vec<String>>> {
        let detailed = self.dependency_graph_with_import_quality()?;
        let mut out: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for (src, targets) in detailed {
            let keep: Vec<String> = targets
                .into_iter()
                .filter(|(_, q)| q.suitable_for_strict_analysis())
                .map(|(path, _)| path)
                .collect();
            out.insert(src, keep);
        }
        Ok(out)
    }

    /// Return all symbols with their source text, for call graph building.
    pub fn all_symbols_with_source(
        &self,
    ) -> anyhow::Result<Vec<crate::storage::SymbolSourceRecord>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare_cached(
            "SELECT id, kind, extraction_tier, name, file_path, start_line, end_line, source_text FROM symbols ORDER BY file_path, start_line",
        )?;
        let rows = stmt.query_map([], |row| {
            let tier_str: String = row.get(2)?;
            let extraction_tier = crate::core::symbols::ExtractionTier::from_db(&tier_str)
                .unwrap_or(crate::core::symbols::ExtractionTier::ManualOverride);
            Ok(crate::storage::SymbolSourceRecord {
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
        let mut stmt = conn.prepare_cached("SELECT file_path FROM files ORDER BY file_path")?;
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
        let mut stmt =
            conn.prepare_cached("SELECT file_path, language FROM files ORDER BY file_path")?;
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

    /// Language-agnostic "most imported files" ranked by in-degree from the
    /// stored dependency edge table. Excludes build scripts and tool manifests
    /// (`is_outline_noise`). Returns `(file_path, importer_count)` pairs.
    pub fn most_imported_files(&self, limit: usize) -> anyhow::Result<Vec<(String, usize)>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare_cached(
            "SELECT target_file, COUNT(DISTINCT source_file) AS importers
             FROM dependency_edges
             WHERE target_file != source_file
             GROUP BY target_file
             ORDER BY importers DESC, target_file ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
        })?;
        let mut out: Vec<(String, usize)> = Vec::new();
        let cap = limit.clamp(1, 100);
        for row in rows {
            let (path, n) = row?;
            if crate::storage::is_outline_noise(&path) {
                continue;
            }
            out.push((path, n));
            if out.len() >= cap {
                break;
            }
        }
        Ok(out)
    }

    /// Fast stat-based staleness check. Returns true if any indexed file has
    /// a different size or mtime on disk, or if new source files exist that
    /// aren't in the index. Does NOT read file contents or compute hashes.
    pub fn is_stale<F>(&self, root_path: &Path, is_source_file: F) -> anyhow::Result<bool>
    where
        F: Fn(&Path) -> bool,
    {
        let conn = self.open_read_only()?;
        Ok(!crate::db::check_staleness_detail(
            &conn,
            root_path,
            "files",
            |rel| is_source_file(Path::new(rel)),
            1,
        )?
        .is_empty())
    }

    pub fn stale_paths<F>(
        &self,
        root_path: &Path,
        is_source_file: F,
        limit: usize,
    ) -> anyhow::Result<Vec<String>>
    where
        F: Fn(&Path) -> bool,
    {
        let conn = self.open_read_only()?;
        Ok(crate::db::check_staleness_detail(
            &conn,
            root_path,
            "files",
            |rel| is_source_file(Path::new(rel)),
            limit,
        )?)
    }

    pub fn file_count(&self) -> anyhow::Result<usize> {
        let conn = self.open_read_only()?;
        Ok(conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get::<_, i64>(0))? as usize)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}
