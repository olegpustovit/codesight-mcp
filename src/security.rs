//! Security hardening utilities for path validation, secret detection,
//! binary detection, encoding safety, and composite file filtering.

use std::fs;
use std::path::Path;

// ── Defaults ────────────────────────────────────────────────────────────────

/// Default maximum file size in bytes (500 KB).
pub const DEFAULT_MAX_FILE_SIZE: u64 = 500 * 1024;

/// Default maximum number of files to index across a whole repo/source.
pub const DEFAULT_MAX_INDEX_FILES: usize = 10_000;

/// Default maximum number of files to list in a single folder operation.
pub const DEFAULT_MAX_FOLDER_FILES: usize = 5_000;

// ── Path Traversal Prevention (S-01) ────────────────────────────────────────

/// Returns `true` if `path` is logically contained within `root`.
///
/// Both paths are canonicalized (resolving `..`, symlinks, etc.) before
/// comparison, so `root/a/../../etc/passwd` is correctly rejected.
/// If either path cannot be canonicalized the result is `false`.
pub fn validate_path(root: &Path, path: &Path) -> bool {
    let Ok(canon_root) = fs::canonicalize(root) else {
        return false;
    };
    let Ok(canon_path) = fs::canonicalize(path) else {
        // The path may not exist yet — try canonicalizing the parent.
        // If the path *is* root, we already handled that above.
        if path == root {
            return true;
        }
        // For non-existent paths, resolve what we can.
        // Try to canonicalize the path without the last component.
        if let Some(parent) = path.parent()
            && let Ok(canon_parent) = fs::canonicalize(parent)
        {
            let full = canon_parent.join(path.file_name().unwrap_or_default());
            return full.starts_with(&canon_root);
        }
        return false;
    };
    canon_path.starts_with(&canon_root)
}

// ── Symlink Escape Protection (S-02) ────────────────────────────────────────

/// Returns `true` if `path` is a symlink whose target resolves outside `root`.
///
/// Non-symlinks always return `false`.
pub fn is_symlink_escape(root: &Path, path: &Path) -> bool {
    // Check if it is a symlink at all.
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.file_type().is_symlink() {
        return false;
    }
    // Resolve the symlink target and check containment.
    let Ok(canon_root) = fs::canonicalize(root) else {
        return true; // can't verify root — conservative reject
    };
    let Ok(canon_target) = fs::canonicalize(path) else {
        return true; // dangling symlink — conservative reject
    };
    !canon_target.starts_with(&canon_root)
}

// ── Secret Detection (S-04) ─────────────────────────────────────────────────

/// Known documentation extensions that should NOT be flagged even if the
/// filename contains "secret".
const DOC_EXTENSIONS: &[&str] = &[".md", ".rst", ".txt", ".adoc", ".html", ".htm", ".ipynb"];

/// File extensions that are always secret material.
const SECRET_EXTENSIONS: &[&str] = &[".pem", ".key", ".p12", ".pfx", ".jks", ".keystore"];

/// Exact basenames (case-insensitive) that are secret.
const SECRET_BASENAMES: &[&str] = &[
    ".env",
    ".htpasswd",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".secrets",
    "id_rsa",
    "id_rsa.pub",
    "id_ed25519",
    "id_ed25519.pub",
    "id_dsa",
    "id_ecdsa",
    "credentials.json",
];

/// Basename patterns that signal secrets (checked case-insensitively).
const SECRET_SUFFIXES: &[&str] = &[
    ".env",     // catches config.env, .env.local, .env.production
    ".secrets", // catches app.secrets
];

/// Returns `true` if the file at `path` (may be just a filename or a
/// relative/absolute path) looks like a secret or credential file.
///
/// Documentation files whose names happen to contain "secret" are NOT
/// flagged (e.g. `docs/secrets-handling.md`).
pub fn is_secret_file(path: &str) -> bool {
    let p = Path::new(path);
    let basename = p
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let lower_basename = basename.to_lowercase();
    let lower_path = path.to_lowercase();

    // Check if the file has a documentation extension — if so, never flag.
    for ext in DOC_EXTENSIONS {
        if lower_basename.ends_with(ext) {
            return false;
        }
    }

    // Exact basename matches (case-insensitive).
    for name in SECRET_BASENAMES {
        if lower_basename == *name {
            return true;
        }
    }

    // Secret extensions.
    for ext in SECRET_EXTENSIONS {
        if lower_basename.ends_with(ext) {
            return true;
        }
    }

    // .env variants: basename starts with .env or ends with .env
    if lower_basename.starts_with(".env") || lower_basename.ends_with(".env") {
        return true;
    }

    // Basename or path contains "secrets" or "secret" as a significant component
    // (but not doc files — already excluded above).
    // Non-doc files with "secret" in name/path.
    if lower_basename.contains("secret") {
        return true;
    }
    // Also check path components for secrets in directory names
    // e.g. config/secrets.yaml → basename is secrets.yaml which contains "secret"
    // Already handled above.

    // .secrets suffix on basename
    for suffix in SECRET_SUFFIXES {
        if lower_basename.ends_with(suffix) && !lower_basename.starts_with(".env") {
            return true;
        }
    }

    // Check for credential-like patterns in basename
    if lower_basename == "credentials" || lower_basename.starts_with("credentials.") {
        return true;
    }

    // Check path for secrets in intermediate directories — but only for
    // the basename part (already covered above). For paths like
    // config/secrets.json the basename "secrets.json" contains "secret".
    let _ = lower_path;

    false
}

