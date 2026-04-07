//! Ingest paths into code/docs indexes.

use std::path::Path;

use serde_json::{Value, json};

use crate::doc_storage::DocStore;
use crate::doc_storage::list_doc_summaries;
use crate::path_map;
use crate::repo_id::normalize_repo_id;
use crate::savings::MetaBuilder;
use crate::service::CodeService;
use crate::service::agent_v1::args::IngestSourcesArgs;
use crate::service::agent_v1::util::doc_format_to_type_label;
use crate::service::args::parse_args;
use crate::service::helpers::{ToolError, canonicalize_root};

impl CodeService {
    pub(in crate::service) fn ingest_sources(&self, v: Value) -> anyhow::Result<Value> {
        let args: IngestSourcesArgs = parse_args(v)?;
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
                            code: "INVALID_REPO",
                            message: e.to_string(),
                            detail: json!({ "path": args.path }),
                        },
                    );
                }
            }
        };

        // ── Trust checking ──
        match crate::config::check_trust(&self.config, &path_for_index) {
            crate::config::TrustCheckResult::Trusted => {}
            crate::config::TrustCheckResult::TrustedBroadRoot { warning } => {
                warnings.push(json!({
                    "code": "BROAD_PATH_TRUSTED",
                    "message": warning,
                }));
            }
            crate::config::TrustCheckResult::Untrusted { error } => {
                return self.error_response(
                    &timer,
                    ToolError {
                        code: "TRUST_ERROR",
                        message: error,
                        detail: json!({ "path": args.path }),
                    },
                );
            }
        }

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
                if !c.missing_extractors.is_empty() {
                    data["missing_extractors"] = json!(c.missing_extractors);
                    let pw: Vec<String> = c
                        .missing_extractors
                        .iter()
                        .map(|lang| {
                            format!(
                                "Language '{}' has symbol extraction but no import extractor",
                                lang
                            )
                        })
                        .collect();
                    data["parse_warnings"] = json!(pw);
                }
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
                if !c.missing_extractors.is_empty() {
                    d["missing_extractors"] = json!(c.missing_extractors);
                    let pw: Vec<String> = c
                        .missing_extractors
                        .iter()
                        .map(|lang| {
                            format!(
                                "Language '{}' has symbol extraction but no import extractor",
                                lang
                            )
                        })
                        .collect();
                    d["parse_warnings"] = json!(pw);
                }
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
                        code: "INVALID_REQUEST",
                        message: "domains must be code, docs, or auto.".to_string(),
                        detail: json!({ "domains": args.domains }),
                    },
                );
            }
        };

        if indexed_docs {
            if let Value::Object(ref mut map) = data {
                if let Ok(store) = DocStore::open(self.config.doc_db_path(&source_id)) {
                    if let Ok(fmts) = store.distinct_doc_formats() {
                        let doc_types: Vec<String> =
                            fmts.iter().map(|f| doc_format_to_type_label(f)).collect();
                        map.insert("doc_types".to_string(), json!(doc_types));
                    }
                }
            }
        }

        self.success_response_with_warnings(&timer, data, warnings)
    }
}
