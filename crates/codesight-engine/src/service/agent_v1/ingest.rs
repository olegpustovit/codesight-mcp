//! Ingest paths into code/docs indexes.

use std::path::Path;

use serde_json::{Value, json};

use crate::repo_id::normalize_repo_id;
use crate::service::CodeService;
use crate::service::agent_v1::args::IngestSourcesArgs;
use crate::service::agent_v1::util::doc_format_to_type_label;
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory, canonicalize_root};
use crate::service::savings::MetaBuilder;
use codesight_storage::storage::doc_index::DocStore;

impl CodeService {
    pub(in crate::service) fn ingest_sources(&self, v: Value) -> anyhow::Result<Value> {
        let args: IngestSourcesArgs = ValidatedArgs::from_validated(v)?.into_inner();
        let timer = MetaBuilder::new(0);
        let mut warnings = Vec::new();

        if !args.enrichment.is_empty() {
            warnings.push(json!({
                "code": "UNSUPPORTED_ENRICHMENT",
                "message": "enrichment is not implemented; ignoring.",
            }));
        }

        let path_for_index = if args.input_kind == "single_file" {
            let p = Path::new(&args.path);
            let abs = match p.canonicalize() {
                Ok(x) => x,
                Err(e) => {
                    return self.error_response(
                        &timer,
                        ToolError {
                            category: ToolErrorCategory::InvalidArgument,
                            code: "INVALID_PATH",
                            message: format!("single_file path invalid: {e}"),
                            detail: json!({ "path": args.path }),
                        },
                    );
                }
            };
            if !abs.is_file() {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
                        code: "INVALID_PATH",
                        message: "single_file path must be an existing file.".to_string(),
                        detail: json!({ "path": args.path }),
                    },
                );
            }
            let Some(parent) = abs.parent() else {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
                        code: "INVALID_PATH",
                        message: "single_file path must have a parent directory.".to_string(),
                        detail: json!({ "path": args.path }),
                    },
                );
            };
            warnings.push(json!({
                "code": "SINGLE_FILE_SCOPE",
                "message": "single_file reindexes the parent directory with the standard crawl.",
            }));
            parent.to_path_buf()
        } else if args.input_kind == "git_repo" {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "UNSUPPORTED",
                    message: "input_kind git_repo is not implemented.".to_string(),
                    detail: json!({ "input_kind": args.input_kind }),
                },
            );
        } else {
            match canonicalize_root(&args.path) {
                Ok(p) => p,
                Err(e) => {
                    return self.error_response(
                        &timer,
                        ToolError {
                            category: ToolErrorCategory::InvalidArgument,
                            code: "INVALID_REPO",
                            message: e.to_string(),
                            detail: json!({ "path": args.path }),
                        },
                    );
                }
            }
        };

        // ── Trust checking ──
        match crate::security::check_trust(&self.config, &path_for_index) {
            crate::security::TrustCheckResult::Trusted => {}
            crate::security::TrustCheckResult::TrustedBroadRoot { warning } => {
                warnings.push(json!({
                    "code": "BROAD_PATH_TRUSTED",
                    "message": warning,
                }));
            }
            crate::security::TrustCheckResult::Untrusted { error } => {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
                        code: "TRUST_ERROR",
                        message: error,
                        detail: json!({ "path": args.path }),
                    },
                );
            }
        }

        // Freshness lifecycle: the actual index-mutation work begins below.
        // Early returns before this point are validation failures and must not
        // flip the repo into `reindexing=true`.
        let repo_id_for_state = normalize_repo_id(&path_for_index);
        if let Err(busy) = self
            .reindex_states()
            .try_mark_reindex_start(&repo_id_for_state, &[])
        {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::Transient,
                    code: "REINDEX_BUSY",
                    message: busy.to_string(),
                    detail: json!({ "source_id": busy.repo }),
                },
            );
        }
        let mut reindex_guard = ReindexGuard::new(self, repo_id_for_state.clone());

        if !args.incremental {
            let repo = normalize_repo_id(&path_for_index);
            self.wipe_indexes_for_repo(&repo)?;
            warnings.push(json!({
                "code": "FULL_REINDEX",
                "message": "incremental=false removed existing indexes for this source before reingest.",
            }));
        }

        let dom = args.domains.to_ascii_lowercase();
        let mut last_code: Option<crate::service::args::IndexResult> = None;
        let mut last_doc: Option<crate::service::args::DocIndexResult> = None;

        if dom == "code" || dom == "auto" {
            let (result, index_warnings) = self.do_index_folder_with_warnings(&path_for_index)?;
            warnings.extend(index_warnings);
            last_code = Some(result);
        }
        if dom == "docs" || dom == "auto" {
            last_doc = Some(self.do_index_local(&path_for_index)?);
        }

        let indexed_docs = last_doc.is_some();
        let source_id = normalize_repo_id(&path_for_index);
        let mut data = match (last_code, last_doc) {
            (Some(c), Some(d)) => {
                let mut data = json!({
                    "source_id": source_id,
                    "indexed_at_code": c.indexed_at,
                    "indexed_at_docs": d.indexed_at,
                    "file_count_code": c.file_count,
                    "symbol_count": c.symbol_count,
                    "file_count_docs": d.file_count,
                    "section_count": d.section_count,
                    "languages": c.languages,
                });
                if !c.languages_without_import_extractor.is_empty() {
                    data["languages_without_import_extractor"] =
                        json!(c.languages_without_import_extractor);
                }
                data["delta"] = json!({
                    "files_reindexed": c.files_reindexed,
                    "files_removed": c.files_removed,
                });
                data
            }
            (Some(c), None) => {
                let mut d = json!({
                    "source_id": c.repo,
                    "indexed_at": c.indexed_at,
                    "file_count": c.file_count,
                    "symbol_count": c.symbol_count,
                    "languages": c.languages,
                });
                if !c.languages_without_import_extractor.is_empty() {
                    d["languages_without_import_extractor"] =
                        json!(c.languages_without_import_extractor);
                }
                d["delta"] = json!({
                    "files_reindexed": c.files_reindexed,
                    "files_removed": c.files_removed,
                });
                d
            }
            (None, Some(d)) => {
                json!({
                    "source_id": d.repo,
                    "indexed_at": d.indexed_at,
                    "file_count": d.file_count,
                    "section_count": d.section_count,
                })
            }
            (None, None) => {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
                        code: "INVALID_REQUEST",
                        message: "domains must be code, docs, or auto.".to_string(),
                        detail: json!({ "domains": args.domains }),
                    },
                );
            }
        };

        if indexed_docs
            && let Value::Object(ref mut map) = data
            && let Ok(store) = DocStore::open(self.config.doc_db_path(&source_id))
            && let Ok(fmts) = store.distinct_doc_formats()
        {
            let doc_types: Vec<String> = fmts.iter().map(|f| doc_format_to_type_label(f)).collect();
            map.insert("doc_types".to_string(), json!(doc_types));
        }

        reindex_guard.mark_done();

        // The on-disk fingerprint (`repo_info.indexed_at`) just moved, so the
        // next cross-source reader should refresh. `snapshot()` would catch
        // the drift on its own, but explicit invalidation skips the extra
        // SELECT on the next hit.
        self.source_registry().invalidate(&source_id);

        self.watchers.ensure_watching(
            &source_id,
            path_for_index,
            self.config.clone(),
            self.indexer().clone(),
        );

        self.success_response_with_warnings(&timer, data, warnings)
    }
}

