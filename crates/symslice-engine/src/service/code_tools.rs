use std::fs;

use serde_json::{Value, json};
use symslice_storage::storage::vector_index::OwnedVectorRow;
use tracing::info;

use super::args::*;
use super::helpers::{slice_file_lines, *};
use super::{CodeService, MAX_QUERY_LEN};
use symslice_language_support::imports_orchestration::language_for_path;

use symslice_storage::storage::{
    self, IndexStore, RetrievedSymbol, SearchSymbol, doc_index::DocStore,
};

use symslice_core::pipeline::IndexedFileUpdate;

use crate::{indexing::code_index, repo_id::normalize_repo_id, savings::MetaBuilder};

fn semantic_embed_warning(message: impl Into<String>) -> Value {
    json!({
        "code": "SEMANTIC_EMBED_SKIPPED",
        "message": message.into(),
    })
}

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
        let indexer = self.indexer().clone();
        let mut embed_warning = None;
        let outcome = code_index::index_workspace_with_symbol_vectors(
            &self.config,
            root,
            &indexer,
            |_source_id, updated_files| match self.symbol_vectors_for_updates(updated_files) {
                Ok(rows) => Ok(rows),
                Err(message) => {
                    embed_warning = Some(semantic_embed_warning(message));
                    Ok(Vec::new())
                }
            },
        )?;
        let mut warnings: Vec<Value> = outcome
            .warnings
            .into_iter()
            .map(|warning| {
                json!({
                    "code": warning.code,
                    "message": warning.message,
                })
            })
            .collect();
        if let Some(embed_warning) = embed_warning {
            warnings.push(embed_warning);
        }

        Ok((
            IndexResult {
                repo: outcome.summary.repo,
                indexed_at: outcome.summary.indexed_at,
                file_count: outcome.summary.file_count,
                symbol_count: outcome.summary.symbol_count,
                languages: outcome.summary.languages,
                languages_without_import_extractor: outcome
                    .summary
                    .languages_without_import_extractor,
                files_reindexed: outcome.delta.files_reindexed,
                files_removed: outcome.delta.files_removed,
            },
            warnings,
        ))
    }

    fn symbol_vectors_for_updates(
        &self,
        updated_files: &[IndexedFileUpdate],
    ) -> Result<Vec<OwnedVectorRow>, String> {
        let chunks: Vec<(String, String)> = updated_files
            .iter()
            .flat_map(|file| {
                file.symbols.iter().map(|symbol| {
                    (
                        symbol.id.clone(),
                        symslice_retrieval::embedder::symbol_chunk_text(
                            symbol.kind.as_str(),
                            &symbol.name,
                            &symbol.signature,
                            &symbol.summary,
                            &symbol.source_text,
                        ),
                    )
                })
            })
            .collect();
        self.encode_chunks(chunks)
    }

    pub(super) fn get_file_content(&self, args: FileContentArgs) -> anyhow::Result<Value> {
        let file_path = match args.file_path.or(args.path) {
            Some(p) => p,
            None => {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
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
                        category: ToolErrorCategory::NotFound,
                        code: "NOT_FOUND",
                        message: format!("File '{file_path}' not found on disk."),
                        detail: json!({ "source_id": args.source_id, "file": file_path }),
                    },
                );
            }
        };
        let language = store
            .file_language(&file_path)?
            .filter(|l| l != "unknown")
            .unwrap_or_else(|| language_for_path(&file_path).to_string());
        let requested_start = args.start_line;
        let requested_end = args.end_line;
        let (line_count, start_line, end_line, body) =
            slice_file_lines(&content, args.start_line, args.end_line);
        let end_line_clamped = matches!(requested_end, Some(req) if req > end_line);
        let start_line_clamped = matches!(requested_start, Some(req) if req > start_line);
        let mut warnings: Vec<&'static str> = Vec::new();
        if start_line_clamped {
            warnings.push("start_line_clamped");
        }
        if end_line_clamped {
            warnings.push("end_line_clamped");
        }
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
                "start_line_clamped": start_line_clamped,
                "end_line_clamped": end_line_clamped,
                "line_count": line_count,
                "warnings": warnings,
            }),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn run_search_ex(
        &self,
        store: &IndexStore,
        query: String,
        scope: Option<String>,
        exclude: &[String],
        kind: Option<String>,
        max_results: usize,
        name_only: bool,
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
            .search_symbols_ex(
                effective_query,
                kind.as_deref(),
                scope.as_deref(),
                exclude,
                max_results,
                name_only,
            )
            .map_err(index_error)
    }

    pub(super) fn symbol_body_with_context(
        &self,
        store: &IndexStore,
        symbol_id: &str,
        context_lines: usize,
    ) -> anyhow::Result<Option<String>> {
        let Some(record) = store.symbol_record(symbol_id)? else {
            return Ok(None);
        };
        let file_path = record.root_path.join(&record.file_path);
        let bytes = fs::read(&file_path)?;
        let source = String::from_utf8_lossy(&bytes).into_owned();
        if context_lines == 0 {
            return Ok(source
                .get(record.start_byte..record.end_byte)
                .map(ToOwned::to_owned));
        }
        let (_, _, body) =
            expand_context(&source, record.start_line, record.end_line, context_lines);
        Ok(Some(body))
    }

    pub(super) fn section_body_with_context(
        &self,
        store: &DocStore,
        section_id: &str,
        context_lines: usize,
    ) -> anyhow::Result<Option<String>> {
        let Some(record) = store.section_record(section_id)? else {
            return Ok(None);
        };
        if context_lines == 0 {
            return Ok(Some(record.content_text));
        }
        let file_path = record.root_path.join(&record.file_path);
        let bytes = fs::read(&file_path)?;
        let source = String::from_utf8_lossy(&bytes).into_owned();
        let (_, _, body) =
            expand_context(&source, record.start_line, record.end_line, context_lines);
        Ok(Some(body))
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
                category: ToolErrorCategory::NotFound,
                code: "NOT_FOUND",
                message: format!("Symbol '{symbol_id}' not found in index."),
                detail: json!({ "id": symbol_id }),
            })?;

        // Fast path: use stored source from index when no verification or
        // context expansion is needed. Avoids reading the entire file from disk.
        if !verify && context_lines == 0 {
            let current_hash = blake3::hash(record.stored_source.as_bytes())
                .to_hex()
                .to_string();
            return Ok(RetrievedSymbol {
                id: record.id,
                kind: record.kind,
                file: record.file_path,
                line: record.start_line,
                end_line: record.end_line,
                source: record.stored_source,
                content_hash: current_hash,
                source_drift: None,
            });
        }

        // Slow path: read file from disk for verify/context_lines.
        let file_path = record.root_path.join(&record.file_path);
        let bytes = fs::read(&file_path).map_err(|_| ToolError {
            category: ToolErrorCategory::NotFound,
            code: "NOT_FOUND",
            message: format!("File '{}' is missing from disk.", record.file_path),
            detail: json!({ "id": symbol_id, "file": record.file_path }),
        })?;
        let source = String::from_utf8_lossy(&bytes).into_owned();

        let symbol_slice = source
            .get(record.start_byte..record.end_byte)
            .ok_or_else(|| ToolError {
                category: ToolErrorCategory::InvalidArgument,
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
        // Best-effort WAL checkpoint on every SQLite file under the repo and
        // doc dirs *before* deletion. Without it, a writer mid-checkpoint can
        // create a fresh `-wal`/`-shm` between our `read_dir` and the final
        // `rmdir`, surfacing as `Directory not empty` (Linux ENOTEMPTY).
        let code_db = self.config.repo_db_path(repo);
        if code_db.exists() {
            let _ = symslice_storage::db::wal_checkpoint_passive(&code_db);
        }
        let doc_db = self.config.doc_db_path(repo);
        if doc_db.exists() {
            let _ = symslice_storage::db::wal_checkpoint_passive(&doc_db);
        }
        wipe_dir_with_retry(&self.config.repo_dir(repo), "code")?;
        wipe_dir_with_retry(&self.config.doc_dir(repo), "doc")?;
        Ok(())
    }

    /// Claim the reindex slot and run `do_index_folder`. When the service is
    /// behind an `Arc` (production path), the reindex runs on a detached
    /// background thread and the caller returns its current (stale) store
    /// right away — handlers never block on the reindex. Bare-service test
    /// callers take the synchronous fallback because there is no shared
    /// handle to hand the worker.
    fn kick_off_reindex(
        &self,
        repo: &str,
        repo_id: String,
        root: std::path::PathBuf,
        files_in_flight: Vec<String>,
    ) {
        if self
            .reindex_states
            .try_mark_reindex_start(&repo_id, &files_in_flight)
            .is_err()
        {
            // Another call beat us to it; let its result drive the lifecycle.
            return;
        }

        if let Some(arc) = self.shared_handle() {
            info!(repo = %repo, "spawning background auto-reindex for stale code repo");
            let states = self.reindex_states.clone();
            std::thread::spawn(move || match arc.do_index_folder(&root) {
                Ok(_) => states.mark_reindex_done(&repo_id),
                Err(error) => {
                    tracing::warn!(
                        repo = %repo_id,
                        error = %error,
                        "background auto-reindex failed"
                    );
                    states.mark_reindex_failed(&repo_id, &format!("{error:#}"));
                }
            });
        } else {
            info!(repo = %repo, "auto-reindexing stale code repo (synchronous fallback)");
            match self.do_index_folder(&root) {
                Ok(_) => self.reindex_states.mark_reindex_done(&repo_id),
                Err(error) => {
                    tracing::warn!(
                        repo = %repo_id,
                        error = %error,
                        "synchronous auto-reindex failed"
                    );
                    self.reindex_states
                        .mark_reindex_failed(&repo_id, &format!("{error:#}"));
                }
            }
        }
    }

    pub(crate) fn ensure_repo_store(
        &self,
        repo: &str,
        repo_path: Option<&str>,
    ) -> Result<IndexStore, ToolError> {
        match self.open_repo_store(repo) {
            Ok(store) => {
                if let Ok(root) = store.root_path()
                    && root.is_dir()
                {
                    self.watchers.ensure_watching(
                        repo,
                        root.clone(),
                        self.config.clone(),
                        self.indexer().clone(),
                    );
                    let indexer = self.indexer();
                    let known = |p: &std::path::Path| indexer.detect_language(&root, p).is_some();
                    if store.is_stale(&root, known).unwrap_or(false) {
                        let repo_id = crate::repo_id::normalize_repo_id(&root);
                        let files_in_flight =
                            store.stale_paths(&root, known, 5).unwrap_or_default();
                        self.kick_off_reindex(repo, repo_id, root, files_in_flight);
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
                        category: ToolErrorCategory::InvalidArgument,
                        code: "INVALID_REPO",
                        message: e.to_string(),
                        detail: json!({ "path": path_str }),
                    })?;
                    info!(
                        repo = %repo,
                        root = %root.display(),
                        "auto-indexing code repo from path"
                    );
                    self.do_index_folder(&root).map_err(|e| ToolError {
                        category: ToolErrorCategory::Internal,
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
                category: ToolErrorCategory::InvalidArgument,
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
                category: ToolErrorCategory::InvalidArgument,
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

        match matches.as_slice() {
            [] => Err(ToolError {
                category: ToolErrorCategory::InvalidArgument,
                code: "NOT_INDEXED",
                message: format!("Source '{query}' not found among indexed sources."),
                detail: json!({ "source_id": query }),
            }),
            [single] => Ok(single.clone()),
            _ => Err(ToolError {
                category: ToolErrorCategory::InvalidArgument,
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

/// Recursive remove with up to 5 retries against transient SQLite WAL/SHM
/// races: a writer checkpoint can create a `-wal`/`-shm` file between our
/// `read_dir` and the final `rmdir`, producing a spurious ENOTEMPTY. Each
/// retry sleeps 50 ms before re-attempting.
fn wipe_dir_with_retry(path: &std::path::Path, label: &'static str) -> anyhow::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut last_err = None;
    for _ in 0..5 {
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(_) if !path.exists() => return Ok(()), // raced to empty already.
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
    Err(anyhow::Error::new(last_err.unwrap()).context(format!(
        "failed to remove {label} index at {}",
        path.display()
    )))
}
