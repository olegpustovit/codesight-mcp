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
use crate::storage::DocStore;
use crate::storage::IndexStore;

use semantic::run_dense_lane;

/// Fuse the already-ranked lexical items with the dense lane's id lists
/// via RRF. Symbol and section lanes fuse separately; file-text hits
/// (which the dense lane never produces) pass through unchanged.
fn fuse_items_with_dense(
    items: Vec<Value>,
    dense_symbol_ids: &[String],
    dense_section_ids: &[String],
) -> Vec<Value> {
    // Partition: symbols, sections, others (file hits). Preserve input
    // order inside each partition so RRF sees the lexical ranking.
    let mut lex_symbols: Vec<String> = Vec::new();
    let mut lex_sections: Vec<String> = Vec::new();
    let mut by_id: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    let mut others: Vec<Value> = Vec::new();
    for item in items {
        let id = item
            .get("ref")
            .and_then(|r| r.get("id"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let kind = item
            .get("ref")
            .and_then(|r| r.get("kind"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        match (id, kind) {
            (Some(i), "symbol") => {
                lex_symbols.push(i.clone());
                by_id.insert(i, item);
            }
            (Some(i), "section") => {
                lex_sections.push(i.clone());
                by_id.insert(i, item);
            }
            _ => others.push(item),
        }
    }

    let fused_symbols =
        crate::retrieval::fusion::rrf(lex_symbols, dense_symbol_ids.iter().cloned());
    let fused_sections =
        crate::retrieval::fusion::rrf(lex_sections, dense_section_ids.iter().cloned());

    let mut out: Vec<Value> = Vec::with_capacity(others.len() + by_id.len());
    out.extend(others);
    for (id, _score) in fused_symbols {
        if let Some(item) = by_id.remove(&id) {
            out.push(item);
        }
    }
    for (id, _score) in fused_sections {
        if let Some(item) = by_id.remove(&id) {
            out.push(item);
        }
    }
    // Any items the dense lane surfaced but the lexical lane didn't
    // produce are dropped — we don't yet materialize records from
    // bare IDs in this path. Follow-up work.
    out
}

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
    Ok(())
}

/// Coerce `pattern` alias into `query` and validate `output_mode`.
fn coerce_search_args(args: &mut SearchCatalogArgs) -> Result<String, ToolError> {
    if let Some(pattern) = args.pattern.take() {
        if !args.query.is_empty() {
            return Err(ToolError {
                category: ToolErrorCategory::InvalidArgument,
                code: "INVALID_REQUEST",
                message: "Cannot set both `query` and `pattern`. Use one or the other.".into(),
                detail: json!({}),
            });
        }
        args.query = pattern;
    }
    let output_mode = args
        .output_mode
        .as_deref()
        .unwrap_or("items")
        .to_ascii_lowercase();
    if !matches!(
        output_mode.as_str(),
        "items" | "count" | "files_with_matches"
    ) {
        return Err(ToolError {
            category: ToolErrorCategory::InvalidArgument,
            code: "INVALID_REQUEST",
            message: format!(
                "Invalid output_mode '{}'. Valid: items, count, files_with_matches.",
                output_mode
            ),
            detail: json!({ "output_mode": output_mode }),
        });
    }
    Ok(output_mode)
}

struct FormatCtx<'a> {
    output_mode: &'a str,
    query: &'a str,
    query_mode_effective: &'a str,
    lim: usize,
    max_tokens: Option<u32>,
    more_file_hits_exist: bool,
}

/// Format collected items for count / files_with_matches / items modes.
fn format_output(
    svc: &CodeService,
    ctx: &FormatCtx<'_>,
    mut items: Vec<Value>,
    timer: &MetaBuilder,
    mut warnings: Vec<Value>,
    mut extra: Vec<(String, Value)>,
) -> anyhow::Result<Value> {
    match ctx.output_mode {
        "count" => {
            let total_count = items.len();
            let file_set: std::collections::HashSet<&str> = items
                .iter()
                .filter_map(|item| item.pointer("/ref/path")?.as_str())
                .collect();
            let data = json!({ "count": total_count, "files": file_set.len() });
            extra.push(("truncated".to_string(), json!(false)));
            svc.success_response_with_meta_extra(timer, data, warnings, extra)
        }
        "files_with_matches" => {
            let mut file_counts: std::collections::BTreeMap<String, usize> =
                std::collections::BTreeMap::new();
            for item in &items {
                if let Some(path) = item.pointer("/ref/path").and_then(|v| v.as_str()) {
                    *file_counts.entry(path.to_string()).or_default() += 1;
                }
            }
            let file_items: Vec<Value> = file_counts
                .into_iter()
                .map(|(file, count)| json!({ "file": file, "count": count }))
                .collect();
            let data = json!({ "files": file_items });
            extra.push(("truncated".to_string(), json!(false)));
            svc.success_response_with_meta_extra(timer, data, warnings, extra)
        }
        _ => {
            let mut budget_truncated = false;
            if let Some(mt) = ctx.max_tokens {
                let (packed, bt) = pack_catalog_items_to_token_budget(items, mt);
                items = packed;
                budget_truncated = bt;
            }

            let lim = ctx.lim;
            let mut limit_truncated = items.len() > lim || ctx.more_file_hits_exist;
            if items.len() > lim {
                warnings.push(json!({
                    "code": "TRUNCATED",
                    "message": format!("results truncated to limit {lim}"),
                }));
                items.truncate(lim);
                limit_truncated = true;
            }

            let meta_truncated = budget_truncated || limit_truncated;
            let mut data = json!({ "items": items });
            if limit_truncated && let Value::Object(ref mut map) = data {
                map.insert("truncated".to_string(), json!(true));
            }
            extra.push(("truncated".to_string(), json!(meta_truncated)));

            // Suggest find_references when keyword query matches exact symbol name.
            if ctx.query_mode_effective == "keyword" && items.len() <= 3 {
                for item in items.iter().take(3) {
                    if let Some(title) = item.pointer("/ref/title").and_then(|v| v.as_str())
                        && title.eq_ignore_ascii_case(ctx.query)
                    {
                        extra.push((
                            "suggestion".to_string(),
                            json!(format!(
                                "For complete reference tracking, use `inspect_graph graph=find_references identifier=\"{}\"`.",
                                ctx.query
                            )),
                        ));
                        break;
                    }
                }
            }

            svc.success_response_with_meta_extra(timer, data, warnings, extra)
        }
    }
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
        let mut args: SearchCatalogArgs = ValidatedArgs::from_validated(v)?.into_inner();

        let output_mode = match coerce_search_args(&mut args) {
            Ok(m) => m,
            Err(e) => return self.error_response(&MetaBuilder::new(0), e),
        };
        if let Err(e) = validate_presearch(&args) {
            return self.error_response(&MetaBuilder::new(0), e);
        }

        let scope = args.scope.clone().or(args.file_pattern.clone());
        let counting = output_mode == "count" || output_mode == "files_with_matches";
        let lim = if counting {
            args.limit.clamp(1, 10_000)
        } else {
            args.limit.clamp(1, 50)
        };
        let query_mode_effective = args
            .query_mode
            .clone()
            .unwrap_or_else(|| "keyword".to_string())
            .to_ascii_lowercase();
        let domain_explicit = args.domain.is_some();
        let domain = match args.domain.clone() {
            Some(d) => d.to_ascii_lowercase(),
            None => match query_mode_effective.as_str() {
                "natural_language" => "hybrid".to_string(),
                _ => "code".to_string(),
            },
        };

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
            match self.search_code_side(
                store,
                &args,
                scope.as_ref(),
                &flags,
                per_kind_cap,
                &mut items,
            ) {
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

        let semantic_requested = args.semantic.unwrap_or(false)
            || query_mode_effective == "natural_language";
        if semantic_requested && (flags.want_symbol || flags.want_section) {
            let dense = run_dense_lane(
                self,
                &args.query,
                code_store.as_ref(),
                doc_store.as_ref(),
                lim,
            );
            if let Some(w) = dense.warning {
                warnings.push(w);
            } else {
                items = fuse_items_with_dense(items, &dense.symbol_ids, &dense.section_ids);
                if items.len() > lim {
                    items.truncate(lim);
                }
            }
        }

        let timer = MetaBuilder::new(raw_bytes.max(1));
        let extra: Vec<(String, Value)> = vec![
            ("domain_effective".to_string(), json!(domain)),
            (
                "query_mode_effective".to_string(),
                json!(query_mode_effective),
            ),
            ("domain_explicit".to_string(), json!(domain_explicit)),
            ("semantic_applied".to_string(), json!(semantic_requested)),
        ];

        let fmt_ctx = FormatCtx {
            output_mode: &output_mode,
            query: &args.query,
            query_mode_effective: &query_mode_effective,
            lim,
            max_tokens: args.max_tokens,
            more_file_hits_exist,
        };
        format_output(self, &fmt_ctx, items, &timer, warnings, extra)
    }
}
