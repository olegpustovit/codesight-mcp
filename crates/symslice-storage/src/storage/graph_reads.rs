//! Repo-level graph reads, file listings, and staleness.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use rustc_hash::FxHashMap;
use serde_json::Value;

use crate::core::symbols::FactQuality;

use super::IndexStore;

/// Process-wide cache of derived analysis results keyed on
/// `(canonical_db_path, index_version)`. Survives `IndexStore::open_existing`
/// returning a fresh handle on every dispatch — without it, the per-`IndexStore`
/// `cached_pagerank` field cannot help because every request opens a brand new
/// store. Entries from older index versions are evicted on the next write.
type PagerankKey = (PathBuf, u32);
type PagerankCache = Mutex<FxHashMap<PagerankKey, Arc<FxHashMap<String, f64>>>>;
type SymbolsKey = (PathBuf, u32);
type SymbolsCache = Mutex<FxHashMap<SymbolsKey, Arc<Vec<crate::storage::SymbolSourceRecord>>>>;

fn pagerank_cache() -> &'static PagerankCache {
    static CACHE: OnceLock<PagerankCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(FxHashMap::default()))
}

fn symbols_with_source_cache() -> &'static SymbolsCache {
    static CACHE: OnceLock<SymbolsCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(FxHashMap::default()))
}

impl IndexStore {
    fn parse_import_edge_quality(value: &str) -> FactQuality {
        FactQuality::from_db(value).unwrap_or(FactQuality::BaselineStructural)
    }

    /// Return the file-level dependency adjacency list from stored edges (all import tiers).
    pub fn dependency_graph(&self) -> anyhow::Result<FxHashMap<String, Vec<String>>> {
        Ok(self
            .dependency_graph_with_import_quality()?
            .into_iter()
            .map(|(src, targets)| (src, targets.into_iter().map(|(path, _)| path).collect()))
            .collect())
    }

    /// Stored dependency edges with [`FactQuality`] from import extraction (see REFACTOR_PLAN).
    pub fn dependency_graph_with_import_quality(
        &self,
    ) -> anyhow::Result<FxHashMap<String, Vec<(String, FactQuality)>>> {
        // RW connection so schema migration runs before reads on older indexes.
        let conn = self.open_connection()?;

        let mut adj: FxHashMap<String, Vec<(String, FactQuality)>> = FxHashMap::default();
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
    pub fn dependency_graph_strict(&self) -> anyhow::Result<FxHashMap<String, Vec<String>>> {
        let detailed = self.dependency_graph_with_import_quality()?;
        let mut out: FxHashMap<String, Vec<String>> = FxHashMap::default();
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
    /// Cached variant of [`Self::all_symbols_with_source`]. Backed by a
    /// process-global map keyed on `(db_path, index_version)` so a fresh
    /// `IndexStore::open_existing` (the default at every dispatch) reuses
    /// the same `Arc` snapshot. A bump in `index_version` after
    /// `apply_index_with_deps` invalidates older entries naturally because
    /// the lookup key changes.
    pub fn all_symbols_with_source_cached(
        &self,
    ) -> anyhow::Result<Arc<Vec<crate::storage::SymbolSourceRecord>>> {
        let version = self.current_index_version().unwrap_or(0);
        let key = (self.db_path.clone(), version);
        if let Ok(map) = symbols_with_source_cache().lock()
            && let Some(existing) = map.get(&key)
        {
            return Ok(existing.clone());
        }
        let loaded = Arc::new(self.all_symbols_with_source()?);
        if let Ok(mut map) = symbols_with_source_cache().lock() {
            map.retain(|(p, _), _| p != &self.db_path);
            map.insert(key, loaded.clone());
        }
        Ok(loaded)
    }

    /// Returns a cached map of `file_path -> pagerank_score` over the file
    /// dependency graph. PetGraph's 50-iteration page-rank costs ~1.6s on
    /// a 565-file repo; without this cache `assemble_context context_bundle`
    /// pays that cost on every call. The cache is process-global and keyed
    /// on `(db_path, index_version)` so it survives `IndexStore::open_existing`
    /// returning a fresh handle. The caller passes the compute closure so
    /// the analysis-layer PageRank impl stays out of `symslice-storage`.
    pub fn pagerank_scores_cached_with<F>(
        &self,
        compute: F,
    ) -> anyhow::Result<Arc<rustc_hash::FxHashMap<String, f64>>>
    where
        F: FnOnce(
            &rustc_hash::FxHashMap<String, Vec<String>>,
        ) -> rustc_hash::FxHashMap<String, f64>,
    {
        let version = self.current_index_version().unwrap_or(0);
        let key = (self.db_path.clone(), version);
        if let Ok(map) = pagerank_cache().lock()
            && let Some(existing) = map.get(&key)
        {
            return Ok(existing.clone());
        }
        let adj = self.dependency_graph()?;
        let scores = compute(&adj);
        let arc = Arc::new(scores);
        if let Ok(mut map) = pagerank_cache().lock() {
            map.retain(|(p, _), _| p != &self.db_path);
            map.insert(key, arc.clone());
        }
        Ok(arc)
    }

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
    /// stored dependency edge table. Returns raw `(file_path, importer_count)`
    /// pairs ordered by importer count desc; per-language manifest / build-script
    /// filtering is the engine layer's job (see `is_outline_noise_basename`).
    pub fn most_imported_files(&self, limit: usize) -> anyhow::Result<Vec<(String, usize)>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare_cached(
            "SELECT target_file, COUNT(DISTINCT source_file) AS importers
             FROM dependency_edges
             WHERE target_file != source_file
             GROUP BY target_file
             ORDER BY importers DESC, target_file ASC
             LIMIT ?1",
        )?;
        let cap = limit.clamp(1, 100) as i64;
        let rows = stmt.query_map([cap], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
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
