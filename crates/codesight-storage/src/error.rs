//! Typed failures for persistence and SQLite I/O. Map to `anyhow::Error` at the MCP boundary via `?`.

use thiserror::Error;

/// Result alias for storage operations.
pub type StorageResult<T> = Result<T, StorageError>;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    WalkDir(#[from] walkdir::Error),
    /// Catch-all for typed storage-layer errors that don't map to a more
    /// specific variant — e.g. an index file written by a newer binary
    /// (`user_version > target`) when the running binary can't downgrade.
    #[error("{0}")]
    Other(String),
}
