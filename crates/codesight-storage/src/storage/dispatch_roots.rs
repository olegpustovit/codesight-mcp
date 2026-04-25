//! Per-source cache of string-dispatch root signals (P3 perf fix).
//!
//! `evaluate_quality dead_code` used to scan every file body on every call to
//! detect symbol names that appear as string literals in 2+ other files
//! (handler-table dispatch). For a 5k-symbol repo the scan cost ~8 s per
//! call. The detector's input is `(symbol set, file contents)`, both
//! fingerprinted at ingest, so it's safe to compute once at the end of
//! `index_workspace_with_symbol_vectors` and serve subsequent queries from
//! this table.
//!
//! Schema (created in `migrations::migrate_v10_dispatch_roots`):
//! ```sql
//! dispatch_roots(
//!     file_path TEXT NOT NULL,
//!     symbol_name TEXT NOT NULL,
//!     strength TEXT NOT NULL CHECK (strength IN ('strong', 'weak')),
//!     PRIMARY KEY (file_path, symbol_name)
//! )
//! ```
//!
//! Strong: name appears as a string literal in ≥2 *other* files. Likely a
//! router / handler-table entry, treated as a live root.
//! Weak: name appears in exactly one other file. Tie-break signal — promoted
//! only under `paranoid=true`.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::StorageResult;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DispatchStrength {
    Strong,
    Weak,
}

impl DispatchStrength {
    pub fn as_str(&self) -> &'static str {
        match self {
            DispatchStrength::Strong => "strong",
            DispatchStrength::Weak => "weak",
        }
    }
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "strong" => Some(Self::Strong),
            "weak" => Some(Self::Weak),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DispatchRoot {
    pub file_path: String,
    pub symbol_name: String,
    pub strength: DispatchStrength,
}

/// Replace the entire `dispatch_roots` table contents in a single
/// transaction. Called at the end of every successful ingest by
/// `compute_dispatch_roots` in `codesight-engine::indexing::code_index`.
/// `roots` is expected to be the full per-source set; partial updates are
/// not supported (the detector is whole-repo by construction).
pub fn replace_all(conn: &mut Connection, roots: &[DispatchRoot]) -> StorageResult<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM dispatch_roots", [])?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO dispatch_roots (file_path, symbol_name, strength) \
             VALUES (?1, ?2, ?3)",
        )?;
        for root in roots {
            stmt.execute(rusqlite::params![
                root.file_path,
                root.symbol_name,
                root.strength.as_str(),
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Read the full set of dispatch roots, with strength stratification.
/// Returns `(strong_roots, weak_roots)` keyed by symbol name. Empty when
/// the table has not been populated yet (first run after migration v10);
/// callers should fall back to in-memory detection in that case.
pub fn load_all(conn: &Connection) -> StorageResult<(Vec<DispatchRoot>, Vec<DispatchRoot>)> {
    let mut stmt =
        conn.prepare("SELECT file_path, symbol_name, strength FROM dispatch_roots")?;
    let rows = stmt.query_map([], |row| {
        let file_path: String = row.get(0)?;
        let symbol_name: String = row.get(1)?;
        let strength_s: String = row.get(2)?;
        Ok((file_path, symbol_name, strength_s))
    })?;

    let mut strong = Vec::new();
    let mut weak = Vec::new();
    for row in rows {
        let (file_path, symbol_name, strength_s) = row?;
        let Some(strength) = DispatchStrength::from_str(&strength_s) else {
            continue;
        };
        let entry = DispatchRoot {
            file_path,
            symbol_name,
            strength,
        };
        match strength {
            DispatchStrength::Strong => strong.push(entry),
            DispatchStrength::Weak => weak.push(entry),
        }
    }
    Ok((strong, weak))
}

/// True when the `dispatch_roots` table has at least one row. Used by the
/// dead_code path to decide between the cached-fast path and the on-the-fly
/// detector fallback.
pub fn is_populated(conn: &Connection) -> StorageResult<bool> {
    let count: i64 =
        conn.query_row("SELECT COUNT(*) FROM dispatch_roots LIMIT 1", [], |row| {
            row.get(0)
        })?;
    Ok(count > 0)
}

/// Collapse the granular `(file, name, strength)` rows into the per-file
/// strong/weak sets that the dead_code reachability path consumes. A file
/// is `strong` if *any* of its symbols qualifies; otherwise `weak` if any
/// symbol matched at the weaker threshold. Used by `IndexStore::dispatch_root_file_sets`.
pub fn load_file_sets(
    conn: &Connection,
) -> StorageResult<(rustc_hash::FxHashSet<String>, rustc_hash::FxHashSet<String>)> {
    let (strong, weak) = load_all(conn)?;
    let strong_files: rustc_hash::FxHashSet<String> =
        strong.into_iter().map(|r| r.file_path).collect();
    let mut weak_files = rustc_hash::FxHashSet::default();
    for r in weak {
        if !strong_files.contains(&r.file_path) {
            weak_files.insert(r.file_path);
        }
    }
    Ok((strong_files, weak_files))
}
