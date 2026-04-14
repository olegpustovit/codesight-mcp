//! Shared SQLite helpers for IndexStore and DocStore.

use std::{
    collections::HashSet,
    fs,
    ops::Deref,
    path::Path,
    sync::{Mutex, MutexGuard, Once},
    time::Duration,
};

use rusqlite::{Connection, OpenFlags, ffi};

use crate::error::StorageResult;

/// Register `sqlite-vec` as an auto-extension so every subsequent
/// `Connection::open*` gets `vec0` virtual-table support without manual
/// loading. Idempotent — the `Once` guard makes double-registration cheap
/// across parallel test binaries.
fn ensure_sqlite_vec_registered() {
    type AutoExt = unsafe extern "C" fn(
        *mut ffi::sqlite3,
        *mut *mut std::os::raw::c_char,
        *const ffi::sqlite3_api_routines,
    ) -> std::os::raw::c_int;
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| unsafe {
        let entry: AutoExt = std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ());
        ffi::sqlite3_auto_extension(Some(entry));
    });
}

/// Per-store read-only connection cache. First call lazily opens via
/// `db::open_ro`; subsequent calls within the same store instance reuse it.
/// Reentrant or contended callers fall through to a fresh connection via
/// `try_lock`, so the mutex never deadlocks on nested reads. Lifetime is the
/// store instance — RO WAL readers pick up writers' commits on every
/// implicit `SELECT` transaction, so no explicit invalidation is needed.
pub type RoConnCache = Mutex<Option<Connection>>;

pub enum RoConn<'a> {
    Cached(MutexGuard<'a, Option<Connection>>),
    Fresh(Connection),
}

impl Deref for RoConn<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        match self {
            RoConn::Cached(g) => g.as_ref().expect("RoConn::Cached primed by open_cached_ro"),
            RoConn::Fresh(c) => c,
        }
    }
}

pub fn open_cached_ro<'a>(cache: &'a RoConnCache, path: &Path) -> StorageResult<RoConn<'a>> {
    match cache.try_lock() {
        Ok(mut guard) => {
            if guard.is_none() {
                *guard = Some(open_ro(path)?);
            }
            Ok(RoConn::Cached(guard))
        }
        Err(_) => Ok(RoConn::Fresh(open_ro(path)?)),
    }
}

/// Shared `repo_info` DDL for code and documentation SQLite stores.
pub const REPO_INFO_DDL: &str = r#"CREATE TABLE IF NOT EXISTS repo_info (
    repo TEXT PRIMARY KEY,
    root_path TEXT NOT NULL,
    indexed_at TEXT NOT NULL,
    index_version INTEGER NOT NULL,
    schema_version INTEGER NOT NULL,
    extractor_version TEXT NOT NULL DEFAULT ''
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
    // codesight's own state: the index dir (when nested under a repo being
    // indexed) and generated caches. Indexing these makes the server
    // see its own savings ledger as "dead code", which is absurd.
    ".codesight-index",
    ".codesight",
];

fn apply_rw_pragmas(conn: &Connection) -> StorageResult<()> {
    // Writable connection: production-oriented SQLite tuning (REFACTOR_PLAN P2).
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "mmap_size", 268_435_456i64)?;
    conn.pragma_update(None, "cache_size", -65_536i64)?;
    Ok(())
}

/// Read-only open: cannot switch `journal_mode` (requires write). Assumes writers already
/// enabled WAL; still set busy timeout so concurrent readers wait on checkpoints.
pub fn open_ro(path: &Path) -> StorageResult<Connection> {
    ensure_sqlite_vec_registered();
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.set_prepared_statement_cache_capacity(32);
    Ok(conn)
}

pub fn open_rw(path: &Path) -> StorageResult<Connection> {
    ensure_sqlite_vec_registered();
    let conn = Connection::open(path)?;
    apply_rw_pragmas(&conn)?;
    conn.set_prepared_statement_cache_capacity(32);
    Ok(conn)
}

/// Best-effort passive WAL checkpoint against an existing database file.
///
/// Intended for cleanup hooks (shutdown, abnormal reindex drop) where we want
/// to avoid leaving a fat WAL behind but must not block writers. Silently
/// returns `Ok(())` if the file is missing — there is nothing to checkpoint.
pub fn wal_checkpoint_passive(path: &Path) -> StorageResult<()> {
    if !path.exists() {
        return Ok(());
    }
    let conn = open_rw(path)?;
    conn.pragma_update(None, "wal_checkpoint", "PASSIVE")?;
    Ok(())
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
) -> StorageResult<bool> {
    let count_sql = format!("SELECT COUNT(*) FROM {file_table}");
    let file_count = conn.query_row(&count_sql, [], |r| r.get::<_, i64>(0))? as usize;
    if file_count > 10_000 {
        return Ok(false);
    }

    let select_sql = format!("SELECT file_path, size_bytes, modified_ns FROM {file_table}");
    let mut stmt = conn.prepare_cached(&select_sql)?;
    let mut indexed_paths = HashSet::new();
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)? as u64,
            row.get::<_, i64>(2)? as u64,
        ))
    })?;

    let skip_set: HashSet<&str> = SKIP_DIRS.iter().copied().collect();
    let path_in_skipped_dir = |rel: &str| -> bool {
        rel.split('/')
            .next()
            .is_some_and(|seg| skip_set.contains(seg))
    };

    for row in rows {
        let (file_path, db_size, db_mtime) = row?;
        // Skip stale entries from older ingests that ran before a dir was
        // added to SKIP_DIRS — e.g. `.codesight-index/_savings.json`, whose
        // mtime is rewritten by codesight on every call and would otherwise
        // mark the index permanently stale.
        if path_in_skipped_dir(&file_path) {
            continue;
        }
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

    // Check for new files not in the index. Mirror the ingest walker
    // (`ignore::WalkBuilder` with git-ignore support + SKIP_DIRS) so the
    // staleness check never flags files the ingester would skip — otherwise
    // gitignored source files permanently mark the index stale.
    let walker = ignore::WalkBuilder::new(root_path)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(|entry| {
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                let name = entry.file_name().to_string_lossy();
                return !SKIP_DIRS.iter().any(|skip| *skip == name);
            }
            true
        })
        .build();
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
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
