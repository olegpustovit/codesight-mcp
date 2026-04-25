//! Forward-only SQLite migrations for the code index (`user_version`).
//!
//! `repo_info.schema_version` (see [`crate::config::SCHEMA_VERSION`]) is separate: it records
//! the logical index format the app wrote last. `PRAGMA user_version` is the SQLite migration
//! chain for DDL evolution only.

use rusqlite::Connection;
use tracing::instrument;

use crate::error::StorageResult;

use super::schema::{EMBEDDING_DIM, INDEX_STORE_INIT_SQL};

/// Bump when adding a migration; keep in sync with [`apply_code_index_migrations`].
pub const CODE_INDEX_TARGET_USER_VERSION: i32 = 10;

type MigrationStep = fn(&Connection) -> StorageResult<()>;

fn migrate_v1_initial(conn: &Connection) -> StorageResult<()> {
    conn.execute_batch(INDEX_STORE_INIT_SQL.as_str())?;
    Ok(())
}

fn migrate_v2_import_fact_quality(conn: &Connection) -> StorageResult<()> {
    add_import_fact_quality_if_missing(conn)
}

fn migrate_v3_extractor_version(conn: &Connection) -> StorageResult<()> {
    add_extractor_version_column_if_missing(conn)
}

fn migrate_v4_quality_snapshots(conn: &Connection) -> StorageResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS quality_snapshots (
            source_id TEXT NOT NULL,
            name TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            score INTEGER NOT NULL,
            structural_score INTEGER NOT NULL,
            agent_readiness_score INTEGER NOT NULL,
            modularity REAL,
            acyclicity REAL,
            depth REAL,
            equality REAL,
            redundancy REAL,
            navigability REAL,
            comprehensibility REAL,
            verifiability REAL,
            change_safety REAL,
            cycle_count INTEGER NOT NULL,
            dead_count INTEGER NOT NULL,
            dup_count INTEGER NOT NULL,
            file_count INTEGER NOT NULL,
            symbol_count INTEGER NOT NULL,
            confidence TEXT NOT NULL,
            avg_complexity REAL NOT NULL,
            PRIMARY KEY (source_id, name)
        );",
    )?;
    Ok(())
}

fn migrate_v6_vec_symbol_embeddings(conn: &Connection) -> StorageResult<()> {
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS vec_symbol_embeddings USING vec0(symbol_rowid INTEGER PRIMARY KEY, embedding FLOAT[{EMBEDDING_DIM}]);"
    ))?;
    Ok(())
}

fn migrate_v7_symbol_indexes(conn: &Connection) -> StorageResult<()> {
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_symbols_file_line
             ON symbols(file_path, start_line, end_line);
         CREATE INDEX IF NOT EXISTS idx_symbols_file_path
             ON symbols(file_path);",
    )?;
    Ok(())
}

/// Uniform `[0.0, 1.0]` float encoding for quality scores. The v5 table
/// declared `score`, `structural_score`, `agent_readiness_score` as INTEGER
/// holding thousandths (e.g. `7402` = 0.7402). We now keep those in `[0,1]`
/// floats everywhere; rebuild the table with REAL columns and rescale any
/// surviving rows so regression comparisons still work.
fn migrate_v8_quality_scores_as_float(conn: &Connection) -> StorageResult<()> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='quality_snapshots'",
        [],
        |row| row.get(0),
    )?;
    if exists == 0 {
        return Ok(());
    }
    conn.execute_batch(
        "ALTER TABLE quality_snapshots RENAME TO quality_snapshots_v5;
         CREATE TABLE quality_snapshots (
             source_id TEXT NOT NULL,
             name TEXT NOT NULL,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             score REAL NOT NULL,
             structural_score REAL NOT NULL,
             agent_readiness_score REAL NOT NULL,
             modularity REAL,
             acyclicity REAL,
             depth REAL,
             equality REAL,
             redundancy REAL,
             navigability REAL,
             comprehensibility REAL,
             verifiability REAL,
             change_safety REAL,
             cycle_count INTEGER NOT NULL,
             dead_count INTEGER NOT NULL,
             dup_count INTEGER NOT NULL,
             file_count INTEGER NOT NULL,
             symbol_count INTEGER NOT NULL,
             confidence TEXT NOT NULL,
             avg_complexity REAL NOT NULL,
             PRIMARY KEY (source_id, name)
         );
         INSERT INTO quality_snapshots (
             source_id, name, created_at, score, structural_score, agent_readiness_score,
             modularity, acyclicity, depth, equality, redundancy,
             navigability, comprehensibility, verifiability, change_safety,
             cycle_count, dead_count, dup_count, file_count, symbol_count,
             confidence, avg_complexity
         )
         SELECT source_id, name, created_at,
                score / 10000.0, structural_score / 10000.0, agent_readiness_score / 10000.0,
                modularity, acyclicity, depth, equality, redundancy,
                navigability, comprehensibility, verifiability, change_safety,
                cycle_count, dead_count, dup_count, file_count, symbol_count,
                confidence, avg_complexity
         FROM quality_snapshots_v5;
         DROP TABLE quality_snapshots_v5;",
    )?;
    Ok(())
}

