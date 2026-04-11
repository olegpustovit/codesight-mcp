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
use crate::storage::IndexStore;
use crate::storage::DocStore;

use semantic::{SemanticMode, apply_semantic};

struct KindFlags {
    want_symbol: bool,
    want_section: bool,
    want_file: bool,
}

impl KindFlags {
    fn from_args(args: &SearchCatalogArgs, domain: &str) -> Self {
        let regex_mode = args.query_mode.as_deref() == Some("regex");
        if regex_mode {
            return Self {
                want_symbol: false,
                want_section: false,
                want_file: true,
            };
        }
        let kinds = args.unit_kinds.as_ref();
        let want_symbol = kinds
            .map(|k| k.iter().any(|x| x == "symbol"))
            .unwrap_or(true)
            && domain != "docs";
        let want_section = kinds
            .map(|k| k.iter().any(|x| x == "section"))
            .unwrap_or(false)
            || domain == "docs"
            || domain == "hybrid";
        let want_file = kinds
            .map(|k| k.iter().any(|x| x == "file"))
            .unwrap_or(false);
        Self {
            want_symbol,
            want_section,
            want_file,
        }
    }
}

fn validate_presearch(args: &SearchCatalogArgs) -> Result<(), ToolError> {
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
            return Err(ToolError {
                category: ToolErrorCategory::InvalidArgument,
                code: "INVALID_REQUEST",
                message: format!(
                    "Invalid sort_by value '{}'. Valid values: {:?}",
                    sort_by, valid
                ),
                detail: json!({ "sort_by": sort_by }),
            });
        }
    }
    let semantic_enabled = args.semantic.unwrap_or(false);
    let semantic_only = args.semantic_only.unwrap_or(false);
    if (semantic_enabled || semantic_only)
        && crate::retrieval::semantic::detect_embedding_provider().is_none()
    {
        return Err(ToolError {
            category: ToolErrorCategory::InvalidArgument,
            code: "no_embedding_provider",
            message: "No embedding provider configured. Set CODESIGHT_EMBED_MODEL, or OPENAI_API_KEY+OPENAI_EMBED_MODEL, or GOOGLE_API_KEY+GOOGLE_EMBED_MODEL.".into(),
            detail: json!({}),
        });
    }
    Ok(())
}

impl CodeService {
    fn resolve_stores(
        &self,
        args: &SearchCatalogArgs,
        domain: &str,
        flags: &KindFlags,
        warnings: &mut Vec<Value>,
    ) -> Result<(Option<IndexStore>, Option<DocStore>), ToolError> {
        let need_code = domain != "docs" && (flags.want_symbol || flags.want_file);
        let need_doc =
            domain != "code" && (flags.want_section || domain == "docs" || domain == "hybrid");

        let code_store = if need_code {
            match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
                Ok(s) => Some(s),
                Err(e) if domain == "hybrid" => {
                    warnings.push(json!({
                        "code": "CODE_INDEX_MISSING",
                        "message": e.message,
                    }));
                    None
                }
                Err(e) => return Err(e),
            }
        } else {
            None
        };

        let doc_store = if need_doc {
            match self.ensure_doc_store(&args.source_id, args.repo_path.as_deref()) {
                Ok(s) => Some(s),
                Err(e) if domain == "hybrid" && code_store.is_some() => {
                    warnings.push(json!({
                        "code": "DOC_INDEX_MISSING",
                        "message": e.message,
                    }));
                    None
                }
                Err(e) if domain == "docs" || domain == "hybrid" => return Err(e),
                Err(_) => None,
            }
        } else {
            None
        };

