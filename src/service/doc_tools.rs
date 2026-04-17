use std::fs;

use codesight_storage::storage::vector_index::OwnedVectorRow;
use tracing::info;

use super::CodeService;
use super::args::*;
use super::helpers::*;
use crate::{
    repo_id::normalize_repo_id,
    savings::MetaBuilder,
    storage::{
        DocFileUpdate,
        doc_index::{self, DocStore},
    },
};
use serde_json::{Value, json};

impl CodeService {
    pub(crate) fn do_index_local(&self, root: &std::path::Path) -> anyhow::Result<DocIndexResult> {
        let mut embed_warning = None;
        let summary = doc_index::index_local_docs_with_section_vectors(
            self.config.doc_db_path(&normalize_repo_id(root)),
            root,
            |_source_id, updated_files| match self.section_vectors_for_updates(updated_files) {
                Ok(rows) => Ok(rows),
                Err(message) => {
                    embed_warning = Some(message);
                    Ok(Vec::new())
                }
            },
        )?;
        if let Some(message) = embed_warning {
            tracing::warn!(repo = %summary.repo, error = %message, "section embedding skipped");
        }
        Ok(DocIndexResult {
            repo: summary.repo,
            indexed_at: summary.indexed_at,
            file_count: summary.file_count,
            section_count: summary.section_count,
        })
    }

    fn section_vectors_for_updates(
        &self,
        updated_files: &[DocFileUpdate],
    ) -> Result<Vec<OwnedVectorRow>, String> {
        let chunks: Vec<(String, String)> = updated_files
            .iter()
            .flat_map(|file| {
                file.sections.iter().map(|section| {
                    let content = file
                        .source
                        .get(section.start_byte..section.end_byte)
                        .unwrap_or("");
                    (
                        section.id.clone(),
                        codesight_retrieval::embedder::section_chunk_text(
                            &section.heading,
                            &section.heading_path,
                            &section.summary,
                            content,
                        ),
                    )
                })
            })
            .collect();
        self.encode_chunks(chunks)
    }

    pub(super) fn get_section(&self, args: GetSectionArgs) -> anyhow::Result<Value> {
        let Some(section_id) = args.section_id.or(args.id) else {
            return self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_ID",
                    message: "Missing section identifier.".to_string(),
                    detail: json!({ "source_id": args.source_id }),
                },
            );
        };
        let store = match self.ensure_doc_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(store) => store,
            Err(error) => return self.error_response(&MetaBuilder::new(0), error),
        };
        let raw_bytes = store.section_file_size(&section_id)?.unwrap_or(0);
        let timer = MetaBuilder::new(raw_bytes);

        let record = match store.section_record(&section_id) {
            Ok(Some(r)) => r,
            Ok(None) => {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::NotFound,
                        code: "NOT_FOUND",
                        message: format!("Section '{section_id}' not found in index."),
                        detail: json!({ "id": section_id }),
                    },
                );
            }
            Err(e) => return self.error_response(&timer, index_error(e)),
        };

        let file_path = record.root_path.join(&record.file_path);
        let content = match fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(_) => {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::NotFound,
                        code: "NOT_FOUND",
                        message: format!("File '{}' is missing from disk.", record.file_path),
                        detail: json!({ "id": section_id }),
                    },
                );
            }
        };

        let section_text = content
            .get(record.start_byte..record.end_byte)
            .unwrap_or(&record.content_text);
        let current_hash = blake3::hash(section_text.as_bytes()).to_hex().to_string();
        let source_drift = args.verify && current_hash != record.content_hash;

        let hash_verified = args.verify.then(|| current_hash == record.content_hash);
        let result = doc_index::RetrievedSection {
            id: record.id,
            heading: record.heading,
            file: record.file_path,
            level: record.level,
            line: record.start_line,
            end_line: record.end_line,
            content: section_text.to_string(),
            content_hash: current_hash,
            source_drift: source_drift.then_some(true),
            hash_verified,
            content_truncated: None,
        };
        self.success_response(&timer, serde_json::to_value(result)?)
    }

    pub(crate) fn ensure_doc_store(
        &self,
        repo: &str,
        repo_path: Option<&str>,
    ) -> Result<DocStore, ToolError> {
        match self.open_doc_store(repo) {
            Ok(store) => {
                if let Ok(root) = store.root_path()
                    && root.is_dir()
                    && store.is_stale(&root).unwrap_or(false)
                {
                    info!(repo = %repo, "auto-reindexing stale doc source");
                    let _ = self.do_index_local(&root);
                    return self.open_doc_store(repo);
                }
                Ok(store)
            }
            Err(err) if err.code == "NOT_INDEXED" => {
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
                        "auto-indexing docs source from path"
                    );
                    self.do_index_local(&root).map_err(|e| ToolError {
                        category: ToolErrorCategory::Internal,
                        code: "INDEX_ERROR",
                        message: format!("Auto-indexing failed: {e:#}"),
                        detail: json!({ "source_id": repo }),
                    })?;
                    self.open_doc_store(&normalize_repo_id(&root))
                } else {
                    Err(err)
                }
            }
            Err(err) => Err(err),
        }
    }

    fn open_doc_store(&self, repo: &str) -> Result<DocStore, ToolError> {
        let db_path = self.config.doc_db_path(repo);
        DocStore::open_existing(db_path)
            .map_err(index_error)?
            .ok_or_else(|| ToolError {
                category: ToolErrorCategory::InvalidArgument,
                code: "NOT_INDEXED",
                message: format!("Doc source '{repo}' has not been indexed."),
                detail: json!({ "source_id": repo }),
            })
    }
}
