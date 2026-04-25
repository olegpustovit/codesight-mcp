//! Domain-owned code indexing orchestration: crawl, delta planning, graph construction, persist.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use ignore::WalkBuilder;
use rayon::prelude::*;
use tracing::instrument;

use codesight_storage::storage::vector_index::OwnedVectorRow;

use codesight_storage::storage::IndexStore;

use codesight_core::pipeline::{FileFingerprint, IndexedFileUpdate};

use codesight_language_support::{LanguageRegistry, dep_graph};

use crate::{
    config::{AppConfig, GITIGNORE_WARNING_FILE_THRESHOLD},
    indexing::Indexer,
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
    skip(config, indexer),
    fields(root = %root.display())
)]
pub fn index_workspace(
    config: &AppConfig,
    root: &Path,
    indexer: &Arc<Indexer>,
) -> anyhow::Result<CodeIndexOutcome> {
    index_workspace_with_symbol_vectors(config, root, indexer, |_, _| Ok(Vec::new()))
}

/// Per-file result of the parallel ingest stage.
enum FileOutcome {
    /// Fingerprint matched exactly — nothing to persist. Payload is the
    /// repo-relative path so the caller can record it in `seen_files` and
    /// keep the removed-files diff correct.
    Unchanged(String),
    /// Stat drifted but content hash matches; emit a fingerprint touch.
    Touched(FileFingerprint),
    /// Content differs; symbols were re-extracted.
    Updated {
        update: IndexedFileUpdate,
        /// Kept around so we can hand the raw text to the dep-graph builder
        /// without re-reading from disk.
        source: String,
    },
}

struct WalkedFile {
    abs_path: PathBuf,
    relative_path: String,
    language: String,
    size_bytes: u64,
    modified_ns: u64,
}

pub fn index_workspace_with_symbol_vectors<F>(
    config: &AppConfig,
    root: &Path,
    indexer: &Arc<Indexer>,
    build_symbol_vectors: F,
) -> anyhow::Result<CodeIndexOutcome>
where
    F: FnOnce(&str, &[IndexedFileUpdate]) -> anyhow::Result<Vec<OwnedVectorRow>>,
{
    let repo = normalize_repo_id(root);
    let store = IndexStore::open(config.repo_db_path(&repo))?;
    let index_version = store.current_index_version()?.saturating_add(1);
    let extractor_version = indexer.version_stamp();
    // If the backend's version stamp changed since the last ingest, drop
    // cached file fingerprints so every matching file gets re-extracted
    // even when its content hash hasn't changed. Without this, edits to
    // `runtime/queries/**/tags.scm` or the extractor pipeline would leave
    // stale symbol ids in the index until a full rebuild.
    let stored_stamp = store.stored_extractor_version().unwrap_or_default();
    let existing = if stored_stamp == extractor_version {
        store.file_fingerprints()?
    } else {
        HashMap::new()
    };

    let registry = indexer.registry_for(root);

    let (walked, indexed_languages) = walk_candidates(root, &registry);

    let outcomes: Vec<FileOutcome> = walked
        .into_par_iter()
        .filter_map(|file| process_file(file, &existing, &registry))
        .collect();

    let mut warnings = Vec::new();
    let mut seen_files: HashSet<String> =
        HashSet::with_capacity(outcomes.len().max(existing.len()));
    let mut touched_files = Vec::new();
    let mut updated_files = Vec::new();
    let mut graph_sources: Vec<(String, String)> = Vec::new();
    for outcome in outcomes {
        match outcome {
            FileOutcome::Unchanged(rel) => {
                seen_files.insert(rel);
            }
            FileOutcome::Touched(fp) => {
                seen_files.insert(fp.file_path.clone());
                touched_files.push(fp);
            }
            FileOutcome::Updated { update, source } => {
                seen_files.insert(update.file_path.clone());
                graph_sources.push((update.file_path.clone(), source));
                updated_files.push(update);
            }
        }
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
    let removed_files: Vec<String> = existing
        .keys()
        .filter(|path| !seen_files.contains(*path))
        .cloned()
        .collect();

    let alias_map = codesight_language_support::builtin_registry()
        .get(codesight_language_support::TYPESCRIPT)
        .map(|l| l.path_aliases(root))
        .unwrap_or_default();
    let alias_ref = (!alias_map.is_empty()).then_some(&alias_map);
    let workspace_crates = codesight_language_support::packages::workspace_crates_from_root(root);
    let workspace_ref = (!workspace_crates.is_empty()).then_some(&workspace_crates);
    // Resolution must see every file on disk (updated + touched + unchanged),
    // otherwise an import from a new file to an unchanged file fails to
    // resolve and drops the edge.
    let known_paths: Vec<String> = seen_files.iter().cloned().collect();
    let dep_edges = dep_graph::build_dependency_graph_edges_full(
        &graph_sources,
        Some(&known_paths),
        alias_ref,
        workspace_ref,
    );

    // Reachability edges: per-language structural extraction at ingest
    // time. Replaces the file-scan augmentation passes that used to live
    // in `codesight_analysis::dead_code::reachability`.
    let reachability_edges =
        compute_reachability_edges_for_sources(&graph_sources, &registry);

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
        &reachability_edges,
        &extractor_version,
    )?;

    let package_names =
        codesight_language_support::packages::extract_package_names(&root.to_string_lossy());
    if !package_names.is_empty() {
        let _ = store.set_package_names(&package_names);
    }

    let metadata = store.repo_metadata()?;
    let languages_without_import_extractor: Vec<String> = indexed_languages
        .into_iter()
        .filter(|language| {
            !codesight_language_support::imports_orchestration::supports_import_extraction(language)
        })
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

/// Walk the workspace once, returning the set of on-disk files that carry a
/// recognized language plus the sorted language-id set. Runs on the calling
/// thread — cheap vs the per-file parse work that follows in parallel.
fn walk_candidates(
    root: &Path,
    registry: &LanguageRegistry,
) -> (Vec<WalkedFile>, BTreeSet<String>) {
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(|entry| {
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                let name = entry.file_name().to_string_lossy();
                return !codesight_storage::db::SKIP_DIRS
                    .iter()
                    .any(|skip| *skip == name);
            }
            true
        })
        .build();

    let mut files = Vec::new();
    let mut languages: BTreeSet<String> = BTreeSet::new();
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
        let Some(lang) = registry.for_path(path) else {
            continue;
        };
        let language = lang.id().as_str().to_string();
        languages.insert(language.clone());
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(err) => {
                tracing::warn!(path = %path.display(), error = %err, "skipping file (metadata)");
                continue;
            }
        };
        let Ok(relative_path) = to_repo_relative(path, root) else {
            continue;
        };
        files.push(WalkedFile {
            abs_path: path.to_path_buf(),
            relative_path,
            language,
            size_bytes: metadata.len(),
            modified_ns: modified_ns(&metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH)),
        });
    }
    (files, languages)
}

