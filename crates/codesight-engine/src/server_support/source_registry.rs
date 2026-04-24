//! Process-wide cache of small, frequently re-read per-source metadata
//! (root path, package names, file paths, repo summary).
//!
//! Cross-source tools (`inspect_graph` with `cross_repo=true`,
//! `catalog_sources`) loop over N registered sources and call 1–4
//! cheap-but-non-free `IndexStore` methods on each. Each call opens
//! a fresh read-only SQLite connection through `open_read_only`, runs a
//! small indexed query, and decodes a handful of rows. On a 10-source setup
//! the overhead stacks to 50–100 ms *just for the opens and stale checks*.
//!
//! The batch-reads API (`file_outlines_batch`, etc.) doesn't help here:
//! `WHERE IN (?, ?, …)` collapses lookups against ONE connection, but the
//! cross-source workload has N *separate* database files. The solution is
//! an application-level snapshot keyed by `source_id` and fingerprinted on
//! `repo_info.indexed_at`.
//!
//! Freshness: every `snapshot()` does one tiny indexed `SELECT indexed_at`
//! against the source and compares it to the cached fingerprint. Match →
//! return cheap `Arc::clone`. Drift → refresh from the passed-in store.
//! Explicit `invalidate(sid)` is called after ingest / `manage_indexes
//! invalidate|delete|rebuild` so the *next* reader refreshes without
//! waiting for the fingerprint check.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use codesight_storage::storage::{IndexStore, RepoMetadata};

/// Per-source snapshot returned from [`SourceRegistry::snapshot`]. Fields
/// are the ones every cross-source loop body reaches for today. Intentionally
/// excludes bulky payloads (`all_file_contents`, `all_symbols_for_analysis`)
/// that would blow the memory budget for users with many registered sources.
#[derive(Debug, Clone)]
pub struct CachedSource {
    pub source_id: String,
    pub root_path: PathBuf,
    pub package_names: Vec<String>,
    pub all_file_paths: Vec<String>,
    pub repo_summary: RepoMetadata,
    /// Fingerprint compared against `repo_info.indexed_at` on every read.
    pub indexed_at: String,
}

/// Process-wide, `Send + Sync`. One per `CodeService`.
#[derive(Debug, Default)]
pub struct SourceRegistry {
    inner: RwLock<HashMap<String, Arc<CachedSource>>>,
}