        if code_store.is_none() && doc_store.is_none() {
            return Err(ToolError {
                category: ToolErrorCategory::InvalidArgument,
                code: "NOT_INDEXED",
                message: format!(
                    "Source '{}' has no usable index for this search.",
                    args.source_id
                ),
                detail: json!({ "source_id": args.source_id, "domain": domain }),
            });
        }
        Ok((code_store, doc_store))
    }

    fn search_code_side(
        &self,
        store: &IndexStore,
        args: &SearchCatalogArgs,
        scope: Option<&String>,
        flags: &KindFlags,
        per_kind_cap: usize,
        items: &mut Vec<Value>,
    ) -> Result<bool, ToolError> {
        let mut more_file_hits = false;
        if flags.want_file {
            let qmode = args.query_mode.as_deref().unwrap_or("keyword");
            let ctx = args.context_lines.unwrap_or(0).min(50);
            let probe_cap = per_kind_cap.saturating_add(1).clamp(1, 50);
            let matches = self
                .search_code_files_text(
                    store,
                    &args.source_id,
                    &args.query,
                    scope.cloned(),
                    probe_cap,
                    qmode,
                    ctx,
                )
                .map_err(index_error)?;
            more_file_hits = matches.len() > per_kind_cap;
            let mut matches = matches;
            matches.truncate(per_kind_cap);
            items.extend(matches);
        }
        if flags.want_symbol || (!flags.want_file && !flags.want_section) {
            let mut symbols = self.run_search(
                store,
                args.query.clone(),
                scope.cloned(),
                args.kind.clone(),
                per_kind_cap,
            )?;
            let fuzzy_explicit = args.fuzzy == Some(true);
            let auto_fuzzy =
                !fuzzy_explicit && symbols.is_empty() && args.fuzzy_threshold.is_some();
            if fuzzy_explicit && !symbols.is_empty() {
                for s in &mut symbols {
                    s.match_type = Some("exact".to_string());
                }
            }
            if fuzzy_explicit || auto_fuzzy {
                fuzzy::augment_with_fuzzy(
                    store,
                    args,
                    scope.map(String::as_str),
                    per_kind_cap,
                    auto_fuzzy,
                    &mut symbols,
                );
            }
            for s in symbols {
                items.push(search_hit_symbol(&args.source_id, &s));
            }
        }
        Ok(more_file_hits)
    }

    fn search_doc_side(
        &self,
        store: &DocStore,
        args: &SearchCatalogArgs,
        scope: Option<&String>,
        per_kind_cap: usize,
        items: &mut Vec<Value>,
    ) -> Result<(), ToolError> {
        let trimmed = args.query.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let effective_query = if trimmed.len() > MAX_QUERY_LEN {
            &trimmed[..MAX_QUERY_LEN]
        } else {
            trimmed
        };
        let sections = store
            .search_sections(effective_query, scope.map(String::as_str), per_kind_cap)
            .map_err(index_error)?;
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
                "score": crate::service::agent_v1::util::round_score(sec.score),
                "line": sec.line,
            }));
        }
        Ok(())
    }

    pub(in crate::service) fn search_catalog(&self, v: Value) -> anyhow::Result<Value> {
        let args: SearchCatalogArgs = ValidatedArgs::from_validated(v)?.into_inner();

        if let Err(e) = validate_presearch(&args) {
            return self.error_response(&MetaBuilder::new(0), e);
        }

        let semantic_enabled = args.semantic.unwrap_or(false);
        let semantic_only = args.semantic_only.unwrap_or(false);
        let search_mode = SemanticMode::resolve(semantic_enabled, semantic_only);

        let scope = args.scope.clone().or(args.file_pattern.clone());
        let lim = args.limit.clamp(1, 50);
        let domain = args
            .domain
            .clone()
            .unwrap_or_else(|| "hybrid".to_string())
            .to_ascii_lowercase();

        let flags = KindFlags::from_args(&args, &domain);

        let mut warnings = Vec::new();
        let (code_store, doc_store) =
            match self.resolve_stores(&args, &domain, &flags, &mut warnings) {
                Ok(pair) => pair,
                Err(e) => return self.error_response(&MetaBuilder::new(0), e),
            };

        let per_kind_cap = if domain == "hybrid" {
            (lim / 2).max(1)
        } else {
            lim
        };

        let mut items: Vec<Value> = Vec::new();
        let mut raw_bytes = 0u64;
        let mut more_file_hits_exist = false;

        if let Some(ref store) = code_store {
            raw_bytes = raw_bytes.max(store.repo_total_bytes()?);
            match self.search_code_side(store, &args, scope.as_ref(), &flags, per_kind_cap, &mut items) {
                Ok(more) => more_file_hits_exist = more,
                Err(e) => return self.error_response(&MetaBuilder::new(raw_bytes.max(1)), e),
            }
        }

        if let Some(ref dstore) = doc_store
            && (flags.want_section || domain == "docs" || domain == "hybrid")
        {
            raw_bytes = raw_bytes.max(dstore.repo_total_bytes()?);
            if let Err(e) =
                self.search_doc_side(dstore, &args, scope.as_ref(), per_kind_cap, &mut items)
            {
                return self.error_response(&MetaBuilder::new(raw_bytes.max(1)), e);
            }
        }

        if (semantic_enabled || semantic_only) && flags.want_symbol {
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