/// RAII guard for `ReindexStateManager` transitions during ingest.
///
/// Defaults to marking the reindex as *failed* on drop so that any early
/// return — including `?` error propagation — flips the repo into a visible
/// failure state. The success path must call [`ReindexGuard::mark_done`]
/// before returning.
struct ReindexGuard<'a> {
    svc: &'a CodeService,
    repo: String,
    done: bool,
}

impl<'a> ReindexGuard<'a> {
    fn new(svc: &'a CodeService, repo: String) -> Self {
        Self {
            svc,
            repo,
            done: false,
        }
    }

    fn mark_done(&mut self) {
        self.svc.reindex_states().mark_reindex_done(&self.repo);
        self.done = true;
    }
}

impl Drop for ReindexGuard<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.svc
                .reindex_states()
                .mark_reindex_failed(&self.repo, "ingest aborted");
            // Abnormal drop: passive-checkpoint the repo databases so an
            // aborted ingest doesn't leave a fat WAL file behind. SQLite WAL
            // is already crash-safe; this is cleanup, not correctness.
            let code_db = self.svc.config.repo_db_path(&self.repo);
            if let Err(error) = codesight_storage::db::wal_checkpoint_passive(&code_db) {
                tracing::debug!(
                    db = %code_db.display(),
                    error = %error,
                    "reindex abort wal checkpoint failed"
                );
            }
            let doc_db = self.svc.config.doc_db_path(&self.repo);
            if let Err(error) = codesight_storage::db::wal_checkpoint_passive(&doc_db) {
                tracing::debug!(
                    db = %doc_db.display(),
                    error = %error,
                    "reindex abort doc wal checkpoint failed"
                );
            }
        }
    }
}
