//! SQLite DDL + forward-only migrations for the doc index.
//!
//! Lives in `symslice-storage` alongside the code-index schema so all SQLite
//! DDL evolution flows through one crate. The product crate composes this
//! with its own `Section` parser to build doc indexes.

use std::sync::LazyLock;

use rusqlite::Connection;
use tracing::instrument;

use crate::db;
use crate::error::StorageResult;
use crate::storage::schema::EMBEDDING_DIM;

const DOC_STORE_INIT_SQL_TAIL_PREFIX: &str = r#"
CREATE TABLE IF NOT EXISTS doc_files (
    file_path TEXT PRIMARY KEY,
    format TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    modified_ns INTEGER NOT NULL,
    content_hash TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sections (
    row_id INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    file_path TEXT NOT NULL,
    heading TEXT NOT NULL,
    heading_path TEXT NOT NULL,
    level INTEGER NOT NULL,
    summary TEXT NOT NULL,
    content_text TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    file_size_bytes INTEGER NOT NULL,
    FOREIGN KEY(file_path) REFERENCES doc_files(file_path) ON DELETE CASCADE
);

CREATE VIRTUAL TABLE IF NOT EXISTS section_fts USING fts5(
    heading,
    heading_path,
    summary,
    content_text,
    file_path,
    content='sections',
    content_rowid='row_id',
    tokenize='porter unicode61'
);

CREATE TRIGGER IF NOT EXISTS sections_ai AFTER INSERT ON sections BEGIN
    INSERT INTO section_fts(rowid, heading, heading_path, summary, content_text, file_path)
    VALUES (new.row_id, new.heading, new.heading_path, new.summary, new.content_text, new.file_path);
END;

CREATE TRIGGER IF NOT EXISTS sections_ad AFTER DELETE ON sections BEGIN
    INSERT INTO section_fts(section_fts, rowid, heading, heading_path, summary, content_text, file_path)
    VALUES ('delete', old.row_id, old.heading, old.heading_path, old.summary, old.content_text, old.file_path);
END;

CREATE TRIGGER IF NOT EXISTS sections_au AFTER UPDATE ON sections BEGIN
    INSERT INTO section_fts(section_fts, rowid, heading, heading_path, summary, content_text, file_path)
    VALUES ('delete', old.row_id, old.heading, old.heading_path, old.summary, old.content_text, old.file_path);
    INSERT INTO section_fts(rowid, heading, heading_path, summary, content_text, file_path)
    VALUES (new.row_id, new.heading, new.heading_path, new.summary, new.content_text, new.file_path);
END;
"#;

pub static DOC_STORE_INIT_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "{}{}\nCREATE VIRTUAL TABLE IF NOT EXISTS vec_section_embeddings USING vec0(section_rowid INTEGER PRIMARY KEY, embedding FLOAT[{dim}]);\n",
        db::REPO_INFO_DDL,
        DOC_STORE_INIT_SQL_TAIL_PREFIX,
        dim = EMBEDDING_DIM,
    )
});

/// Bump when adding a doc-index migration.
pub const DOC_INDEX_TARGET_USER_VERSION: i32 = 2;

/// Apply forward-only migrations so `PRAGMA user_version == DOC_INDEX_TARGET_USER_VERSION`.
#[instrument(level = "debug", skip(conn), fields(kind = "migrate_doc_index"))]
pub fn apply_doc_index_migrations(conn: &Connection) -> StorageResult<()> {
    let v: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if v < 1 {
        conn.execute_batch(DOC_STORE_INIT_SQL.as_str())?;
        conn.execute("PRAGMA user_version = 1", [])?;
    }
    let v: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if v < 2 {
        conn.execute_batch(&format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS vec_section_embeddings USING vec0(section_rowid INTEGER PRIMARY KEY, embedding FLOAT[{EMBEDDING_DIM}]);"
        ))?;
        conn.execute("PRAGMA user_version = 2", [])?;
    }
    Ok(())
}