impl SourceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a cached snapshot for `source_id`, refreshing from `store` if
    /// the on-disk `indexed_at` has drifted from the cache (or there is no
    /// cache yet). The passed-in store is used as the sole data source —
    /// callers are responsible for passing one that has already been
    /// resolved through `ensure_repo_store` (or equivalent) so background
    /// reindex kicks + auto-index-on-path can happen.
    pub fn snapshot(
        &self,
        source_id: &str,
        store: &IndexStore,
    ) -> anyhow::Result<Arc<CachedSource>> {
        let on_disk = store
            .repo_summary()
            .map(|s| s.indexed_at.clone())
            .unwrap_or_default();

        if let Ok(map) = self.inner.read()
            && let Some(existing) = map.get(source_id)
            && existing.indexed_at == on_disk
            && !on_disk.is_empty()
        {
            return Ok(existing.clone());
        }

        let fresh = Arc::new(Self::load(source_id, store, on_disk)?);
        if let Ok(mut map) = self.inner.write() {
            map.insert(source_id.to_string(), fresh.clone());
        }
        Ok(fresh)
    }

    /// Drop the cached entry for `source_id`. Call after commits that change
    /// the source's data (ingest, `manage_indexes invalidate|delete|rebuild`).
    /// Missing entries are a no-op.
    pub fn invalidate(&self, source_id: &str) {
        if let Ok(mut map) = self.inner.write() {
            map.remove(source_id);
        }
    }

    /// Drop all cached entries. Used on config reload.
    pub fn clear(&self) {
        if let Ok(mut map) = self.inner.write() {
            map.clear();
        }
    }

    fn load(
        source_id: &str,
        store: &IndexStore,
        indexed_at: String,
    ) -> anyhow::Result<CachedSource> {
        let root_path = store.root_path()?;
        let repo_summary = store.repo_summary()?;
        let package_names = store.get_package_names().unwrap_or_default();
        let all_file_paths = store.all_file_paths().unwrap_or_default();
        Ok(CachedSource {
            source_id: source_id.to_string(),
            root_path,
            package_names,
            all_file_paths,
            repo_summary,
            indexed_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use codesight_storage::core::pipeline::{FileFingerprint, IndexedFileUpdate, StoredSymbol};
    use tempfile::tempdir;

    fn fingerprint(path: &str, size: u64, mtime: u64) -> FileFingerprint {
        FileFingerprint {
            file_path: path.to_string(),
            language: codesight_core::RUST,
            size_bytes: size,
            modified_ns: mtime,
            content_hash: "deadbeef".to_string(),
        }
    }

    fn seed_store(root: &std::path::Path, db_path: std::path::PathBuf) -> IndexStore {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        let store = IndexStore::open(db_path).unwrap();
        let ff = fingerprint("a.rs", 10, 1);
        let update = IndexedFileUpdate {
            file_path: "a.rs".to_string(),
            language: codesight_core::RUST,
            size_bytes: 10,
            modified_ns: 1,
            content_hash: "deadbeef".to_string(),
            symbols: vec![StoredSymbol {
                id: "a.rs::a#function".to_string(),
                extraction_tier: codesight_storage::core::symbols::ExtractionTier::CuratedQueryPack,
                language: codesight_core::RUST,
                file_path: "a.rs".to_string(),
                qualified_name: "a".to_string(),
                name: "a".to_string(),
                kind: codesight_storage::core::symbols::SymbolKind::Function,
                signature: "fn a() {}".to_string(),
                summary: String::new(),
                start_line: 1,
                end_line: 1,
                start_byte: 0,
                end_byte: 9,
                source_hash: "deadbeef".to_string(),
                source_text: "fn a() {}".to_string(),
                file_size_bytes: 10,
            }],
        };
        store
            .apply_code_index(
                "unit/test",
                root,
                "2026-01-01T00:00:00Z",
                1,
                &[ff],
                &[update],
                &[],
                &[],
            )
            .unwrap();
        store
    }

    #[test]
    fn snapshot_miss_populates_cache_and_returns_loaded_fields() {
        let ws = tempdir().unwrap();
        let root = ws.path().join("repo");
        let db = ws
            .path()
            .join("idx")
            .join("unit_test")
            .join("index.sqlite3");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let store = seed_store(&root, db);

        let reg = SourceRegistry::new();
        let snap = reg.snapshot("unit/test", &store).unwrap();
        assert_eq!(snap.source_id, "unit/test");
        assert_eq!(snap.indexed_at, "2026-01-01T00:00:00Z");
        assert!(snap.all_file_paths.iter().any(|f| f == "a.rs"));
        // Compare via canonicalize on both sides — macOS returns
        // `/private/var/folders/...` from `canonicalize` but `/var/folders/...`
        // from `tempdir()`, and IndexStore stores whichever the ingester saw.
        let expected = root.canonicalize().unwrap_or(root.clone());
        let got = snap
            .root_path
            .canonicalize()
            .unwrap_or(snap.root_path.clone());
        assert_eq!(got, expected);
    }

    #[test]
    fn snapshot_hit_returns_same_arc_without_reload() {
        let ws = tempdir().unwrap();
        let root = ws.path().join("repo");
        let db = ws
            .path()
            .join("idx")
            .join("unit_test")
            .join("index.sqlite3");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let store = seed_store(&root, db);

        let reg = SourceRegistry::new();
        let a = reg.snapshot("unit/test", &store).unwrap();
        let b = reg.snapshot("unit/test", &store).unwrap();
        assert!(Arc::ptr_eq(&a, &b), "second snapshot should reuse the Arc");
    }

    #[test]
    fn fingerprint_drift_forces_reload() {
        let ws = tempdir().unwrap();
        let root = ws.path().join("repo");
        let db = ws
            .path()
            .join("idx")
            .join("unit_test")
            .join("index.sqlite3");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let store = seed_store(&root, db.clone());

        let reg = SourceRegistry::new();
        let a = reg.snapshot("unit/test", &store).unwrap();

        // Simulate a reindex: bump indexed_at. The registry must refresh on
        // the next snapshot even though we never called invalidate().
        let ff = fingerprint("a.rs", 10, 2);
        store
            .apply_code_index(
                "unit/test",
                &root,
                "2026-02-02T00:00:00Z",
                2,
                &[ff],
                &[],
                &[],
                &[],
            )
            .unwrap();

        let b = reg.snapshot("unit/test", &store).unwrap();
        assert!(
            !Arc::ptr_eq(&a, &b),
            "fingerprint drift should rebuild the Arc"
        );
        assert_eq!(b.indexed_at, "2026-02-02T00:00:00Z");
    }

    #[test]
    fn invalidate_removes_entry() {
        let ws = tempdir().unwrap();
        let root = ws.path().join("repo");
        let db = ws
            .path()
            .join("idx")
            .join("unit_test")
            .join("index.sqlite3");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let store = seed_store(&root, db);

        let reg = SourceRegistry::new();
        let a = reg.snapshot("unit/test", &store).unwrap();
        reg.invalidate("unit/test");
        let b = reg.snapshot("unit/test", &store).unwrap();
        assert!(
            !Arc::ptr_eq(&a, &b),
            "after invalidate, next snapshot is a fresh Arc"
        );
    }
}
