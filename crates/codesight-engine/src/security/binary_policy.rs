use std::fs;
use std::path::Path;

use super::config::DEFAULT_MAX_FILE_SIZE;
use super::secret_policy::is_secret_file;

/// Extensions considered binary (will not be indexed).
const BINARY_EXTENSIONS: &[&str] = &[
    "exe", "dll", "so", "dylib", "a", "o", "obj", "lib", "png", "jpg", "jpeg", "gif", "bmp", "ico",
    "svg", "webp", "tiff", "tif", "mp3", "mp4", "wav", "avi", "mkv", "flv", "mov", "ogg", "webm",
    "zip", "tar", "gz", "bz2", "xz", "7z", "rar", "zst", "wasm", "pyc", "pyo", "class", "beam",
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "db", "sqlite", "sqlite3", "mdb", "ttf",
    "otf", "woff", "woff2", "eot", "iso", "img", "dmg", "bin", "dat",
];

/// Returns `true` if the file extension signals a binary format.
///
/// This filename-based test is intentional policy. It lets indexing reject
/// known binary formats before doing any content reads.
pub fn is_binary_extension(filename: &str) -> bool {
    let ext = match Path::new(filename).extension() {
        Some(ext) => ext.to_string_lossy().to_lowercase(),
        None => return false,
    };
    BINARY_EXTENSIONS.contains(&ext.as_str())
}

/// Returns `true` if `data` contains null bytes (a heuristic for binary content).
pub fn is_binary_content(data: &[u8]) -> bool {
    data.contains(&0)
}

/// Returns `true` if the file at `path` is binary, checking extension first,
/// then sampling file content for null bytes.
pub fn is_binary_file(path: &Path) -> bool {
    if let Some(name) = path.file_name()
        && is_binary_extension(&name.to_string_lossy())
    {
        return true;
    }
    if let Ok(data) = fs::read(path) {
        let sample_len = data.len().min(8192);
        return is_binary_content(&data[..sample_len]);
    }
    false
}

/// Reason a file was excluded by [`should_exclude_file`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExcludeReason {
    SecretFile,
    BinaryExtension,
    BinaryContent,
    FileTooLarge,
}

impl ExcludeReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SecretFile => "secret_file",
            Self::BinaryExtension => "binary_extension",
            Self::BinaryContent => "binary_content",
            Self::FileTooLarge => "file_too_large",
        }
    }
}

impl std::fmt::Display for ExcludeReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Options for [`should_exclude_file`].
#[derive(Debug, Clone)]
pub struct ExcludeOptions {
    pub check_secrets: bool,
    pub max_file_size: u64,
}

impl Default for ExcludeOptions {
    fn default() -> Self {
        Self {
            check_secrets: true,
            max_file_size: DEFAULT_MAX_FILE_SIZE,
        }
    }
}

/// Returns `Some(reason)` if the file should be excluded from indexing,
/// or `None` if it passes all checks.
pub fn should_exclude_file(
    path: &Path,
    _root: &Path,
    opts: &ExcludeOptions,
) -> Option<ExcludeReason> {
    if opts.check_secrets {
        let rel = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        let path_str = path.to_string_lossy();
        if is_secret_file(&rel) || is_secret_file(&path_str) {
            return Some(ExcludeReason::SecretFile);
        }
    }

    if let Some(name) = path.file_name()
        && is_binary_extension(&name.to_string_lossy())
    {
        return Some(ExcludeReason::BinaryExtension);
    }

    if let Ok(meta) = fs::metadata(path)
        && meta.len() > opts.max_file_size
    {
        return Some(ExcludeReason::FileTooLarge);
    }

    if let Ok(data) = fs::read(path) {
        let sample_len = data.len().min(8192);
        if is_binary_content(&data[..sample_len]) {
            return Some(ExcludeReason::BinaryContent);
        }
    }

    None
}