fn migrate_v9_reachability_edges(conn: &Connection) -> StorageResult<()> {
    // Nullable target_file / target_symbol are intentional: an edge may be
    // file-only (`mod child;` resolved to a path) or symbol-only (a re-export
    // of a symbol whose defining file we couldn't pin down at extract time).
    // SQLite treats NULL as distinct in UNIQUE indexes, so best-effort dedup
    // is enforced via INSERT OR IGNORE; bounded duplicates are tolerable.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS reachability_edges (
             source_file   TEXT NOT NULL,
             target_file   TEXT,
             target_symbol TEXT,
             edge_kind     TEXT NOT NULL
         );
         CREATE UNIQUE INDEX IF NOT EXISTS uq_reachability_edges
             ON reachability_edges(source_file, target_file, target_symbol, edge_kind);
         CREATE INDEX IF NOT EXISTS idx_reachability_edges_source
             ON reachability_edges(source_file);
         CREATE INDEX IF NOT EXISTS idx_reachability_edges_target_file
             ON reachability_edges(target_file);
         CREATE INDEX IF NOT EXISTS idx_reachability_edges_target_symbol
             ON reachability_edges(target_symbol);
         CREATE INDEX IF NOT EXISTS idx_reachability_edges_kind
             ON reachability_edges(edge_kind);",
    )?;
    // The new `reachability_edges` table is populated only at extract time, so
    // pre-existing repos with their fingerprint cache intact would skip every
    // unchanged file and leave the table empty until the file got edited.
    // Clear the stored extractor version so the next ingest's stamp-drift
    // guard (codesight-engine `index_workspace_with_symbol_vectors`) treats
    // every existing file as needing re-extraction and fully populates the
    // new table. Tolerate the legacy case where `repo_info` doesn't yet
    // exist (very fresh DBs) — there's nothing to reset.
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='repo_info'",
        [],
        |row| row.get(0),
    )?;
    if exists > 0 {
        conn.execute("UPDATE repo_info SET extractor_version = ''", [])?;
    }
    Ok(())
}

fn migrate_v10_dispatch_roots(conn: &Connection) -> StorageResult<()> {
    // String-dispatch root cache (P3 perf fix). Symbols whose name appears
    // as a string literal in 2+ other files are flagged `strong` (likely
    // router / handler-table dispatch); single-file mentions stay `weak`.
    // Without this table, `evaluate_quality dead_code` recomputed the
    // detector by scanning every file body on every call (~8 s avg on a
    // 5k-symbol repo). Populated at the end of each successful ingest by
    // `compute_dispatch_roots` in `codesight-engine`.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS dispatch_roots (
             file_path TEXT NOT NULL,
             symbol_name TEXT NOT NULL,
             strength TEXT NOT NULL CHECK (strength IN ('strong', 'weak')),
             PRIMARY KEY (file_path, symbol_name)
         );
         CREATE INDEX IF NOT EXISTS idx_dispatch_roots_file
             ON dispatch_roots(file_path);
         CREATE INDEX IF NOT EXISTS idx_dispatch_roots_strength
             ON dispatch_roots(strength);",
    )?;
    // Empty table on first migrate; the next ingest populates it. Until
    // then, the dead_code path falls back to in-memory detection so the
    // gap is invisible to callers.
    Ok(())
}

