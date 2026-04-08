use std::{
    collections::{BTreeSet, HashSet},
    fs,
    time::SystemTime,
};

use anyhow::Context;
use ignore::WalkBuilder;
use serde_json::{Value, json};

use super::args::*;
use super::helpers::{slice_file_lines, *};
use super::{CodeService, MAX_QUERY_LEN};
use crate::{
    config::GITIGNORE_WARNING_FILE_THRESHOLD,
    dep_graph,
    model::{FileFingerprint, IndexedFileUpdate, RetrievedSymbol, SearchSymbol},
    repo_id::normalize_repo_id,
    savings::MetaBuilder,
    storage::{self, IndexStore},
};

/// Languages that have import/dependency extraction support in the dep-graph
/// builder.  Languages indexed but absent from this set produce a
/// `missing_extractors` entry in the ingest response.
const LANGUAGES_WITH_IMPORT_EXTRACTION: &[&str] = &["python"];

impl CodeService {
    /// Index a local folder, returning the result plus any warnings generated
    /// during crawl (e.g. gitignore absence warnings).
    pub(crate) fn do_index_folder(&self, root: &std::path::Path) -> anyhow::Result<IndexResult> {
        let (result, _warnings) = self.do_index_folder_with_warnings(root)?;
        Ok(result)
    }

    /// Index a local folder, returning warnings alongside the index result.
    pub(crate) fn do_index_folder_with_warnings(
        &self,
        root: &std::path::Path,
    ) -> anyhow::Result<(IndexResult, Vec<Value>)> {
        let repo = normalize_repo_id(root);
        let store = IndexStore::open(self.config.repo_db_path(&repo))?;
        let index_version = store.current_index_version()?.saturating_add(1);
        let existing = store.file_fingerprints()?;

        let mut seen_files = HashSet::new();
        let mut touched_files = Vec::new();
        let mut updated_files = Vec::new();
        let mut warnings: Vec<Value> = Vec::new();
        let mut indexed_languages: BTreeSet<String> = BTreeSet::new();
        let extractors = self.extractors_for_root(root);

        // Use the `ignore` crate's WalkBuilder which natively handles nested
        // .gitignore files, root .gitignore, and .git/info/exclude.
        let walker = WalkBuilder::new(root)
            .hidden(false) // don't skip dotfiles by default
            .git_ignore(true) // respect .gitignore (root + nested)
            .git_global(true) // respect global gitignore
            .git_exclude(true) // respect .git/info/exclude
            .filter_entry(|entry| {
                // Still apply our hardcoded skip-dirs on top of gitignore
                if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                    let name = entry.file_name().to_string_lossy();
                    return !crate::db::SKIP_DIRS.iter().any(|skip| *skip == name);
                }
                true
            })
            .build();

        let max_files = self.config.max_folder_files;

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

            // Track the language for missing-import-extractor detection.
            indexed_languages.insert(extractor.language_name().to_string());

