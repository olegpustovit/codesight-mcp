//! Domain-owned code indexing orchestration: crawl, delta planning, graph construction, persist.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use ignore::WalkBuilder;
use tracing::instrument;

use codesight_storage::storage::vector_index::OwnedVectorRow;

use codesight_storage::storage::IndexStore;

use crate::{
    config::{AppConfig, GITIGNORE_WARNING_FILE_THRESHOLD},
    core::pipeline::{FileFingerprint, IndexedFileUpdate},
    extractors::ExtractorRegistry,
    indexing::dep_graph,
    repo_id::normalize_repo_id,
};

#[derive(Debug, Clone)]
pub struct IndexWarning {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct CodeIndexSummary {
    pub repo: String,
    pub indexed_at: String,
    pub file_count: usize,
    pub symbol_count: usize,
    pub languages: BTreeMap<String, usize>,
    pub languages_without_import_extractor: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct IngestDelta {
    pub files_reindexed: usize,
    pub files_removed: usize,
}

#[derive(Debug, Clone)]
pub struct CodeIndexOutcome {
    pub summary: CodeIndexSummary,
    pub warnings: Vec<IndexWarning>,
    pub delta: IngestDelta,
}

#[instrument(
    level = "info",
    name = "ingest",
    skip(config),
    fields(root = %root.display())
)]
pub fn index_workspace(config: &AppConfig, root: &Path) -> anyhow::Result<CodeIndexOutcome> {
    index_workspace_with_symbol_vectors(config, root, |_, _| Ok(Vec::new()))
}

pub fn index_workspace_with_symbol_vectors<F>(
    config: &AppConfig,
    root: &Path,
    build_symbol_vectors: F,
) -> anyhow::Result<CodeIndexOutcome>
where
    F: FnOnce(&str, &[IndexedFileUpdate]) -> anyhow::Result<Vec<OwnedVectorRow>>,
{
    let repo = normalize_repo_id(root);
    let store = IndexStore::open(config.repo_db_path(&repo))?;
    let index_version = store.current_index_version()?.saturating_add(1);
    let extractor_version =
        codesight_extractors::extractors::engine::catalogs::builtin_extractor_version_stamp();
    // If the extractor/query-pack stamp changed since the last ingest,
    // drop cached file fingerprints so every matching file gets re-extracted
    // even when its content hash hasn't changed. Without this, edits to
    // `runtime/queries/**/tags.scm` or the Rust extractor pipeline would
    // leave stale symbol ids in the index until a full rebuild.
    let stored_stamp = store.stored_extractor_version().unwrap_or_default();
    let existing = if stored_stamp == extractor_version {
        store.file_fingerprints()?
    } else {
        std::collections::HashMap::new()
    };
    let extractors = ExtractorRegistry::for_workspace_root(root);

    let mut seen_files = HashSet::new();
    let mut touched_files = Vec::new();
    let mut updated_files = Vec::new();
    let mut warnings = Vec::new();
    let mut indexed_languages: BTreeSet<String> = BTreeSet::new();
    let mut graph_sources: Vec<(String, String)> = Vec::new();

    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(|entry| {
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                let name = entry.file_name().to_string_lossy();
                return !crate::db::SKIP_DIRS.iter().any(|skip| *skip == name);
            }
            true
        })
        .build();

    for result in walker {
        let entry = match result {
            Ok(entry) => entry,
            Err(err) => {
                tracing::warn!(error = %err, "walk error during code index");
                continue;
            }
        };

        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        let path = entry.path();
        let Some(extractor) = extractors.extractor_for_path(path) else {
            continue;
        };

        indexed_languages.insert(extractor.language_name().to_string());

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(err) => {
                tracing::warn!(path = %path.display(), error = %err, "skipping file (metadata)");
                continue;
            }
        };

        let relative_path = to_repo_relative(path, root)?;
        seen_files.insert(relative_path.clone());

        let size_bytes = metadata.len();
        let modified_ns = modified_ns(&metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH));

        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(path = %path.display(), error = %err, "failed to read file");
                continue;
            }
        };
        let content_hash = blake3::hash(&bytes).to_hex().to_string();
        let source = String::from_utf8_lossy(&bytes).into_owned();
        graph_sources.push((relative_path.clone(), source.clone()));

        if let Some(existing_file) = existing.get(&relative_path) {
            if existing_file.size_bytes == size_bytes
                && existing_file.modified_ns == modified_ns
                && existing_file.content_hash == content_hash
            {
                continue;
            }
            if existing_file.content_hash == content_hash {
                touched_files.push(FileFingerprint {
                    file_path: relative_path,
                    language: extractor.language_name().to_string(),
                    size_bytes,
                    modified_ns,
                    content_hash,
                });
                continue;
            }
        }

        let symbols = match extractor.extract(&relative_path, &source) {
            Ok(symbols) => symbols,
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    error = format!("{err:#}"),
                    "tree-sitter extraction failed"
                );
                continue;
            }
        };

        updated_files.push(IndexedFileUpdate {
            file_path: relative_path,
            language: extractor.language_name().to_string(),
            size_bytes,
            modified_ns,
            content_hash,
            symbols,
        });
    }

    let total_indexed = seen_files.len();
    let has_root_gitignore = root.join(".gitignore").exists();
    if !has_root_gitignore && total_indexed >= GITIGNORE_WARNING_FILE_THRESHOLD {
        warnings.push(IndexWarning {
            code: "NO_GITIGNORE",
            message: format!(
                "No .gitignore found at project root and {total_indexed} files were indexed. Consider adding a .gitignore to exclude generated or vendored files."
            ),
        });
    }

    if seen_files.is_empty() && !existing.is_empty() {
        anyhow::bail!(
            "empty_walk_guard_tripped: disk walk found zero files while index tracks {} existing files; refusing to delete the index",
            existing.len()
        );
    }
    let removed_files = existing
        .keys()
        .filter(|path| !seen_files.contains(*path))
        .cloned()
        .collect::<Vec<_>>();

    let alias_map = codesight_imports::imports::load_tsconfig_aliases(&root.to_string_lossy());
    let alias_ref = (!alias_map.is_empty()).then_some(&alias_map);
    let dep_edges = dep_graph::build_dependency_graph_edges_with_aliases(&graph_sources, alias_ref);

    let indexed_at = chrono::Utc::now().to_rfc3339();
    let symbol_vectors = build_symbol_vectors(&repo, &updated_files)?;
    store.apply_index_with_deps(
        &repo,
        root,
        &indexed_at,
        index_version,
        &touched_files,
        &updated_files,
        &removed_files,
        &symbol_vectors,
        &dep_edges,
        extractor_version,
    )?;

    let package_names = codesight_imports::packages::extract_package_names(&root.to_string_lossy());
    if !package_names.is_empty() {
        let _ = store.set_package_names(&package_names);
    }

    let metadata = store.repo_metadata()?;
    let languages_without_import_extractor: Vec<String> = indexed_languages
        .into_iter()
        .filter(|language| !codesight_imports::imports::supports_import_extraction(language))
        .collect();

    let files_reindexed = updated_files.len();
    let files_removed = removed_files.len();
    Ok(CodeIndexOutcome {
        summary: CodeIndexSummary {
            repo,
            indexed_at,
            file_count: metadata.file_count,
            symbol_count: metadata.symbol_count,
            languages: metadata.languages,
            languages_without_import_extractor,
        },
        warnings,
        delta: IngestDelta {
            files_reindexed,
            files_removed,
        },
    })
}