// ── Binary Detection (S-05) ─────────────────────────────────────────────────

/// Extensions considered binary (will not be indexed).
const BINARY_EXTENSIONS: &[&str] = &[
    // Executables / libraries
    "exe", "dll", "so", "dylib", "a", "o", "obj", "lib", // Images
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "svg", "webp", "tiff", "tif",
    // Audio / video
    "mp3", "mp4", "wav", "avi", "mkv", "flv", "mov", "ogg", "webm", // Archives
    "zip", "tar", "gz", "bz2", "xz", "7z", "rar", "zst",  // WebAssembly
    "wasm", // Compiled / bytecode
    "pyc", "pyo", "class", "beam", // Documents
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", // Databases
    "db", "sqlite", "sqlite3", "mdb", // Fonts
    "ttf", "otf", "woff", "woff2", "eot", // Disk images
    "iso", "img", "dmg", // Other
    "bin", "dat",
];

/// Returns `true` if the file extension signals a binary format.
pub fn is_binary_extension(filename: &str) -> bool {
    let p = Path::new(filename);
    let ext = match p.extension() {
        Some(e) => e.to_string_lossy().to_lowercase(),
        None => return false,
    };
    BINARY_EXTENSIONS.contains(&ext.as_str())
}

/// Returns `true` if `data` contains null bytes (a heuristic for binary content).
///
/// Empty data is considered non-binary.
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
    // Read a small prefix to check for null bytes.
    if let Ok(data) = fs::read(path) {
        let sample_len = data.len().min(8192);
        return is_binary_content(&data[..sample_len]);
    }
    false
}

// ── Encoding Safety (S-06) ──────────────────────────────────────────────────

/// Decodes `data` as UTF-8, replacing invalid byte sequences with U+FFFD.
///
/// Never panics.
pub fn safe_decode(data: &[u8]) -> String {
    String::from_utf8_lossy(data).into_owned()
}

// ── Composite Filter ────────────────────────────────────────────────────────

/// Reason a file was excluded by [`should_exclude_file`].
///
/// The string values match the reference test expectations.
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
    /// Whether to check for secret files. Default: `true`.
    pub check_secrets: bool,
    /// Maximum file size in bytes. Default: [`DEFAULT_MAX_FILE_SIZE`].
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
    // 1. Secret check.
    if opts.check_secrets {
        let rel = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        // Use full path string for is_secret_file so subdirectory context is available.
        let path_str = path.to_string_lossy();
        if is_secret_file(&rel) || is_secret_file(&path_str) {
            return Some(ExcludeReason::SecretFile);
        }
    }

    // 2. Binary extension check.
    if let Some(name) = path.file_name()
        && is_binary_extension(&name.to_string_lossy())
    {
        return Some(ExcludeReason::BinaryExtension);
    }

    // 3. File size check (requires the file to exist).
    if let Ok(meta) = fs::metadata(path)
        && meta.len() > opts.max_file_size
    {
        return Some(ExcludeReason::FileTooLarge);
    }

    // 4. Binary content check (sample).
    if let Ok(data) = fs::read(path) {
        let sample_len = data.len().min(8192);
        if is_binary_content(&data[..sample_len]) {
            return Some(ExcludeReason::BinaryContent);
        }
    }

    None
}

// ── Config helpers ──────────────────────────────────────────────────────────

/// Configuration for security limits, loadable from external config.
#[derive(Debug, Clone, Default)]
pub struct SecurityConfig {
    pub max_index_files: Option<usize>,
    pub max_folder_files: Option<usize>,
    pub extra_ignore_patterns: Vec<String>,
}

/// Returns the maximum number of files to index, respecting an optional
/// explicit override. Returns `Err` if `explicit` is `Some(0)` or negative-equivalent.
pub fn get_max_index_files(
    config: &SecurityConfig,
    explicit: Option<usize>,
) -> Result<usize, &'static str> {
    if let Some(v) = explicit {
        if v == 0 {
            return Err("max_index_files must be a positive integer");
        }
        return Ok(v);
    }
    if let Some(v) = config.max_index_files
        && v > 0
    {
        return Ok(v);
    }
    // Invalid config value — fall back to default.
    Ok(DEFAULT_MAX_INDEX_FILES)
}

