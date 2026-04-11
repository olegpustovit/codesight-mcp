//! Forward-only SQLite migrations for the code index (`user_version`).
//!
//! `repo_info.schema_version` (see [`crate::config::SCHEMA_VERSION`]) is separate: it records
//! the logical index format the app wrote last. `PRAGMA user_version` is the SQLite migration
//! chain for DDL evolution only.

use rusqlite::Connection;
use tracing::instrument;

use crate::error::StorageResult;

use super::schema::INDEX_STORE_INIT_SQL;

/// Bump when adding a migration; keep in sync with [`apply_code_index_migrations`].
pub const CODE_INDEX_TARGET_USER_VERSION: i32 = 3;

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