const CODE_INDEX_MIGRATIONS: &[(i32, MigrationStep)] = &[
    (1, migrate_v1_initial),
    (2, migrate_v2_import_fact_quality),
    (3, migrate_v3_extractor_version),
    (4, migrate_v4_quality_snapshots),
    (5, rebuild_quality_snapshots_v5),
    (6, migrate_v6_vec_symbol_embeddings),
    (7, migrate_v7_symbol_indexes),
    (8, migrate_v8_quality_scores_as_float),
    (9, migrate_v9_reachability_edges),
    (10, migrate_v10_dispatch_roots),
];

/// Verdict from probing an existing index file. Callers (the engine's index
/// open path) act on `Incompatible` by deleting the file and rebuilding —
/// agents see one `INDEX_REBUILT` warning instead of a confusing parse
/// error two layers down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexCompatibility {
    /// Forward migration possible (`db_version <= target`).
    Compatible,
    /// `db_version > target` — index was written by a newer binary. The
    /// running binary cannot reason about the future-version schema.
    Incompatible,
}

/// Inspect `PRAGMA user_version` against the target. Used by the engine to
/// decide whether to migrate forward or scrap the index file.
pub fn check_index_compatibility(conn: &Connection) -> StorageResult<IndexCompatibility> {
    let current: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current > CODE_INDEX_TARGET_USER_VERSION {
        Ok(IndexCompatibility::Incompatible)
    } else {
        Ok(IndexCompatibility::Compatible)
    }
}

/// Apply migrations so `PRAGMA user_version == CODE_INDEX_TARGET_USER_VERSION`.
/// Returns an error if the index reports a *higher* user_version than the
/// running binary supports — caller is expected to delete + rebuild rather
/// than try to downgrade in place.
#[instrument(level = "debug", skip(conn), fields(kind = "migrate_code_index"))]
pub fn apply_code_index_migrations(conn: &Connection) -> StorageResult<()> {
    let mut current: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current > CODE_INDEX_TARGET_USER_VERSION {
        return Err(crate::error::StorageError::Other(format!(
            "index user_version={current} > target={CODE_INDEX_TARGET_USER_VERSION}; \
             this index was written by a newer binary. Delete the index file or \
             upgrade the binary to match."
        )));
    }
    for &(target, step) in CODE_INDEX_MIGRATIONS {
        if current < target {
            step(conn)?;
            conn.execute(&format!("PRAGMA user_version = {target}"), [])?;
            current = target;
        }
    }
    Ok(())
}

fn add_extractor_version_column_if_missing(conn: &Connection) -> StorageResult<()> {
    let mut info = conn.prepare("PRAGMA table_info(repo_info)")?;
    let names: Vec<String> = info
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<_, _>>()?;
    if names.iter().any(|col| col == "extractor_version") {
        return Ok(());
    }
    conn.execute(
        "ALTER TABLE repo_info ADD COLUMN extractor_version TEXT NOT NULL DEFAULT ''",
        [],
    )?;
    Ok(())
}

fn add_import_fact_quality_if_missing(conn: &Connection) -> StorageResult<()> {
    let mut stmt = conn.prepare(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='dependency_edges'",
    )?;
    let count: i64 = stmt.query_row([], |row| row.get(0))?;
    if count == 0 {
        return Ok(());
    }
    let mut info = conn.prepare("PRAGMA table_info(dependency_edges)")?;
    let names: Vec<String> = info
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<_, _>>()?;
    if names.iter().any(|col| col == "import_fact_quality") {
        return Ok(());
    }
    conn.execute(
        "ALTER TABLE dependency_edges ADD COLUMN import_fact_quality TEXT NOT NULL DEFAULT 'baseline_structural'",
        [],
    )?;
    Ok(())
}

