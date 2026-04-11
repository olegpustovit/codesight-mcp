//! `search_catalog` — BM25 symbols/files/sections with optional fuzzy + semantic layers.

mod fuzzy;
mod semantic;

use serde_json::{Value, json};

use crate::service::agent_v1::args::SearchCatalogArgs;
use crate::service::agent_v1::util::*;
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory, index_error};
use crate::service::savings::MetaBuilder;
use crate::service::{CodeService, MAX_QUERY_LEN};

use semantic::{SemanticMode, apply_semantic};

impl CodeService {
    pub(in crate::service) fn search_catalog(&self, v: Value) -> anyhow::Result<Value> {
        let args: SearchCatalogArgs = ValidatedArgs::from_validated(v)?.into_inner();

        if let Some(ref sort_by) = args.sort_by {
            let valid = [
                "relevance",
                "centrality",
                "name",
                "file",
                "line",
                "combined",
            ];
            if !valid.contains(&sort_by.as_str()) {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
                        code: "INVALID_REQUEST",
                        message: format!(
                            "Invalid sort_by value '{}'. Valid values: {:?}",
                            sort_by, valid
                        ),
                        detail: json!({ "sort_by": sort_by }),
                    },
                );
            }
        }

        let semantic_enabled = args.semantic.unwrap_or(false);
        let semantic_only = args.semantic_only.unwrap_or(false);
        let search_mode = SemanticMode::resolve(semantic_enabled, semantic_only);

        if semantic_enabled || semantic_only {
            let provider = crate::retrieval::semantic::detect_embedding_provider();
            if provider.is_none() {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
                        code: "no_embedding_provider",
                        message: "No embedding provider configured. Set CODESIGHT_EMBED_MODEL, or OPENAI_API_KEY+OPENAI_EMBED_MODEL, or GOOGLE_API_KEY+GOOGLE_EMBED_MODEL.".into(),
                        detail: json!({}),
                    },
                );
            }
        }

        let scope = args.scope.clone().or(args.file_pattern.clone());
        let lim = args.limit.clamp(1, 50);
        let domain = args
            .domain
            .clone()
            .unwrap_or_else(|| "hybrid".to_string())
            .to_ascii_lowercase();
        let kinds = args.unit_kinds.clone();

        let mut items: Vec<Value> = Vec::new();
        let mut warnings = Vec::new();

        // regex mode is only implemented over file text; auto-route it so the
        // symbol/section branches don't silently drop the query on the floor.
        let regex_mode = args.query_mode.as_deref() == Some("regex");
        let want_symbol = if regex_mode {
            false
        } else {
            kinds
                .as_ref()
                .map(|k| k.iter().any(|x| x == "symbol"))
                .unwrap_or(true)
                && domain != "docs"
        };
        let want_section = if regex_mode {
            false
        } else {
            kinds
                .as_ref()
                .map(|k| k.iter().any(|x| x == "section"))
                .unwrap_or(false)
                || domain == "docs"
                || domain == "hybrid"
        };
        let want_file = if regex_mode {
            true
        } else {
            kinds
                .as_ref()
                .map(|k| k.iter().any(|x| x == "file"))
                .unwrap_or(false)
        };

        let need_code_index = domain != "docs" && (want_symbol || want_file);
        let need_doc_index =
            domain != "code" && (want_section || domain == "docs" || domain == "hybrid");

        let code_store = if need_code_index {
            match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
                Ok(s) => Some(s),
                Err(e) if domain == "hybrid" => {
                    warnings.push(json!({
                        "code": "CODE_INDEX_MISSING",
                        "message": e.message,
                    }));
                    None
                }
                Err(e) => return self.error_response(&MetaBuilder::new(0), e),
            }
        } else {
            None
        };

        let doc_store = if need_doc_index {
            match self.ensure_doc_store(&args.source_id, args.repo_path.as_deref()) {
                Ok(s) => Some(s),
                Err(e) if domain == "hybrid" && code_store.is_some() => {
                    warnings.push(json!({
                        "code": "DOC_INDEX_MISSING",
                        "message": e.message,
                    }));
                    None
                }
                Err(e) if domain == "docs" || domain == "hybrid" => {
                    return self.error_response(&MetaBuilder::new(0), e);
                }
                Err(_) => None,
            }
        } else {
            None
        };

        if code_store.is_none() && doc_store.is_none() {
            return self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "NOT_INDEXED",
                    message: format!(
                        "Source '{}' has no usable index for this search.",
                        args.source_id
                    ),
                    detail: json!({ "source_id": args.source_id, "domain": domain }),
                },
            );
        }

        let per_kind_cap = if domain == "hybrid" {
            (lim / 2).max(1)
        } else {
            lim
        };

        let mut raw_bytes = 0u64;
        let mut more_file_hits_exist = false;

        if let Some(ref store) = code_store {
            raw_bytes = raw_bytes.max(store.repo_total_bytes()?);
            if want_file {
                let qmode = args.query_mode.as_deref().unwrap_or("keyword");
                let ctx = args.context_lines.unwrap_or(0).min(50);
                let probe_cap = per_kind_cap.saturating_add(1).clamp(1, 50);
                let matches = self.search_code_files_text(
                    store,
                    &args.source_id,
                    &args.query,
                    scope.clone(),
                    probe_cap,
                    qmode,
                    ctx,
                )?;
                more_file_hits_exist |= matches.len() > per_kind_cap;
                let mut matches = matches;
                matches.truncate(per_kind_cap);
                items.extend(matches);
            }
            if want_symbol || (!want_file && !want_section) {
                let mut symbols = match self.run_search(
                    store,
                    args.query.clone(),
                    scope.clone(),
                    args.kind.clone(),
                    per_kind_cap,
                ) {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&MetaBuilder::new(raw_bytes.max(1)), e),
                };
                let fuzzy_explicit = args.fuzzy == Some(true);
                let auto_fuzzy = args.fuzzy != Some(true)
                    && symbols.is_empty()
                    && args.fuzzy_threshold.is_some();
                let run_fuzzy = fuzzy_explicit || auto_fuzzy;
                if fuzzy_explicit && !symbols.is_empty() {
                    for s in &mut symbols {
                        s.match_type = Some("exact".to_string());
                    }
                }
                if run_fuzzy {
                    fuzzy::augment_with_fuzzy(
                        store,
                        &args,
                        scope.as_deref(),
                        per_kind_cap,
                        auto_fuzzy,
                        &mut symbols,
                    );
                }
                for s in symbols {
                    items.push(search_hit_symbol(&args.source_id, &s));
                }
            }
        }

        if let Some(ref dstore) = doc_store
            && (want_section || domain == "docs" || domain == "hybrid")
        {
            raw_bytes = raw_bytes.max(dstore.repo_total_bytes()?);
            let trimmed = args.query.trim();
            if !trimmed.is_empty() {
                let effective_query = if trimmed.len() > MAX_QUERY_LEN {
                    &trimmed[..MAX_QUERY_LEN]
                } else {
                    trimmed
                };
                let sections = match dstore.search_sections(
                    effective_query,
                    scope.as_deref(),
                    per_kind_cap,
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        return self.error_response(
                            &MetaBuilder::new(raw_bytes.max(1)),
                            index_error(e),
                        );
                    }
                };
                for sec in sections {
                    items.push(json!({
                        "ref": {
                            "id": sec.id,
                            "stable_id": sec.id,
                            "source_id": args.source_id,
                            "domain": "docs",
                            "kind": "section",
                            "path": sec.file,
                            "title": sec.heading,
                            "summary": sec.summary,
                        },
                        "snippet": sec.heading_path,
                        "score": sec.score,
                        "line": sec.line,
                    }));
                }
            }
        }

        if (semantic_enabled || semantic_only) && want_symbol {
            apply_semantic(self, &args, lim, &search_mode, &mut items);
        }

        let mut budget_truncated = false;
        if let Some(mt) = args.max_tokens {
            let (packed, bt) = pack_catalog_items_to_token_budget(items, mt);
            items = packed;
            budget_truncated = bt;
        }

        let mut limit_truncated = items.len() > lim || more_file_hits_exist;
        if items.len() > lim {
            warnings.push(json!({
                "code": "TRUNCATED",
                "message": format!("results truncated to limit {lim}"),
            }));
            items.truncate(lim);
            limit_truncated = true;
        }

        let meta_truncated = budget_truncated || limit_truncated;

        let timer = MetaBuilder::new(raw_bytes.max(1));
        let mut data = json!({ "items": items });
        if limit_truncated && let Value::Object(ref mut map) = data {
            map.insert("truncated".to_string(), json!(true));
        }
        let mut extra: Vec<(String, Value)> =
            vec![("truncated".to_string(), json!(meta_truncated))];
        if semantic_enabled || semantic_only {
            extra.push(("search_mode".to_string(), json!(search_mode.as_str())));
        }
        self.success_response_with_meta_extra(&timer, data, warnings, extra)
    }
}
