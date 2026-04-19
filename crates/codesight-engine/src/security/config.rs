/// Default maximum file size in bytes (500 KB).
pub const DEFAULT_MAX_FILE_SIZE: u64 = 500 * 1024;

/// Default maximum number of files to index across a whole repo/source.
pub const DEFAULT_MAX_INDEX_FILES: usize = 10_000;

/// Default maximum number of files to list in a single folder operation.
pub const DEFAULT_MAX_FOLDER_FILES: usize = 5_000;

/// Configuration for security limits, loadable from external config.
#[derive(Debug, Clone, Default)]
pub struct SecurityConfig {
    pub max_index_files: Option<usize>,
    pub max_folder_files: Option<usize>,
    pub extra_ignore_patterns: Vec<String>,
}

/// Returns the maximum number of files to index, respecting an optional
/// explicit override. Returns `Err` if `explicit` is `Some(0)`.
pub fn get_max_index_files(
    config: &SecurityConfig,
    explicit: Option<usize>,
) -> Result<usize, &'static str> {
    if let Some(value) = explicit {
        if value == 0 {
            return Err("max_index_files must be a positive integer");
        }
        return Ok(value);
    }
    if let Some(value) = config.max_index_files
        && value > 0
    {
        return Ok(value);
    }
    Ok(DEFAULT_MAX_INDEX_FILES)
}

/// Returns the maximum number of files for a folder operation.
/// Returns `Err` if `explicit` is `Some(0)`.
pub fn get_max_folder_files(
    config: &SecurityConfig,
    explicit: Option<usize>,
) -> Result<usize, &'static str> {
    if let Some(value) = explicit {
        if value == 0 {
            return Err("max_folder_files must be a positive integer");
        }
        return Ok(value);
    }
    if let Some(value) = config.max_folder_files
        && value > 0
    {
        return Ok(value);
    }
    Ok(DEFAULT_MAX_FOLDER_FILES)
}

/// Returns merged ignore patterns from config and an explicit call-site list.
pub fn get_extra_ignore_patterns(config: &SecurityConfig, call_patterns: &[&str]) -> Vec<String> {
    let mut result = config.extra_ignore_patterns.clone();
    for pattern in call_patterns {
        result.push(pattern.to_string());
    }
    result
}