fn rebuild_quality_snapshots_v5(conn: &Connection) -> StorageResult<()> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='quality_snapshots'",
        [],
        |row| row.get(0),
    )?;
    if exists == 0 {
        conn.execute_batch(
            "CREATE TABLE quality_snapshots (
                source_id TEXT NOT NULL,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                score INTEGER NOT NULL,
                structural_score INTEGER NOT NULL,
                agent_readiness_score INTEGER NOT NULL,
                modularity REAL,
                acyclicity REAL,
                depth REAL,
                equality REAL,
                redundancy REAL,
                navigability REAL,
                comprehensibility REAL,
                verifiability REAL,
                change_safety REAL,
                cycle_count INTEGER NOT NULL,
                dead_count INTEGER NOT NULL,
                dup_count INTEGER NOT NULL,
                file_count INTEGER NOT NULL,
                symbol_count INTEGER NOT NULL,
                confidence TEXT NOT NULL,
                avg_complexity REAL NOT NULL,
                PRIMARY KEY (source_id, name)
            );",
        )?;
        return Ok(());
    }

    conn.execute_batch(
        "ALTER TABLE quality_snapshots RENAME TO quality_snapshots_v4;
         CREATE TABLE quality_snapshots (
             source_id TEXT NOT NULL,
             name TEXT NOT NULL,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             score INTEGER NOT NULL,
             structural_score INTEGER NOT NULL,
             agent_readiness_score INTEGER NOT NULL,
             modularity REAL,
             acyclicity REAL,
             depth REAL,
             equality REAL,
             redundancy REAL,
             navigability REAL,
             comprehensibility REAL,
             verifiability REAL,
             change_safety REAL,
             cycle_count INTEGER NOT NULL,
             dead_count INTEGER NOT NULL,
             dup_count INTEGER NOT NULL,
             file_count INTEGER NOT NULL,
             symbol_count INTEGER NOT NULL,
             confidence TEXT NOT NULL,
             avg_complexity REAL NOT NULL,
             PRIMARY KEY (source_id, name)
         );
         INSERT INTO quality_snapshots (
             source_id, name, created_at, score, structural_score, agent_readiness_score,
             modularity, acyclicity, depth, equality, redundancy,
             navigability, comprehensibility, verifiability, change_safety,
             cycle_count, dead_count, dup_count, file_count, symbol_count,
             confidence, avg_complexity
         )
         SELECT source_id, name, created_at, score, structural_score, agent_readiness_score,
                modularity, acyclicity, depth, equality, redundancy,
                NULL, NULL, NULL, NULL,
                cycle_count, dead_count, dup_count, file_count, symbol_count,
                confidence, avg_complexity
         FROM quality_snapshots_v4;
         DROP TABLE quality_snapshots_v4;",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::CODE_INDEX_TARGET_USER_VERSION;
    use crate::storage::IndexStore;

    #[test]
    fn repo_info_has_extractor_version_column_after_migrations() {
        let base = std::env::temp_dir().join(format!(
            "codesight_storage_extractor_version_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&base).unwrap();
        let path: PathBuf = base.join("index.sqlite3");
        IndexStore::open(path.clone()).unwrap();

        let conn = rusqlite::Connection::open(&path).unwrap();
        let mut stmt = conn.prepare("PRAGMA table_info(repo_info)").unwrap();
        let names: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(
            names.iter().any(|c| c == "extractor_version"),
            "repo_info must carry extractor_version after migrations; got {names:?}"
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn code_index_uses_wal_and_migration_version() {
        let base = std::env::temp_dir().join(format!(
            "codesight_storage_wal_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&base).unwrap();
        let path: PathBuf = base.join("index.sqlite3");
        IndexStore::open(path.clone()).unwrap();

        let conn = rusqlite::Connection::open(&path).unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");

        let uv: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(uv, CODE_INDEX_TARGET_USER_VERSION);

        let _ = fs::remove_dir_all(&base);
    }
}
