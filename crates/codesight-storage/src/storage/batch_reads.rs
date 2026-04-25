//! Batch variants of single-key lookups that were previously called in
//! loops (file-keyed / symbol-keyed). Each method issues one SQL query
//! via `WHERE x IN (?, ?, ...)` and chunks safely under SQLite's
//! `SQLITE_MAX_VARIABLE_NUMBER` (defaults around 32k on modern builds,
//! we use 500 to be safe under every shipped config).
//!
//! Kill-class for N+1 bugs: call-sites that need per-key data for many
//! keys should call the `_batch` variant once, not the single-key
//! variant in a loop. The single-key methods are kept for genuinely
//! one-off lookups (e.g. `CodeService::resolve_stores`).

use rustc_hash::{FxHashMap, FxHashSet};
use std::path::PathBuf;

use rusqlite::OptionalExtension;

use crate::storage::SymbolRecord;

use super::IndexStore;
use super::row_utils::symbol_record_from_row;

/// Minimal enriched view of a symbol — the union of
/// `symbol_names_by_id` + `symbol_signature_by_id` + file language,
/// used by `assemble_context mode=context_bundle` to dress each ref.
#[derive(Debug, Clone)]
pub struct SymbolEnriched {
    pub name: String,
    pub qualified_name: String,
    pub signature: String,
    pub file_path: String,
    pub language: Option<String>,
}

/// Safe chunk size for `?`-parameter binding. SQLite's compile-time
/// `SQLITE_MAX_VARIABLE_NUMBER` is 32766 on modern builds, but older
/// builds and some ABI compat configs drop it to 999 — 500 is a boring,
/// reliable ceiling.
const BATCH_CHUNK: usize = 500;

fn placeholders(n: usize) -> String {
    let mut s = String::with_capacity(n * 2);
    for i in 0..n {
        if i > 0 {
            s.push(',');
        }
        s.push('?');
    }
    s
}

