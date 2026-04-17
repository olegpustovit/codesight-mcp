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
pub const CODE_INDEX_TARGET_USER_VERSION: i32 = 7;

/// Apply migrations so `PRAGMA user_version == CODE_INDEX_TARGET_USER_VERSION`.
#[instrument(level = "debug", skip(conn), fields(kind = "migrate_code_index"))]
pub fn apply_code_index_migrations(conn: &Connection) -> StorageResult<()> {
    let v: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if v < 1 {
        conn.execute_batch(INDEX_STORE_INIT_SQL.as_str())?;
        conn.execute("PRAGMA user_version = 1", [])?;
    }
    let v: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if v < 2 {
        add_import_fact_quality_if_missing(conn)?;
        conn.execute("PRAGMA user_version = 2", [])?;
    }
    let v: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if v < 3 {
        add_extractor_version_column_if_missing(conn)?;
        conn.execute("PRAGMA user_version = 3", [])?;
    }
    let v: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if v < 4 {
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
        conn.execute("PRAGMA user_version = 4", [])?;
    }
    let v: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if v < 5 {
        rebuild_quality_snapshots_v5(conn)?;
        conn.execute("PRAGMA user_version = 5", [])?;
    }
    let v: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if v < 6 {
        conn.execute_batch(&format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS vec_symbol_embeddings USING vec0(symbol_rowid INTEGER PRIMARY KEY, embedding FLOAT[{EMBEDDING_DIM}]);"
        ))?;
        conn.execute("PRAGMA user_version = 6", [])?;
    }
    let v: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if v < 7 {
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_symbols_file_line
                 ON symbols(file_path, start_line, end_line);
             CREATE INDEX IF NOT EXISTS idx_symbols_file_path
                 ON symbols(file_path);",
        )?;
        conn.execute("PRAGMA user_version = 7", [])?;
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
