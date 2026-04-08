//! SQLite DDL for the on-disk code index (files, symbols, FTS, dependency edges).

use std::sync::LazyLock;

use crate::db;

const INDEX_STORE_INIT_SQL_TAIL: &str = r#"
CREATE TABLE IF NOT EXISTS files (
    file_path TEXT PRIMARY KEY,
    language TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    modified_ns INTEGER NOT NULL,
    content_hash TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS symbols (
    row_id INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    file_path TEXT NOT NULL,
    language TEXT NOT NULL,
    extraction_tier TEXT NOT NULL,
    kind TEXT NOT NULL,
    qualified_name TEXT NOT NULL,
    name TEXT NOT NULL,
    signature TEXT NOT NULL,
    summary TEXT NOT NULL,
    source_text TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    source_hash TEXT NOT NULL,
    file_size_bytes INTEGER NOT NULL,
    FOREIGN KEY(file_path) REFERENCES files(file_path) ON DELETE CASCADE
);

CREATE VIRTUAL TABLE IF NOT EXISTS symbol_fts USING fts5(
    name,
    qualified_name,
    signature,
    summary,
    file_path,
    source_text,
    content='symbols',
    content_rowid='row_id',
    tokenize='porter unicode61'
);

CREATE TRIGGER IF NOT EXISTS symbols_ai AFTER INSERT ON symbols BEGIN
    INSERT INTO symbol_fts(rowid, name, qualified_name, signature, summary, file_path, source_text)
    VALUES (new.row_id, new.name, new.qualified_name, new.signature, new.summary, new.file_path, new.source_text);
END;

CREATE TRIGGER IF NOT EXISTS symbols_ad AFTER DELETE ON symbols BEGIN
    INSERT INTO symbol_fts(symbol_fts, rowid, name, qualified_name, signature, summary, file_path, source_text)
    VALUES ('delete', old.row_id, old.name, old.qualified_name, old.signature, old.summary, old.file_path, old.source_text);
END;

CREATE TRIGGER IF NOT EXISTS symbols_au AFTER UPDATE ON symbols BEGIN
    INSERT INTO symbol_fts(symbol_fts, rowid, name, qualified_name, signature, summary, file_path, source_text)
    VALUES ('delete', old.row_id, old.name, old.qualified_name, old.signature, old.summary, old.file_path, old.source_text);
    INSERT INTO symbol_fts(rowid, name, qualified_name, signature, summary, file_path, source_text)
    VALUES (new.row_id, new.name, new.qualified_name, new.signature, new.summary, new.file_path, new.source_text);
END;

CREATE TABLE IF NOT EXISTS dependency_edges (
    source_file TEXT NOT NULL,
    target_file TEXT NOT NULL,
    PRIMARY KEY (source_file, target_file),
    FOREIGN KEY(source_file) REFERENCES files(file_path) ON DELETE CASCADE,
    FOREIGN KEY(target_file) REFERENCES files(file_path) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

pub(crate) static INDEX_STORE_INIT_SQL: LazyLock<String> =
    LazyLock::new(|| format!("{}{}", db::REPO_INFO_DDL, INDEX_STORE_INIT_SQL_TAIL));
