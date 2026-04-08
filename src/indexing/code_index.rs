//! Domain-owned code indexing orchestration: crawl, delta planning, graph construction, persist.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use ignore::WalkBuilder;

use crate::{
    config::{AppConfig, GITIGNORE_WARNING_FILE_THRESHOLD},
    core::pipeline::{FileFingerprint, IndexedFileUpdate},
    dep_graph,
    extractors::ExtractorRegistry,
    indexing::packages,
    repo_id::normalize_repo_id,
    storage::IndexStore,
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
    pub missing_extractors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CodeIndexOutcome {
    pub summary: CodeIndexSummary,
    pub warnings: Vec<IndexWarning>,
}

pub fn index_workspace(config: &AppConfig, root: &Path) -> anyhow::Result<CodeIndexOutcome> {
    let repo = normalize_repo_id(root);
    let store = IndexStore::open(config.repo_db_path(&repo))?;
    let index_version = store.current_index_version()?.saturating_add(1);
    let existing = store.file_fingerprints()?;
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

    let max_files = config.max_folder_files;

    for result in walker {
        let entry = match result {
            Ok(entry) => entry,
            Err(err) => {
                eprintln!("codesight-mcp: walk error: {err}");
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

        if max_files > 0 && seen_files.len() >= max_files {
            break;
        }

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(err) => {
                eprintln!("codesight-mcp: skipping {}: {err}", path.display());
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
                eprintln!("codesight-mcp: failed to read {}: {err}", path.display());
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
                eprintln!(
                    "codesight-mcp: tree-sitter extraction failed for {}: {err:#}",
                    path.display()
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

    let removed_files = existing
        .keys()
        .filter(|path| !seen_files.contains(*path))
        .cloned()
        .collect::<Vec<_>>();

    let alias_map = crate::indexing::imports::load_tsconfig_aliases(&root.to_string_lossy());
    let alias_ref = (!alias_map.is_empty()).then_some(&alias_map);
    let adjacency = dep_graph::build_dependency_graph_with_aliases(&graph_sources, alias_ref);
    let dep_edges: Vec<(String, String)> = adjacency
        .iter()
        .flat_map(|(source, targets)| {
            targets
                .iter()
                .map(move |target| (source.clone(), target.clone()))
        })
        .collect();

    let indexed_at = chrono::Utc::now().to_rfc3339();
    store.apply_index_with_deps(
        &repo,
        root,
        &indexed_at,
        index_version,
        &touched_files,
        &updated_files,
        &removed_files,
        &dep_edges,
    )?;

    let package_names = packages::extract_package_names(&root.to_string_lossy());
    if !package_names.is_empty() {
        let _ = store.set_package_names(&package_names);
    }

    let metadata = store.repo_metadata()?;
    let missing_extractors: Vec<String> = indexed_languages
        .into_iter()
        .filter(|language| !crate::indexing::imports::supports_import_extraction(language))
        .collect();

    Ok(CodeIndexOutcome {
        summary: CodeIndexSummary {
            repo,
            indexed_at,
            file_count: metadata.file_count,
            symbol_count: metadata.symbol_count,
            languages: metadata.languages,
            missing_extractors,
        },
        warnings,
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
