//! `search_catalog` — BM25 symbols/files/sections with optional fuzzy + semantic layers.

mod fuzzy;
mod semantic;

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::service::agent_v1::args::SearchCatalogArgs;
use crate::service::agent_v1::util::*;
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory, index_error};
use crate::service::savings::MetaBuilder;
use crate::service::{CodeService, MAX_QUERY_LEN};
use codesight_storage::storage::IndexStore;
use codesight_storage::storage::doc_index::DocStore;

use semantic::{DenseRankedHit, run_dense_lane, semantic_fallback_warning};

/// Server-side serialization byte cap for the `items` payload. We stay well
/// below the MCP client's overflow-to-file guard so broad queries trim on
/// the server side with a `truncated_by=["byte_budget"]` hint instead of
/// returning a giant `.txt` pointer.
const MAX_RESPONSE_BYTES: usize = 12_000;

/// Fuse the already-ranked lexical items with the dense lane's id lists
/// via RRF. Symbol and section lanes fuse separately; file-text hits
/// (which the dense lane never produces) pass through unchanged.
fn dense_similarity(distance: f64) -> f64 {
    1.0 / (1.0 + distance.max(0.0))
}

fn materialize_dense_symbol_items(
    svc: &CodeService,
    source_id: &str,
    store: Option<&IndexStore>,
    dense_hits: &[DenseRankedHit],
    include_body: bool,
    context_lines: usize,
) -> Result<std::collections::HashMap<String, Value>, ToolError> {
    let Some(store) = store else {
        return Ok(std::collections::HashMap::new());
    };
    let ids: Vec<String> = dense_hits.iter().map(|hit| hit.id.clone()).collect();
    let scores: std::collections::HashMap<&str, f64> = dense_hits
        .iter()
        .map(|hit| (hit.id.as_str(), dense_similarity(hit.distance)))
        .collect();
    let rows = store.search_symbols_by_ids(&ids).map_err(index_error)?;
    Ok(rows
        .into_iter()
        .map(|mut row| {
            row.score = *scores.get(row.id.as_str()).unwrap_or(&0.0);
            let id = row.id.clone();
            let body = if include_body {
                svc.symbol_body_with_context(store, &row.id, context_lines)
                    .ok()
                    .flatten()
            } else {
                None
            };
            (id, search_hit_symbol(source_id, &row, body.as_deref()))
        })
        .collect())
}

fn materialize_dense_section_items(
    svc: &CodeService,
    source_id: &str,
    store: Option<&DocStore>,
    dense_hits: &[DenseRankedHit],
    include_body: bool,
    context_lines: usize,
) -> Result<std::collections::HashMap<String, Value>, ToolError> {
    let Some(store) = store else {
        return Ok(std::collections::HashMap::new());
    };
    let ids: Vec<String> = dense_hits.iter().map(|hit| hit.id.clone()).collect();
    let scores: std::collections::HashMap<&str, f64> = dense_hits
        .iter()
        .map(|hit| (hit.id.as_str(), dense_similarity(hit.distance)))
        .collect();
    let rows = store.search_sections_by_ids(&ids).map_err(index_error)?;
    Ok(rows
        .into_iter()
        .map(|mut row| {
            row.score = *scores.get(row.id.as_str()).unwrap_or(&0.0);
            let id = row.id.clone();
            let body = if include_body {
                svc.section_body_with_context(store, &row.id, context_lines)
                    .ok()
                    .flatten()
            } else {
                None
            };
            (id, search_hit_section(source_id, &row, body.as_deref()))
        })
        .collect())
}