            // Enforce max_folder_files if configured
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
            let mns = modified_ns(&metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH));

            let bytes = match fs::read(path) {
                Ok(bytes) => bytes,
                Err(err) => {
                    eprintln!("codesight-mcp: failed to read {}: {err}", path.display());
                    continue;
                }
            };
            let content_hash = blake3::hash(&bytes).to_hex().to_string();
            let source = String::from_utf8_lossy(&bytes).into_owned();

            if let Some(existing_file) = existing.get(&relative_path) {
                if existing_file.size_bytes == size_bytes
                    && existing_file.modified_ns == mns
                    && existing_file.content_hash == content_hash
                {
                    continue;
                }
                if existing_file.content_hash == content_hash {
                    touched_files.push(FileFingerprint {
                        file_path: relative_path,
                        language: extractor.language_name().to_string(),
                        size_bytes,
                        modified_ns: mns,
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
                modified_ns: mns,
                content_hash,
                symbols,
            });
        }

        // Check for missing .gitignore with large file count
        let total_indexed = seen_files.len();
        let has_root_gitignore = root.join(".gitignore").exists();
        if !has_root_gitignore && total_indexed >= GITIGNORE_WARNING_FILE_THRESHOLD {
            warnings.push(json!({
                "code": "NO_GITIGNORE",
                "message": format!(
                    "No .gitignore found at project root and {total_indexed} files were indexed. \
                     Consider adding a .gitignore to exclude generated or vendored files."
                ),
            }));
        }

        let removed_files = existing
            .keys()
            .filter(|path| !seen_files.contains(*path))
            .cloned()
            .collect::<Vec<_>>();

        // Build dependency edges from import extraction.
        let mut file_sources: Vec<(String, String)> = Vec::new();
        for file in &updated_files {
            let full_path = root.join(&file.file_path);
            if let Ok(bytes) = fs::read(&full_path) {
                let source = String::from_utf8_lossy(&bytes).into_owned();
                file_sources.push((file.file_path.clone(), source));
            }
        }
        // Also include unchanged files that are still in the index (for graph completeness)
        for path in &seen_files {
            if !updated_files.iter().any(|f| f.file_path == *path) {
                let full_path = root.join(path);
                if let Ok(bytes) = fs::read(&full_path) {
                    let source = String::from_utf8_lossy(&bytes).into_owned();
                    file_sources.push((path.clone(), source));
                }
            }
        }

        // Load tsconfig/jsconfig path aliases for import resolution
        let alias_map = crate::indexing::imports::load_tsconfig_aliases(&root.to_string_lossy());
        let alias_ref = if alias_map.is_empty() {
            None
        } else {
            Some(&alias_map)
        };
        let adj = dep_graph::build_dependency_graph_with_aliases(&file_sources, alias_ref);
        let dep_edges: Vec<(String, String)> = adj
            .iter()
            .flat_map(|(src, targets)| targets.iter().map(move |tgt| (src.clone(), tgt.clone())))
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

        // Extract and store package names from manifest files
        let package_names = crate::cross_repo::extract_package_names(&root.to_string_lossy());
        if !package_names.is_empty() {
            let _ = store.set_package_names(&package_names);
        }

        let md = store.repo_metadata()?;
        // Compute missing_extractors: languages indexed (have symbol extraction)
        // but missing import/dependency extraction.  BTreeSet is already sorted.
        let missing_extractors: Vec<String> = indexed_languages
            .into_iter()
            .filter(|lang| {
                !LANGUAGES_WITH_IMPORT_EXTRACTION
                    .iter()
                    .any(|supported| supported.eq_ignore_ascii_case(lang))
            })
            .collect();
        Ok((
            IndexResult {
                repo,
                indexed_at,
                file_count: md.file_count,
                symbol_count: md.symbol_count,
                languages: md.languages,
                missing_extractors,
            },
            warnings,
        ))
    }

    pub(super) fn get_file_content(&self, args: FileContentArgs) -> anyhow::Result<Value> {
        let file_path = match args.file_path.or(args.path) {
            Some(p) => p,
            None => {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError {
                        code: "INVALID_ID",
                        message: "Missing file_path parameter.".to_string(),
                        detail: json!({ "source_id": args.source_id }),
                    },
                );
            }
        };
        let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(store) => store,
            Err(error) => return self.error_response(&MetaBuilder::new(0), error),
        };
        let root = store.root_path()?;
        let full_path = root.join(&file_path);
        let raw_bytes = full_path.metadata().map(|m| m.len()).unwrap_or(0);
        let timer = MetaBuilder::new(raw_bytes);
        let content = match fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(_) => {
                return self.error_response(
                    &timer,
                    ToolError {
                        code: "NOT_FOUND",
                        message: format!("File '{file_path}' not found on disk."),
                        detail: json!({ "source_id": args.source_id, "file": file_path }),
                    },
                );
            }
        };
        let language = store
            .file_language(&file_path)?
            .unwrap_or_else(|| "unknown".to_string());
        let (line_count, start_line, end_line, body) =
            slice_file_lines(&content, args.start_line, args.end_line);
        self.success_response(
            &timer,
            json!({
                "source_id": args.source_id,
                "file": file_path,
                "content": body,
                "size_bytes": raw_bytes,
                "language": language,
                "start_line": start_line,
                "end_line": end_line,
                "line_count": line_count,
            }),
        )
    }

    pub(super) fn run_search(
        &self,
        store: &IndexStore,
        query: String,
        scope: Option<String>,
        kind: Option<String>,
        max_results: usize,
    ) -> Result<Vec<SearchSymbol>, ToolError> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let effective_query = if trimmed.len() > MAX_QUERY_LEN {
            &trimmed[..MAX_QUERY_LEN]
        } else {
            trimmed
        };
        store
            .search_symbols(
                effective_query,
                kind.as_deref(),
                scope.as_deref(),
                max_results,
            )
            .map_err(index_error)
    }

    pub(super) fn retrieve_symbol(
        &self,
        store: &IndexStore,
        symbol_id: &str,
        verify: bool,
        context_lines: usize,
    ) -> Result<RetrievedSymbol, ToolError> {
        validate_id(symbol_id)?;
        let record = store
            .symbol_record(symbol_id)
            .map_err(index_error)?
            .ok_or_else(|| ToolError {
                code: "NOT_FOUND",
                message: format!("Symbol '{symbol_id}' not found in index."),
                detail: json!({ "id": symbol_id }),
            })?;

        let file_path = record.root_path.join(&record.file_path);
        let bytes = fs::read(&file_path).map_err(|_| ToolError {
            code: "NOT_FOUND",
            message: format!("File '{}' is missing from disk.", record.file_path),
            detail: json!({ "id": symbol_id, "file": record.file_path }),
        })?;
        let source = String::from_utf8_lossy(&bytes).into_owned();

        let symbol_slice = source
            .get(record.start_byte..record.end_byte)
            .ok_or_else(|| ToolError {
                code: "STALE_ID",
                message: format!("Stored offsets for '{symbol_id}' are stale."),
                detail: json!({ "id": symbol_id }),
            })?;
        let current_hash = blake3::hash(symbol_slice.as_bytes()).to_hex().to_string();
        let source_drift = verify && current_hash != record.source_hash;

        let (line, end_line, output_source) = if context_lines == 0 {
            (record.start_line, record.end_line, symbol_slice.to_string())
        } else {
            expand_context(&source, record.start_line, record.end_line, context_lines)
        };

        Ok(RetrievedSymbol {
            id: record.id,
            kind: record.kind,
            file: record.file_path,
            line,
            end_line,
            source: output_source,
            content_hash: current_hash,
            source_drift: source_drift.then_some(true),
        })
    }

    pub(crate) fn wipe_indexes_for_repo(&self, repo: &str) -> anyhow::Result<()> {
        let repo_dir = self.config.repo_dir(repo);
        if repo_dir.exists() {
            fs::remove_dir_all(&repo_dir)
                .with_context(|| format!("failed to remove code index for '{}'", repo))?;
        }
        let doc_dir = self.config.doc_dir(repo);
        if doc_dir.exists() {
            fs::remove_dir_all(&doc_dir)
                .with_context(|| format!("failed to remove doc index for '{}'", repo))?;
        }
        Ok(())
    }

    pub(crate) fn ensure_repo_store(
        &self,
        repo: &str,
        repo_path: Option<&str>,
    ) -> Result<IndexStore, ToolError> {
        match self.open_repo_store(repo) {
            Ok(store) => {
                if self.config.auto_reindex
                    && let Ok(root) = store.root_path()
                    && root.is_dir()
                {
                    let extractors = self.extractors_for_root(&root);
                    if store
                        .is_stale(&root, |p| extractors.extractor_for_path(p).is_some())
                        .unwrap_or(false)
                    {
                        eprintln!("codesight-mcp: auto-reindexing stale repo '{repo}'");
                        let _ = self.do_index_folder(&root);
                        return self.open_repo_store(repo);
                    }
                }
                Ok(store)
            }
            Err(err) if err.code == "NOT_INDEXED" => {
                // Try source resolution (name-only, owner/name, display_name)
                match self.resolve_source_id(repo) {
                    Ok(resolved) if resolved != repo => {
                        return self.open_repo_store(&resolved);
                    }
                    Err(resolve_err) if resolve_err.code == "AMBIGUOUS_SOURCE" => {
                        return Err(resolve_err);
                    }
                    _ => {}
                }

                if let Some(path_str) = repo_path {
                    let root = canonicalize_root(path_str).map_err(|e| ToolError {
                        code: "INVALID_REPO",
                        message: e.to_string(),
                        detail: json!({ "path": path_str }),
                    })?;
                    eprintln!(
                        "codesight-mcp: auto-indexing '{}' from {}",
                        repo,
                        root.display()
                    );
                    self.do_index_folder(&root).map_err(|e| ToolError {
                        code: "INDEX_ERROR",
                        message: format!("Auto-indexing failed: {e:#}"),
                        detail: json!({ "source_id": repo }),
                    })?;
                    self.open_repo_store(&normalize_repo_id(&root))
                } else {
                    Err(err)
                }
            }
            Err(err) => Err(err),
        }
    }

    pub(super) fn open_repo_store(&self, repo: &str) -> Result<IndexStore, ToolError> {
        let db_path = self.config.repo_db_path(repo);
        IndexStore::open_existing(db_path)
            .map_err(index_error)?
            .ok_or_else(|| ToolError {
                code: "NOT_INDEXED",
                message: format!("Code source '{repo}' has not been indexed."),
                detail: json!({ "source_id": repo }),
            })
    }

    /// Try to resolve a user-supplied source identifier to an indexed
    /// `source_id`. Supports:
    ///  - Exact match (returned as-is)
    ///  - Name-only match (e.g. "hello-world" -> "local/hello-world")
    ///  - owner/name match (e.g. "octocat/hello-world")
    ///  - display_name match
    ///
    /// Returns `Err` with an appropriate error if zero or multiple sources
    /// match (ambiguous).
    pub(crate) fn resolve_source_id(&self, query: &str) -> Result<String, ToolError> {
        // 1. Exact match
        if self.open_repo_store(query).is_ok() {
            return Ok(query.to_string());
        }

        // 2. Enumerate indexed repos and try fuzzy match
        let repos =
            storage::list_repo_summaries(&self.config.repos_dir()).map_err(|e| ToolError {
                code: "INTERNAL",
                message: format!("Failed to list repos: {e}"),
                detail: json!({}),
            })?;

        let query_lower = query.to_lowercase();
        let query_name = query.rsplit('/').next().unwrap_or(query).to_lowercase();

        let mut matches: Vec<String> = Vec::new();
        for m in &repos {
            let sid = &m.repo;
            let sid_lower = sid.to_lowercase();

            // Exact (case-insensitive)
            if sid_lower == query_lower {
                return Ok(sid.clone());
            }
            // Name portion: the part after the last '/'
            let sid_name = sid.rsplit('/').next().unwrap_or(sid).to_lowercase();
            // Direct name match
            if sid_name == query_name {
                matches.push(sid.clone());
                continue;
            }
            // Match with hash suffix stripped: "my-project-{8-char-hash}" matches "my-project"
            if sid_name.len() == query_name.len() + 9
                && sid_name.starts_with(&query_name)
                && sid_name.as_bytes()[query_name.len()] == b'-'
                && sid_name[query_name.len() + 1..]
                    .chars()
                    .all(|c| c.is_ascii_hexdigit())
            {
                matches.push(sid.clone());
            }
        }

        match matches.len() {
            0 => Err(ToolError {
                code: "NOT_INDEXED",
                message: format!("Source '{query}' not found among indexed sources."),
                detail: json!({ "source_id": query }),
            }),
            1 => Ok(matches.into_iter().next().unwrap()),
            _ => Err(ToolError {
                code: "AMBIGUOUS_SOURCE",
                message: format!(
                    "Source identifier '{query}' is ambiguous; matches: {}",
                    matches.join(", ")
                ),
                detail: json!({ "source_id": query, "matches": matches }),
            }),
        }
    }
}