fn modified_ns(time: &SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or_default()
}

fn to_repo_relative(path: &Path, root: &Path) -> anyhow::Result<String> {
    let relative = path.strip_prefix(root)?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

#[derive(Debug, Clone)]
pub struct DeltaReindexStats {
    pub files_processed: usize,
    pub files_removed: usize,
    pub symbols_extracted: usize,
}

pub fn reindex_files(
    _config: &AppConfig,
    root: &Path,
    repo: &str,
    store: &IndexStore,
    extractors: &ExtractorRegistry,
    changed_paths: &[std::path::PathBuf],
) -> anyhow::Result<DeltaReindexStats> {
    let index_version = store.current_index_version()?.saturating_add(1);
    let extractor_version =
        codesight_extractors::extractors::engine::catalogs::builtin_extractor_version_stamp();

    let mut updated_files = Vec::new();
    let mut removed_files = Vec::new();
    let mut graph_sources: Vec<(String, String)> = Vec::new();
    let mut symbols_extracted = 0usize;

    for abs_path in changed_paths {
        let relative_path = match to_repo_relative(abs_path, root) {
            Ok(r) => r,
            Err(_) => continue,
        };

        if !abs_path.exists() {
            removed_files.push(relative_path);
            continue;
        }

        let metadata = match fs::metadata(abs_path) {
            Ok(m) if m.is_file() => m,
            _ => continue,
        };

        let Some(extractor) = extractors.extractor_for_path(abs_path) else {
            continue;
        };

        let size_bytes = metadata.len();
        let mod_ns = modified_ns(&metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH));

        let bytes = match fs::read(abs_path) {
            Ok(b) => b,
            Err(err) => {
                tracing::warn!(path = %abs_path.display(), error = %err, "failed to read");
                continue;
            }
        };
        let content_hash = blake3::hash(&bytes).to_hex().to_string();
        let source = String::from_utf8_lossy(&bytes).into_owned();
        graph_sources.push((relative_path.clone(), source.clone()));

        let symbols = match extractor.extract(&relative_path, &source) {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!(path = %abs_path.display(), error = format!("{err:#}"), "extraction failed");
                continue;
            }
        };

        symbols_extracted += symbols.len();
        updated_files.push(IndexedFileUpdate {
            file_path: relative_path,
            language: extractor.language_name().to_string(),
            size_bytes,
            modified_ns: mod_ns,
            content_hash,
            symbols,
        });
    }

    let alias_map = codesight_imports::imports::load_tsconfig_aliases(&root.to_string_lossy());
    let alias_ref = (!alias_map.is_empty()).then_some(&alias_map);
    let dep_edges = dep_graph::build_dependency_graph_edges_with_aliases(&graph_sources, alias_ref);

    let indexed_at = chrono::Utc::now().to_rfc3339();
    store.apply_index_with_deps(
        repo,
        root,
        &indexed_at,
        index_version,
        &[],
        &updated_files,
        &removed_files,
        &[],
        &dep_edges,
        extractor_version,
    )?;

    Ok(DeltaReindexStats {
        files_processed: updated_files.len(),
        files_removed: removed_files.len(),
        symbols_extracted,
    })
}