fn merge_dense_ranked_items(
    items: Vec<Value>,
    dense_symbol_hits: &[DenseRankedHit],
    dense_section_hits: &[DenseRankedHit],
    mut dense_symbol_items: std::collections::HashMap<String, Value>,
    mut dense_section_items: std::collections::HashMap<String, Value>,
) -> Vec<Value> {
    // Partition: symbols, sections, others (file hits). Preserve input
    // order inside each partition so RRF sees the lexical ranking.
    let mut lex_symbols: Vec<String> = Vec::new();
    let mut lex_sections: Vec<String> = Vec::new();
    let mut symbol_items: std::collections::HashMap<String, Value> =
        std::collections::HashMap::new();
    let mut section_items: std::collections::HashMap<String, Value> =
        std::collections::HashMap::new();
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
                symbol_items.insert(i, item);
            }
            (Some(i), "section") => {
                lex_sections.push(i.clone());
                section_items.insert(i, item);
            }
            _ => others.push(item),
        }
    }

    let fused_symbols = codesight_retrieval::fusion::rrf(
        lex_symbols,
        dense_symbol_hits.iter().map(|hit| hit.id.clone()),
    );
    let fused_sections = codesight_retrieval::fusion::rrf(
        lex_sections,
        dense_section_hits.iter().map(|hit| hit.id.clone()),
    );

    let mut out: Vec<Value> = Vec::with_capacity(
        others.len()
            + symbol_items.len()
            + section_items.len()
            + dense_symbol_items.len()
            + dense_section_items.len(),
    );
    for (id, _score) in fused_symbols {
        if let Some(item) = symbol_items.remove(&id) {
            out.push(item);
        } else if let Some(item) = dense_symbol_items.remove(&id) {
            out.push(item);
        }
    }
    for (id, _score) in fused_sections {
        if let Some(item) = section_items.remove(&id) {
            out.push(item);
        } else if let Some(item) = dense_section_items.remove(&id) {
            out.push(item);
        }
    }
    out.extend(others);
    out
}

struct KindFlags {
    want_symbol: bool,
    want_section: bool,
    want_file: bool,
}

#[derive(Default, Clone, Copy)]
struct CodeSideStats {
    more_file_hits: bool,
    fuzzy_applied: bool,
}

struct SearchPlan {
    output_mode: String,
    scope: Option<String>,
    offset: usize,
    lim: usize,
    query_mode_effective: String,
    domain_explicit: bool,
    domain: String,
    flags: KindFlags,
    per_kind_cap: usize,
}

impl SearchPlan {
    fn build(args: &mut SearchCatalogArgs) -> Result<Self, ToolError> {
        let output_mode = coerce_search_args(args)?;
        validate_presearch(args)?;
        let scope = args.scope.clone().or(args.file_pattern.clone());
        let counting = output_mode == "count" || output_mode == "files_with_matches";
        let offset = args.offset;
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
        let domain = args
            .domain
            .clone()
            .map(|d| d.to_ascii_lowercase())
            .unwrap_or_else(|| match query_mode_effective.as_str() {
                "natural_language" => "hybrid".to_string(),
                _ => "code".to_string(),
            });
        let flags = KindFlags::from_args(args, &domain);
        let request_cap = lim.saturating_add(offset);
        let per_kind_cap = if domain == "hybrid" {
            (request_cap / 2).max(1)
        } else {
            request_cap
        };
        Ok(Self {
            output_mode,
            scope,
            offset,
            lim,
            query_mode_effective,
            domain_explicit,
            domain,
            flags,
            per_kind_cap,
        })
    }
}

#[derive(Default, Clone, Copy)]
struct LexicalStats {
    more_file_hits_exist: bool,
    fuzzy_applied: bool,
}

fn build_meta_extras(
    plan: &SearchPlan,
    fuzzy_applied: bool,
    semantic_requested: bool,
) -> Vec<(String, Value)> {
    vec![
        ("domain_effective".to_string(), json!(plan.domain)),
        (
            "query_mode_effective".to_string(),
            json!(plan.query_mode_effective),
        ),
        ("domain_explicit".to_string(), json!(plan.domain_explicit)),
        (
            "estimate_method".to_string(),
            json!("byte_approx_candidate_files"),
        ),
        (
            "baseline_scope".to_string(),
            json!("candidate_files_materialized_pre_output"),
        ),
        (
            "unit_kinds_effective".to_string(),
            json!({
                "symbol": plan.flags.want_symbol,
                "section": plan.flags.want_section,
                "file": plan.flags.want_file,
            }),
        ),
        ("fuzzy_applied".to_string(), json!(fuzzy_applied)),
        ("semantic_applied".to_string(), json!(semantic_requested)),
    ]
}