/// Process a single walked file on a worker thread: read bytes, hash, decide
/// whether to re-extract, and return the resulting [`FileOutcome`].
///
/// Returns `None` on I/O failure — the file is silently dropped from the
/// ingest, mirroring the pre-parallel behavior. Tracing warnings are emitted
/// from the worker thread so they still end up in the structured log.
fn process_file(
    file: WalkedFile,
    existing: &HashMap<String, FileFingerprint>,
    registry: &LanguageRegistry,
) -> Option<FileOutcome> {
    let bytes = match fs::read(&file.abs_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!(path = %file.abs_path.display(), error = %err, "failed to read file");
            return None;
        }
    };
    let content_hash = blake3::hash(&bytes).to_hex().to_string();

    if let Some(prev) = existing.get(&file.relative_path) {
        if prev.size_bytes == file.size_bytes
            && prev.modified_ns == file.modified_ns
            && prev.content_hash == content_hash
        {
            // Fully unchanged — skip the SQL UPDATE entirely. Upstream
            // records the path in `seen_files` via the `Unchanged` payload
            // so the removed-files diff stays correct.
            return Some(FileOutcome::Unchanged(file.relative_path));
        }
        if prev.content_hash == content_hash {
            return Some(FileOutcome::Touched(FileFingerprint {
                file_path: file.relative_path,
                language: codesight_core::LanguageId::intern(&file.language),
                size_bytes: file.size_bytes,
                modified_ns: file.modified_ns,
                content_hash,
            }));
        }
    }

    // Content differs (or never seen) — decode UTF-8 and extract symbols.
    let source = String::from_utf8_lossy(&bytes).into_owned();
    let symbols = registry
        .for_path(&file.abs_path)
        .map(|lang| {
            lang.extract_symbols(&file.relative_path, &source)
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let update = IndexedFileUpdate {
        file_path: file.relative_path,
        language: codesight_core::LanguageId::intern(&file.language),
        size_bytes: file.size_bytes,
        modified_ns: file.modified_ns,
        content_hash,
        symbols,
    };
    Some(FileOutcome::Updated { update, source })
}

/// Run [`Language::reachability_edges`] over each `(path, source)` pair
/// using the workspace registry; returns one entry per source file even
/// when the language returned an empty edge set (so the storage write
/// will clear stale rows for that file).
fn compute_reachability_edges_for_sources(
    sources: &[(String, String)],
    registry: &LanguageRegistry,
) -> Vec<(String, Vec<codesight_core::StructuralEdge>)> {
    sources
        .par_iter()
        .map(|(path, content)| {
            let edges = registry
                .for_path(std::path::Path::new(path))
                .map(|lang| lang.reachability_edges(content, path))
                .unwrap_or_default();
            (path.clone(), edges)
        })
        .collect()
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
    indexer: &Arc<Indexer>,
    changed_paths: &[std::path::PathBuf],
) -> anyhow::Result<DeltaReindexStats> {
    let index_version = store.current_index_version()?.saturating_add(1);
    let extractor_version = indexer.version_stamp();
    let registry = indexer.registry_for(root);

    #[derive(Default)]
    struct DeltaBuckets {
        updated: Vec<IndexedFileUpdate>,
        removed: Vec<String>,
        graph_sources: Vec<(String, String)>,
        symbols_extracted: usize,
    }

    impl DeltaBuckets {
        fn merge(mut self, other: Self) -> Self {
            self.updated.extend(other.updated);
            self.removed.extend(other.removed);
            self.graph_sources.extend(other.graph_sources);
            self.symbols_extracted += other.symbols_extracted;
            self
        }
    }

    let buckets: DeltaBuckets = changed_paths
        .par_iter()
        .filter_map(|abs_path| reindex_single(abs_path, root, &registry))
        .fold(DeltaBuckets::default, |mut acc, outcome| {
            match outcome {
                ReindexOutcome::Removed(rel) => acc.removed.push(rel),
                ReindexOutcome::Updated { update, source } => {
                    acc.symbols_extracted += update.symbols.len();
                    acc.graph_sources.push((update.file_path.clone(), source));
                    acc.updated.push(update);
                }
            }
            acc
        })
        .reduce(DeltaBuckets::default, DeltaBuckets::merge);

    let alias_map = codesight_language_support::builtin_registry()
        .get(codesight_language_support::TYPESCRIPT)
        .map(|l| l.path_aliases(root))
        .unwrap_or_default();
    let alias_ref = (!alias_map.is_empty()).then_some(&alias_map);
    let workspace_crates = codesight_language_support::packages::workspace_crates_from_root(root);
    let workspace_ref = (!workspace_crates.is_empty()).then_some(&workspace_crates);
    // Derive the known-path set from the existing index plus any new/changed
    // files, minus the ones being removed. Resolution needs the full picture
    // so edges from changed files to unchanged neighbors still bind.
    let mut known_paths: Vec<String> = store
        .file_fingerprints()
        .map(|fps| fps.into_keys().collect())
        .unwrap_or_default();
    let removed_set: HashSet<&str> = buckets.removed.iter().map(String::as_str).collect();
    known_paths.retain(|p| !removed_set.contains(p.as_str()));
    for update in &buckets.updated {
        if !known_paths.iter().any(|p| p == &update.file_path) {
            known_paths.push(update.file_path.clone());
        }
    }
    let dep_edges = dep_graph::build_dependency_graph_edges_full(
        &buckets.graph_sources,
        Some(&known_paths),
        alias_ref,
        workspace_ref,
    );

    let reachability_edges =
        compute_reachability_edges_for_sources(&buckets.graph_sources, &registry);

    let indexed_at = chrono::Utc::now().to_rfc3339();
    store.apply_index_with_deps(
        repo,
        root,
        &indexed_at,
        index_version,
        &[],
        &buckets.updated,
        &buckets.removed,
        &[],
        &dep_edges,
        &reachability_edges,
        &extractor_version,
    )?;

    Ok(DeltaReindexStats {
        files_processed: buckets.updated.len(),
        files_removed: buckets.removed.len(),
        symbols_extracted: buckets.symbols_extracted,
    })
}

enum ReindexOutcome {
    Removed(String),
    Updated {
        update: IndexedFileUpdate,
        source: String,
    },
}

fn reindex_single(
    abs_path: &Path,
    root: &Path,
    registry: &LanguageRegistry,
) -> Option<ReindexOutcome> {
    let relative_path = to_repo_relative(abs_path, root).ok()?;

    if !abs_path.exists() {
        return Some(ReindexOutcome::Removed(relative_path));
    }

    let metadata = fs::metadata(abs_path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let lang = registry.for_path(abs_path)?;
    let language = lang.id().as_str().to_string();

    let size_bytes = metadata.len();
    let mod_ns = modified_ns(&metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH));

    let bytes = match fs::read(abs_path) {
        Ok(b) => b,
        Err(err) => {
            tracing::warn!(path = %abs_path.display(), error = %err, "failed to read");
            return None;
        }
    };
    let content_hash = blake3::hash(&bytes).to_hex().to_string();
    let source = String::from_utf8_lossy(&bytes).into_owned();
    let symbols = lang
        .extract_symbols(&relative_path, &source)
        .unwrap_or_default();
    Some(ReindexOutcome::Updated {
        update: IndexedFileUpdate {
            file_path: relative_path,
            language: codesight_core::LanguageId::intern(&language),
            size_bytes,
            modified_ns: mod_ns,
            content_hash,
            symbols,
        },
        source,
    })
}