/// Returns the maximum number of files for a folder operation.
/// Returns `Err` if `explicit` is `Some(0)`.
pub fn get_max_folder_files(
    config: &SecurityConfig,
    explicit: Option<usize>,
) -> Result<usize, &'static str> {
    if let Some(v) = explicit {
        if v == 0 {
            return Err("max_folder_files must be a positive integer");
        }
        return Ok(v);
    }
    if let Some(v) = config.max_folder_files
        && v > 0
    {
        return Ok(v);
    }
    // Invalid config value — fall back to default.
    Ok(DEFAULT_MAX_FOLDER_FILES)
}

/// Returns merged ignore patterns from config and an explicit call-site list.
pub fn get_extra_ignore_patterns(config: &SecurityConfig, call_patterns: &[&str]) -> Vec<String> {
    let mut result: Vec<String> = config.extra_ignore_patterns.clone();
    for p in call_patterns {
        result.push(p.to_string());
    }
    result
}

// ── Discovery helpers ───────────────────────────────────────────────────────

/// Options for secure file discovery.
#[derive(Debug, Clone)]
pub struct DiscoverOptions {
    pub exclude: ExcludeOptions,
    pub max_files: Option<usize>,
    pub extra_ignore_patterns: Vec<String>,
    pub follow_symlinks: bool,
    pub respect_gitignore: bool,
}

impl Default for DiscoverOptions {
    fn default() -> Self {
        Self {
            exclude: ExcludeOptions::default(),
            max_files: None,
            extra_ignore_patterns: Vec::new(),
            follow_symlinks: false,
            respect_gitignore: true,
        }
    }
}

/// A warning produced during file discovery.
#[derive(Debug, Clone)]
pub struct DiscoverWarning {
    pub path: String,
    pub reason: String,
}

/// Result of secure file discovery.
#[derive(Debug, Clone)]
pub struct DiscoverResult {
    pub files: Vec<std::path::PathBuf>,
    pub warnings: Vec<DiscoverWarning>,
    pub truncated: bool,
}

/// Discovers files under `root` applying security filters.
///
/// Uses the `ignore` crate to respect `.gitignore` (when enabled) and
/// applies secret/binary/size filtering on each candidate.
pub fn discover_files(root: &Path, opts: &DiscoverOptions) -> DiscoverResult {
    use ignore::WalkBuilder;

    let mut builder = WalkBuilder::new(root);
    builder.follow_links(opts.follow_symlinks);
    builder.git_ignore(opts.respect_gitignore);
    builder.hidden(false); // don't skip hidden files — we filter secrets ourselves

    // Add extra ignore globs.
    for pattern in &opts.extra_ignore_patterns {
        // The ignore crate accepts gitignore-style patterns via an override.
        let mut overrides = ignore::overrides::OverrideBuilder::new(root);
        overrides.add(&format!("!{pattern}")).ok();
        // Actually, use the simpler approach: add as a custom ignore pattern.
        builder.add_custom_ignore_filename(""); // no-op anchor
    }

    // For extra ignore patterns, use the overrides mechanism properly.
    if !opts.extra_ignore_patterns.is_empty() {
        let mut ovr = ignore::overrides::OverrideBuilder::new(root);
        for pattern in &opts.extra_ignore_patterns {
            // Negate to exclude: "!pattern" in override means DO include,
            // so we want the opposite. The override system: matching = include.
            // To exclude, we use the glob negation trick.
            ovr.add(&format!("!{pattern}")).ok();
        }
        if let Ok(built) = ovr.build() {
            builder.overrides(built);
        }
    }

    let mut files = Vec::new();
    let mut warnings = Vec::new();
    let mut truncated = false;

    for entry in builder.build() {
        let Ok(entry) = entry else { continue };

        // Skip directories.
        if entry.file_type().is_none_or(|ft| ft.is_dir()) {
            continue;
        }

        let path = entry.path();

        // Skip symlinks unless following.
        if !opts.follow_symlinks
            && let Ok(meta) = fs::symlink_metadata(path)
            && meta.file_type().is_symlink()
        {
            continue;
        }

        // Apply composite filter.
        if let Some(reason) = should_exclude_file(path, root, &opts.exclude) {
            warnings.push(DiscoverWarning {
                path: path.to_string_lossy().into_owned(),
                reason: reason.as_str().to_string(),
            });
            continue;
        }

        files.push(path.to_path_buf());

        // Check file limit.
        if let Some(max) = opts.max_files
            && files.len() >= max
        {
            truncated = true;
            break;
        }
    }

    DiscoverResult {
        files,
        warnings,
        truncated,
    }
}