fn estimate_candidate_file_bytes(
    code_store: Option<&IndexStore>,
    doc_store: Option<&DocStore>,
    items: &[Value],
) -> Result<u64, ToolError> {
    let mut code_paths = BTreeSet::new();
    let mut doc_paths = BTreeSet::new();

    for item in items {
        let Some(path) = item.pointer("/ref/path").and_then(|value| value.as_str()) else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        match item.pointer("/ref/domain").and_then(|value| value.as_str()) {
            Some("docs") => {
                doc_paths.insert(path.to_string());
            }
            _ => {
                code_paths.insert(path.to_string());
            }
        }
    }

    let mut total = 0u64;
    if let Some(store) = code_store
        && !code_paths.is_empty()
    {
        let paths = code_paths.into_iter().collect::<Vec<_>>();
        total = total.saturating_add(
            store
                .sum_file_sizes_for_paths(&paths)
                .map_err(index_error)?,
        );
    }
    if let Some(store) = doc_store
        && !doc_paths.is_empty()
    {
        let paths = doc_paths.into_iter().collect::<Vec<_>>();
        total = total.saturating_add(
            store
                .sum_file_sizes_for_paths(&paths)
                .map_err(index_error)?,
        );
    }

    Ok(total)
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

#[derive(Default, Clone, Copy)]
struct SearchCounts {
    total_matches: usize,
    total_files_with_matches: usize,
}

impl SearchCounts {
    fn add(&mut self, m: usize, f: usize) {
        self.total_matches += m;
        self.total_files_with_matches += f;
    }
}

struct FormatCtx<'a> {
    output_mode: &'a str,
    query: &'a str,
    query_mode_effective: &'a str,
    offset: usize,
    lim: usize,
    max_tokens: Option<u32>,
    more_file_hits_exist: bool,
    counts: SearchCounts,
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
    let total_matches = ctx.counts.total_matches;
    let total_files_with_matches = ctx.counts.total_files_with_matches;
    match ctx.output_mode {
        "count" => {
            let data = json!({
                "count": total_matches,
                "files": total_files_with_matches,
                "total_matches": total_matches,
                "total_files_with_matches": total_files_with_matches,
            });
            extra.push(("truncated".to_string(), json!(false)));
            extra.push(("truncated_by".to_string(), json!(Vec::<&str>::new())));
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
            let file_items = file_items
                .into_iter()
                .skip(ctx.offset)
                .take(ctx.lim)
                .collect::<Vec<_>>();
            let data = json!({
                "files": file_items,
                "total_matches": total_matches,
                "total_files_with_matches": total_files_with_matches,
            });
            extra.push(("truncated".to_string(), json!(false)));
            extra.push(("truncated_by".to_string(), json!(Vec::<&str>::new())));
            svc.success_response_with_meta_extra(timer, data, warnings, extra)
        }
        _ => {
            let mut truncated_by: Vec<&'static str> = Vec::new();
            if ctx.offset > 0 {
                items = items.into_iter().skip(ctx.offset).collect();
            }
            if let Some(mt) = ctx.max_tokens {
                let (packed, bt) = pack_catalog_items_to_token_budget(items, mt);
                items = packed;
                if bt {
                    truncated_by.push("token_budget");
                }
            }

            let lim = ctx.lim;
            let pre_cap_exceeds_lim = total_matches > items.len().max(lim);
            let overflow = items.len() > lim;
            if overflow {
                warnings.push(json!({
                    "code": "TRUNCATED",
                    "message": format!("results truncated to limit {lim}"),
                }));
                items.truncate(lim);
            } else if pre_cap_exceeds_lim {
                warnings.push(json!({
                    "code": "TRUNCATED",
                    "message": format!("results truncated to limit {lim}"),
                }));
            }
            if overflow || pre_cap_exceeds_lim || ctx.more_file_hits_exist {
                truncated_by.push("limit");
            }

            // Server-side byte cap — trim items until serialized size stays
            // under `MAX_RESPONSE_BYTES` so we don't trip the MCP client's
            // overflow-to-file guard. Applied even after `limit` truncation.
            let mut byte_budgeted = false;
            while items.len() > 1
                && serde_json::to_vec(&items).map(|v| v.len()).unwrap_or(0)
                    > MAX_RESPONSE_BYTES
            {
                items.pop();
                byte_budgeted = true;
            }
            if byte_budgeted {
                truncated_by.push("byte_budget");
                warnings.push(json!({
                    "code": "TRUNCATED",
                    "message": format!(
                        "result trimmed to stay under {MAX_RESPONSE_BYTES}-byte server cap; narrow via scope=, file_pattern=, or a tighter query."
                    ),
                }));
            }

            let meta_truncated = !truncated_by.is_empty();
            let mut data = json!({
                "items": items,
                "total_matches": total_matches,
                "total_files_with_matches": total_files_with_matches,
            });
            if meta_truncated && let Value::Object(ref mut map) = data {
                map.insert("truncated".to_string(), json!(true));
                map.insert("truncated_by".to_string(), json!(truncated_by));
            }
            extra.push(("truncated".to_string(), json!(meta_truncated)));
            extra.push(("truncated_by".to_string(), json!(truncated_by)));

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

    #[allow(clippy::too_many_arguments)]
    fn search_code_side(
        &self,
        store: &IndexStore,
        args: &SearchCatalogArgs,
        scope: Option<&String>,
        flags: &KindFlags,
        per_kind_cap: usize,
        items: &mut Vec<Value>,
        counts: &mut SearchCounts,
    ) -> Result<CodeSideStats, ToolError> {
        let ctx_lines = args.context_lines.unwrap_or(0).min(50);
        let want_body = args.include_body.unwrap_or(false);
        let exclude = args.exclude.as_deref().unwrap_or(&[]);
        let mut more_file_hits = false;
        let mut fuzzy_applied = false;
        if flags.want_file {
            let qmode = args.query_mode.as_deref().unwrap_or("keyword");
            let result = self
                .search_code_files_text(
                    store,
                    &args.source_id,
                    &args.query,
                    scope.cloned(),
                    exclude,
                    per_kind_cap,
                    qmode,
                    ctx_lines,
                )
                .map_err(index_error)?;
            counts.add(result.total_matches, result.files_with_matches);
            more_file_hits = result.more_file_hits;
            items.extend(result.items);
        }
        if flags.want_symbol || (!flags.want_file && !flags.want_section) {
            let mut symbols = self.run_search(
                store,
                args.query.clone(),
                scope.cloned(),
                exclude,
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
                fuzzy_applied = fuzzy::augment_with_fuzzy(
                    store,
                    args,
                    scope.map(String::as_str),
                    exclude,
                    per_kind_cap,
                    auto_fuzzy,
                    &mut symbols,
                ) > 0;
            }
            for s in symbols {
                let body = if want_body {
                    self.symbol_body_with_context(store, &s.id, ctx_lines)
                        .ok()
                        .flatten()
                } else {
                    None
                };
                items.push(search_hit_symbol(&args.source_id, &s, body.as_deref()));
            }
        }
        Ok(CodeSideStats {
            more_file_hits,
            fuzzy_applied,
        })
    }

    fn search_doc_side(
        &self,
        store: &DocStore,
        args: &SearchCatalogArgs,
        scope: Option<&String>,
        per_kind_cap: usize,
        items: &mut Vec<Value>,
    ) -> Result<(), ToolError> {
        let exclude = args.exclude.as_deref().unwrap_or(&[]);
        let want_body = args.include_body.unwrap_or(false);
        let ctx_lines = args.context_lines.unwrap_or(0).min(50);
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
            .search_sections(
                effective_query,
                scope.map(String::as_str),
                exclude,
                per_kind_cap,
            )
            .map_err(index_error)?;
        for sec in sections {
            let body = if want_body {
                self.section_body_with_context(store, &sec.id, ctx_lines)
                    .ok()
                    .flatten()
            } else {
                None
            };
            items.push(search_hit_section(&args.source_id, &sec, body.as_deref()));
        }
        Ok(())
    }

    pub(in crate::service) fn search_catalog(&self, v: Value) -> anyhow::Result<Value> {
        let mut args: SearchCatalogArgs = ValidatedArgs::from_validated(v)?.into_inner();
        let plan = match SearchPlan::build(&mut args) {
            Ok(p) => p,
            Err(e) => return self.error_response(&MetaBuilder::new(0), e),
        };

        let mut warnings = Vec::new();
        let (code_store, doc_store) =
            match self.resolve_stores(&args, &plan.domain, &plan.flags, &mut warnings) {
                Ok(pair) => pair,
                Err(e) => return self.error_response(&MetaBuilder::new(0), e),
            };

        let mut items: Vec<Value> = Vec::new();
        let mut counts = SearchCounts::default();
        self.run_count_pass(
            &args,
            &plan,
            code_store.as_ref(),
            doc_store.as_ref(),
            &mut counts,
        );

        let stats = match self.run_lexical_pass(
            &args,
            &plan,
            code_store.as_ref(),
            doc_store.as_ref(),
            &mut items,
            &mut counts,
        ) {
            Ok(s) => s,
            Err(e) => return self.error_response(&MetaBuilder::new(1), e),
        };

        let semantic_requested = match self.run_semantic_pass(
            &args,
            &plan,
            code_store.as_ref(),
            doc_store.as_ref(),
            &mut items,
            &mut warnings,
        ) {
            Ok(b) => b,
            Err(e) => return self.error_response(&MetaBuilder::new(1), e),
        };

        let raw_bytes =
            match estimate_candidate_file_bytes(code_store.as_ref(), doc_store.as_ref(), &items) {
                Ok(bytes) => bytes,
                Err(e) => return self.error_response(&MetaBuilder::new(1), e),
            };
        let timer = MetaBuilder::new(raw_bytes.max(1))
            .with_class(crate::service::savings::RetrievalClass::Search);
        let extra = build_meta_extras(&plan, stats.fuzzy_applied, semantic_requested);

        let fmt_ctx = FormatCtx {
            output_mode: &plan.output_mode,
            query: &args.query,
            query_mode_effective: &plan.query_mode_effective,
            offset: plan.offset,
            lim: plan.lim,
            max_tokens: args.max_tokens,
            more_file_hits_exist: stats.more_file_hits_exist,
            counts,
        };
        format_output(self, &fmt_ctx, items, &timer, warnings, extra)
    }

    fn run_count_pass(
        &self,
        args: &SearchCatalogArgs,
        plan: &SearchPlan,
        code_store: Option<&IndexStore>,
        doc_store: Option<&DocStore>,
        counts: &mut SearchCounts,
    ) {
        let exclude = args.exclude.as_deref().unwrap_or(&[]);
        if let Some(store) = code_store
            && (plan.flags.want_symbol
                || (!plan.flags.want_file && !plan.flags.want_section))
            && let Ok((m, f)) = store.count_symbols(
                &args.query,
                args.kind.as_deref(),
                plan.scope.as_deref(),
                exclude,
            )
        {
            counts.add(m, f);
        }
        if let Some(dstore) = doc_store
            && (plan.flags.want_section || plan.domain == "docs" || plan.domain == "hybrid")
            && let Ok((m, f)) = dstore.count_sections(&args.query, plan.scope.as_deref(), exclude)
        {
            counts.add(m, f);
        }
    }

    fn run_lexical_pass(
        &self,
        args: &SearchCatalogArgs,
        plan: &SearchPlan,
        code_store: Option<&IndexStore>,
        doc_store: Option<&DocStore>,
        items: &mut Vec<Value>,
        counts: &mut SearchCounts,
    ) -> Result<LexicalStats, ToolError> {
        let mut more_file_hits_exist = false;
        let mut fuzzy_applied = false;
        if let Some(store) = code_store {
            let stats = self.search_code_side(
                store,
                args,
                plan.scope.as_ref(),
                &plan.flags,
                plan.per_kind_cap,
                items,
                counts,
            )?;
            more_file_hits_exist = stats.more_file_hits;
            fuzzy_applied = stats.fuzzy_applied;
        }
        if let Some(dstore) = doc_store
            && (plan.flags.want_section || plan.domain == "docs" || plan.domain == "hybrid")
        {
            self.search_doc_side(dstore, args, plan.scope.as_ref(), plan.per_kind_cap, items)?;
        }
        Ok(LexicalStats {
            more_file_hits_exist,
            fuzzy_applied,
        })
    }

    fn run_semantic_pass(
        &self,
        args: &SearchCatalogArgs,
        plan: &SearchPlan,
        code_store: Option<&IndexStore>,
        doc_store: Option<&DocStore>,
        items: &mut Vec<Value>,
        warnings: &mut Vec<Value>,
    ) -> Result<bool, ToolError> {
        let semantic_requested =
            args.semantic.unwrap_or(false) || plan.query_mode_effective == "natural_language";
        if !semantic_requested {
            return Ok(false);
        }
        if !(plan.flags.want_symbol || plan.flags.want_section) {
            warnings.push(semantic_fallback_warning(
                "files_only",
                "request only targets lexical file hits".to_string(),
            ));
            return Ok(true);
        }
        let dense = run_dense_lane(self, &args.query, code_store, doc_store, plan.lim);
        if let Some(w) = dense.warning {
            warnings.push(w);
            return Ok(true);
        }
        let include_body = args.include_body.unwrap_or(false);
        let context_lines = args.context_lines.unwrap_or(0).min(50);
        let dense_symbol_items = materialize_dense_symbol_items(
            self,
            &args.source_id,
            code_store,
            &dense.symbol_hits,
            include_body,
            context_lines,
        )?;
        let dense_section_items = materialize_dense_section_items(
            self,
            &args.source_id,
            doc_store,
            &dense.section_hits,
            include_body,
            context_lines,
        )?;
        let taken = std::mem::take(items);
        *items = merge_dense_ranked_items(
            taken,
            &dense.symbol_hits,
            &dense.section_hits,
            dense_symbol_items,
            dense_section_items,
        );
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol_item(id: &str) -> Value {
        json!({
            "ref": { "id": id, "kind": "symbol" },
            "snippet": "sig",
        })
    }

    fn file_item(path: &str) -> Value {
        json!({
            "ref": { "path": path, "kind": "file" },
            "snippet": "body",
        })
    }

    #[test]
    fn dense_only_hits_are_materialized_and_files_are_appended() {
        let merged = merge_dense_ranked_items(
            vec![file_item("src/main.rs"), symbol_item("lex")],
            &[
                DenseRankedHit {
                    id: "dense".to_string(),
                    distance: 0.1,
                },
                DenseRankedHit {
                    id: "lex".to_string(),
                    distance: 0.2,
                },
            ],
            &[],
            std::collections::HashMap::from([("dense".to_string(), symbol_item("dense"))]),
            std::collections::HashMap::new(),
        );
        let ids: Vec<&str> = merged
            .iter()
            .filter_map(|item| item.pointer("/ref/id").and_then(Value::as_str))
            .collect();
        assert_eq!(ids, vec!["lex", "dense"]);
        assert_eq!(merged.last().unwrap()["ref"]["kind"], "file");
    }
}
