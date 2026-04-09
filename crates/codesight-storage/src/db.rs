//! Shared SQLite helpers for IndexStore and DocStore.

use std::{collections::HashSet, fs, path::Path};

use rusqlite::{Connection, OpenFlags};

/// Shared `repo_info` DDL for code and documentation SQLite stores.
pub const REPO_INFO_DDL: &str = r#"CREATE TABLE IF NOT EXISTS repo_info (
    repo TEXT PRIMARY KEY,
    root_path TEXT NOT NULL,
    indexed_at TEXT NOT NULL,
    index_version INTEGER NOT NULL,
    schema_version INTEGER NOT NULL
);

"#;

pub const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "__pycache__",
    ".venv",
    "venv",
    "dist",
    "build",
    ".tox",
    "target",
];

pub fn open_rw(path: &Path) -> anyhow::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    Ok(conn)
}

pub fn open_ro(path: &Path) -> anyhow::Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    Ok(conn)
}

pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric() && ch != '_')
        .flat_map(|token| token.split('_'))
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| !token.is_empty())
        .collect()
}

/// Fast stat-based staleness check. Returns true if any indexed file has
/// a different size or mtime on disk, or if new relevant files exist that
/// aren't in the index.
pub fn check_staleness(
    conn: &Connection,
    root_path: &Path,
    file_table: &str,
    is_relevant: impl Fn(&str) -> bool,
) -> anyhow::Result<bool> {
    let count_sql = format!("SELECT COUNT(*) FROM {file_table}");
    let file_count = conn.query_row(&count_sql, [], |r| r.get::<_, i64>(0))? as usize;
    if file_count > 10_000 {
        return Ok(false);
    }

    let select_sql = format!("SELECT file_path, size_bytes, modified_ns FROM {file_table}");
    let mut stmt = conn.prepare(&select_sql)?;
    let mut indexed_paths = HashSet::new();
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)? as u64,
            row.get::<_, i64>(2)? as u64,
        ))
    })?;

    for row in rows {
        let (file_path, db_size, db_mtime) = row?;
        indexed_paths.insert(file_path.clone());
        let full = root_path.join(&file_path);
        match fs::metadata(&full) {
            Ok(meta) => {
                let disk_size = meta.len();
                let disk_mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0);
                if disk_size != db_size || disk_mtime != db_mtime {
                    return Ok(true);
                }
            }
            Err(_) => return Ok(true),
        }
    }

    // Check for new files not in the index
    let skip_set: HashSet<&str> = SKIP_DIRS.iter().copied().collect();
    for entry in walkdir::WalkDir::new(root_path)
        .into_iter()
        .filter_entry(|e| {
            !e.file_type().is_dir() || !skip_set.contains(e.file_name().to_string_lossy().as_ref())
        })
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let rel = entry
            .path()
            .strip_prefix(root_path)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if is_relevant(&rel) && !indexed_paths.contains(&rel) {
            return Ok(true);
        }
    }

    Ok(false)
}
