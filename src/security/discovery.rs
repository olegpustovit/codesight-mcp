use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use super::binary_policy::{ExcludeOptions, should_exclude_file};

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
    pub files: Vec<PathBuf>,
    pub warnings: Vec<DiscoverWarning>,
    pub truncated: bool,
}

/// Discovers files under `root` applying security filters.
pub fn discover_files(root: &Path, opts: &DiscoverOptions) -> DiscoverResult {
    let mut builder = WalkBuilder::new(root);
    builder.follow_links(opts.follow_symlinks);
    builder.git_ignore(opts.respect_gitignore);
    builder.hidden(false);

    if !opts.extra_ignore_patterns.is_empty() {
        let mut overrides = ignore::overrides::OverrideBuilder::new(root);
        for pattern in &opts.extra_ignore_patterns {
            overrides.add(&format!("!{pattern}")).ok();
        }
        if let Ok(built) = overrides.build() {
            builder.overrides(built);
        }
    }

    let mut files = Vec::new();
    let mut warnings = Vec::new();
    let mut truncated = false;

    for entry in builder.build() {
        let Ok(entry) = entry else { continue };
        if entry.file_type().is_none_or(|file_type| file_type.is_dir()) {
            continue;
        }

        let path = entry.path();
        if !opts.follow_symlinks
            && let Ok(meta) = std::fs::symlink_metadata(path)
            && meta.file_type().is_symlink()
        {
            continue;
        }

        if let Some(reason) = should_exclude_file(path, root, &opts.exclude) {
            warnings.push(DiscoverWarning {
                path: path.to_string_lossy().into_owned(),
                reason: reason.as_str().to_string(),
            });
            continue;
        }

        files.push(path.to_path_buf());
        if let Some(max_files) = opts.max_files
            && files.len() >= max_files
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
