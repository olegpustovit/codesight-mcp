//! Re-exports the doc-index DDL + migrations from `codesight-storage`.
//!
//! All SQLite DDL evolution flows through `codesight-storage::storage::doc_schema`.
//! This thin facade keeps existing `crate::storage::doc_index` consumers stable
//! while the storage primitives live in their owning crate.

pub use codesight_storage::storage::doc_schema::{
    DOC_INDEX_TARGET_USER_VERSION, apply_doc_index_migrations,
};

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::DOC_INDEX_TARGET_USER_VERSION;
    use crate::storage::doc_index::DocStore;

    #[test]
    fn doc_index_uses_wal_and_migration_version() {
        let base = std::env::temp_dir().join(format!(
            "codesight_doc_wal_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&base).unwrap();
        let path: PathBuf = base.join("doc_index.sqlite3");
        DocStore::open(path.clone()).unwrap();

        let conn = rusqlite::Connection::open(&path).unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");

        let uv: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(uv, DOC_INDEX_TARGET_USER_VERSION);

        // Idempotent re-open should not advance the version or fail.
        DocStore::open(path.clone()).unwrap();
        let uv2: i32 = rusqlite::Connection::open(&path)
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(uv2, DOC_INDEX_TARGET_USER_VERSION);

        let _ = fs::remove_dir_all(&base);
    }
}
