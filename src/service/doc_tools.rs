use std::{collections::HashSet, fs, time::SystemTime};

use serde_json::{Value, json};
use walkdir::WalkDir;

use super::CodeService;
use super::args::*;
use super::helpers::*;
use crate::{
    doc_storage::{DocFileUpdate, DocStore},
    repo_id::normalize_repo_id,
    savings::MetaBuilder,
    sections::{extract_sections, is_doc_file},
};

impl CodeService {
    pub(crate) fn do_index_local(&self, root: &std::path::Path) -> anyhow::Result<DocIndexResult> {
        let repo = normalize_repo_id(root);
        let store = DocStore::open(self.config.doc_db_path(&repo))?;
        let index_version = store.current_index_version()?.saturating_add(1);
        let existing = store.file_fingerprints()?;

        let mut seen_files = HashSet::new();
        let mut updated_files = Vec::new();

        for entry in WalkDir::new(root)
            .into_iter()
            .filter_entry(should_descend)
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
        {
            let path = entry.path();
            let relative_path = match to_repo_relative(path, root) {
                Ok(r) => r,
                Err(_) => continue,
            };
            if !is_doc_file(&relative_path) {
                continue;
            }
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            seen_files.insert(relative_path.clone());

            let size_bytes = metadata.len();
            let mns = modified_ns(&metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH));
            let bytes = match fs::read(path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let content_hash = blake3::hash(&bytes).to_hex().to_string();

            if let Some(existing_file) = existing.get(&relative_path)
                && existing_file.content_hash == content_hash
            {
                continue;
            }

            let source = String::from_utf8_lossy(&bytes).into_owned();
            let sections = extract_sections(&relative_path, &source);

            let format = if relative_path.ends_with(".md") || relative_path.ends_with(".markdown") {
                "markdown"
            } else if relative_path.ends_with(".rst") {
                "rst"
            } else if relative_path.ends_with(".adoc") || relative_path.ends_with(".asciidoc") {
                "asciidoc"
            } else if relative_path.ends_with(".html") || relative_path.ends_with(".htm") {
                "html"
            } else if relative_path.ends_with(".ipynb") {
                "notebook"
            } else {
                "text"
            };

            updated_files.push(DocFileUpdate {
                file_path: relative_path,
                format: format.to_string(),
                size_bytes,
                modified_ns: mns,
                content_hash,
                sections,
                source,
            });
        }

        let removed_files = existing
            .keys()
            .filter(|path| !seen_files.contains(*path))
            .cloned()
            .collect::<Vec<_>>();

        let indexed_at = chrono::Utc::now().to_rfc3339();
        let section_count: usize = updated_files.iter().map(|f| f.sections.len()).sum();
        let file_count = seen_files.len();
        store.apply_index(
            &repo,
            root,
            &indexed_at,
            index_version,
            &updated_files,
            &removed_files,
        )?;

        Ok(DocIndexResult {
            repo,
            indexed_at,
            file_count,
            section_count,
        })
    }

    pub(super) fn get_section(&self, args: GetSectionArgs) -> anyhow::Result<Value> {
        let Some(section_id) = args.section_id.or(args.id) else {
            return self.error_response(
                &MetaBuilder::new(0),
                ToolError {
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
        let result = crate::doc_storage::RetrievedSection {
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
                if self.config.auto_reindex
                    && let Ok(root) = store.root_path()
                    && root.is_dir()
                    && store.is_stale(&root).unwrap_or(false)
                {
                    eprintln!("codesight-mcp: auto-reindexing stale doc source '{repo}'");
                    let _ = self.do_index_local(&root);
                    return self.open_doc_store(repo);
                }
                Ok(store)
            }
            Err(err) if err.code == "NOT_INDEXED" => {
                if let Some(path_str) = repo_path {
                    let root = canonicalize_root(path_str).map_err(|e| ToolError {
                        code: "INVALID_REPO",
                        message: e.to_string(),
                        detail: json!({ "path": path_str }),
                    })?;
                    eprintln!(
                        "codesight-mcp: auto-indexing docs source '{}' from {}",
                        repo,
                        root.display()
                    );
                    self.do_index_local(&root).map_err(|e| ToolError {
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
                code: "NOT_INDEXED",
                message: format!("Doc source '{repo}' has not been indexed."),
                detail: json!({ "source_id": repo }),
            })
    }
}