impl IndexStore {
    /// Batch-variant of [`IndexStore::file_outline`]. Returns a
    /// `path → outline` map; missing paths are absent from the map.
    /// One SQL round trip per chunk of 500 paths.
    pub fn file_outlines_batch(
        &self,
        paths: &[&str],
    ) -> anyhow::Result<FxHashMap<String, Vec<serde_json::Value>>> {
        let mut out: FxHashMap<String, Vec<serde_json::Value>> = FxHashMap::default();
        if paths.is_empty() {
            return Ok(out);
        }
        let conn = self.open_read_only()?;
        for chunk in paths.chunks(BATCH_CHUNK) {
            let sql = format!(
                "SELECT file_path, id, kind, name, signature, start_line, end_line \
                 FROM symbols \
                 WHERE file_path IN ({}) \
                 ORDER BY file_path, start_line",
                placeholders(chunk.len())
            );
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    serde_json::json!({
                        "id": row.get::<_, String>(1)?,
                        "kind": row.get::<_, String>(2)?,
                        "name": row.get::<_, String>(3)?,
                        "signature": row.get::<_, String>(4)?,
                        "line": row.get::<_, u32>(5)?,
                        "end_line": row.get::<_, u32>(6)?,
                    }),
                ))
            })?;
            for row in rows {
                let (path, sym) = row?;
                out.entry(path).or_default().push(sym);
            }
        }
        Ok(out)
    }

    /// Batch-variant of [`IndexStore::file_language`]. Returns a
    /// `path → language` map; missing paths map to absent keys.
    pub fn file_languages_batch(&self, paths: &[&str]) -> anyhow::Result<FxHashMap<String, String>> {
        let mut out: FxHashMap<String, String> = FxHashMap::default();
        if paths.is_empty() {
            return Ok(out);
        }
        let conn = self.open_read_only()?;
        for chunk in paths.chunks(BATCH_CHUNK) {
            let sql = format!(
                "SELECT file_path, language FROM files WHERE file_path IN ({})",
                placeholders(chunk.len())
            );
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (p, lang) = row?;
                out.insert(p, lang);
            }
        }
        Ok(out)
    }

    /// Batch-variant of [`IndexStore::file_bytes`].
    pub fn file_bytes_batch(&self, paths: &[&str]) -> anyhow::Result<FxHashMap<String, u64>> {
        let mut out: FxHashMap<String, u64> = FxHashMap::default();
        if paths.is_empty() {
            return Ok(out);
        }
        let conn = self.open_read_only()?;
        for chunk in paths.chunks(BATCH_CHUNK) {
            let sql = format!(
                "SELECT file_path, size_bytes FROM files WHERE file_path IN ({})",
                placeholders(chunk.len())
            );
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (p, n) = row?;
                out.insert(p, n as u64);
            }
        }
        Ok(out)
    }

    /// Batch-variant of [`IndexStore::file_exists`]. Returns the set of
    /// input paths that are present in the `files` table.
    pub fn files_exist_batch(&self, paths: &[&str]) -> anyhow::Result<FxHashSet<String>> {
        let mut out: FxHashSet<String> = FxHashSet::default();
        if paths.is_empty() {
            return Ok(out);
        }
        let conn = self.open_read_only()?;
        for chunk in paths.chunks(BATCH_CHUNK) {
            let sql = format!(
                "SELECT file_path FROM files WHERE file_path IN ({})",
                placeholders(chunk.len())
            );
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                row.get::<_, String>(0)
            })?;
            for row in rows {
                out.insert(row?);
            }
        }
        Ok(out)
    }

    /// Returns the subset of `paths` that emit at least one symbol whose
    /// `kind` is in `kinds`. One `IN (...)` query per BATCH_CHUNK-sized
    /// slice of paths; `kinds` is splatted into the same parameter list.
    /// Callers pass language-specific kind lists (see
    /// [`Language::distinctive_symbol_kinds`]).
    pub fn files_with_symbol_kinds_batch(
        &self,
        paths: &[&str],
        kinds: &[&str],
    ) -> anyhow::Result<FxHashSet<String>> {
        let mut out: FxHashSet<String> = FxHashSet::default();
        if paths.is_empty() || kinds.is_empty() {
            return Ok(out);
        }
        let conn = self.open_read_only()?;
        for chunk in paths.chunks(BATCH_CHUNK) {
            let sql = format!(
                "SELECT DISTINCT file_path FROM symbols \
                 WHERE file_path IN ({}) AND kind IN ({})",
                placeholders(chunk.len()),
                placeholders(kinds.len())
            );
            let mut stmt = conn.prepare_cached(&sql)?;
            let params = chunk.iter().copied().chain(kinds.iter().copied());
            let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
                row.get::<_, String>(0)
            })?;
            for row in rows {
                out.insert(row?);
            }
        }
        Ok(out)
    }

    /// Batch-variant of [`IndexStore::symbol_record`]. Returns a
    /// `id → record` map. `root_path` is fetched once; missing ids
    /// are absent from the map.
    pub fn symbol_records_batch(
        &self,
        ids: &[&str],
    ) -> anyhow::Result<FxHashMap<String, SymbolRecord>> {
        let mut out: FxHashMap<String, SymbolRecord> = FxHashMap::default();
        if ids.is_empty() {
            return Ok(out);
        }
        let root_path: PathBuf = self.root_path()?;
        let conn = self.open_read_only()?;
        for chunk in ids.chunks(BATCH_CHUNK) {
            let sql = format!(
                "SELECT id, kind, extraction_tier, file_path, start_line, end_line, \
                        start_byte, end_byte, source_hash, source_text, file_size_bytes \
                 FROM symbols WHERE id IN ({})",
                placeholders(chunk.len())
            );
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                symbol_record_from_row(row, &root_path)
            })?;
            for row in rows {
                let rec = row?;
                out.insert(rec.id.clone(), rec);
            }
        }
        Ok(out)
    }

    /// Return the first file that defines a symbol with the given
    /// name. Indexed by `symbols.name` via the existing `idx_symbols_*`
    /// covering index. Replaces `blast_radius::resolve_symbol_file`'s
    /// legacy fallback that ran `file_outline` per file.
    pub fn file_for_symbol_name(&self, name: &str) -> anyhow::Result<Option<String>> {
        if name.is_empty() {
            return Ok(None);
        }
        let conn = self.open_read_only()?;
        let mut stmt =
            conn.prepare_cached("SELECT file_path FROM symbols WHERE name = ?1 LIMIT 1")?;
        let row: Option<String> = stmt.query_row([name], |row| row.get(0)).optional()?;
        Ok(row)
    }

    /// Combined lookup used by `assemble_context mode=context_bundle`:
    /// returns name, qualified_name, signature, file_path, and the
    /// owning file's language, in one SQL query per chunk.
    /// Replaces four per-ref DB calls (`symbol_names_by_id`,
    /// `symbol_signature_by_id`, `file_language`, `root_path`) with
    /// one LEFT JOIN plus a single `root_path()` call outside the loop.
    pub fn symbol_enriched_batch(
        &self,
        ids: &[&str],
    ) -> anyhow::Result<FxHashMap<String, SymbolEnriched>> {
        let mut out: FxHashMap<String, SymbolEnriched> = FxHashMap::default();
        if ids.is_empty() {
            return Ok(out);
        }
        let conn = self.open_read_only()?;
        for chunk in ids.chunks(BATCH_CHUNK) {
            let sql = format!(
                "SELECT s.id, s.name, s.qualified_name, s.signature, s.file_path, f.language \
                 FROM symbols s \
                 LEFT JOIN files f ON f.file_path = s.file_path \
                 WHERE s.id IN ({})",
                placeholders(chunk.len())
            );
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    SymbolEnriched {
                        name: row.get::<_, String>(1)?,
                        qualified_name: row.get::<_, String>(2)?,
                        signature: row.get::<_, String>(3)?,
                        file_path: row.get::<_, String>(4)?,
                        language: row.get::<_, Option<String>>(5)?,
                    },
                ))
            })?;
            for row in rows {
                let (id, enriched) = row?;
                out.insert(id, enriched);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn empty_store() -> (tempfile::TempDir, IndexStore) {
        let dir = tempdir().unwrap();
        let store = IndexStore::open(dir.path().join("x.sqlite")).unwrap();
        (dir, store)
    }

    #[test]
    fn file_outlines_batch_empty_returns_empty_map() {
        let (_d, s) = empty_store();
        assert!(s.file_outlines_batch(&[]).unwrap().is_empty());
    }

    #[test]
    fn file_languages_batch_empty_returns_empty_map() {
        let (_d, s) = empty_store();
        assert!(s.file_languages_batch(&[]).unwrap().is_empty());
    }

    #[test]
    fn file_bytes_batch_empty_returns_empty_map() {
        let (_d, s) = empty_store();
        assert!(s.file_bytes_batch(&[]).unwrap().is_empty());
    }

    #[test]
    fn files_exist_batch_empty_returns_empty_set() {
        let (_d, s) = empty_store();
        assert!(s.files_exist_batch(&[]).unwrap().is_empty());
    }

    #[test]
    fn files_exist_batch_returns_only_indexed_paths() {
        let (_d, s) = empty_store();
        // None indexed → all absent.
        let got = s.files_exist_batch(&["a.rs", "b.rs"]).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn symbol_records_batch_empty_returns_empty_map() {
        let (_d, s) = empty_store();
        assert!(s.symbol_records_batch(&[]).unwrap().is_empty());
    }

    #[test]
    fn symbol_enriched_batch_empty_returns_empty_map() {
        let (_d, s) = empty_store();
        assert!(s.symbol_enriched_batch(&[]).unwrap().is_empty());
    }

    #[test]
    fn placeholders_shape_is_correct() {
        assert_eq!(placeholders(0), "");
        assert_eq!(placeholders(1), "?");
        assert_eq!(placeholders(3), "?,?,?");
    }
}
