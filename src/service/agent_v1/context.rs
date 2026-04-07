//! Context assembly (bundle, ranked, …).

use serde_json::{Value, json};

use crate::savings::MetaBuilder;
use crate::service::CodeService;
use crate::service::agent_v1::args::RANKED_CONTEXT_MAX_QUERY_LEN;
use crate::service::agent_v1::args::{
    AssembleContextArgs, ContentRef, ReadContentArgs, SearchCatalogArgs,
};
use crate::service::agent_v1::util::*;
use crate::service::args::parse_args;
use crate::service::helpers::ToolError;

impl CodeService {
    pub(in crate::service) fn assemble_context(&self, v: Value) -> anyhow::Result<Value> {
        let args: AssembleContextArgs = parse_args(v)?;
        let mode = args.mode.to_ascii_lowercase();
        let timer = MetaBuilder::new(0);

        match mode.as_str() {
            "bundle" => {
                let Some(q) = args.query.clone() else {
                    return self.error_response(
                        &timer,
                        ToolError {
                            code: "INVALID_REQUEST",
                            message: "assemble_context bundle requires query.".to_string(),
                            detail: json!({}),
                        },
                    );
                };
                let search_json = SearchCatalogArgs {
                    source_id: args.source_id.clone(),
                    query: q.clone(),
                    domain: Some("hybrid".to_string()),
                    unit_kinds: None,
                    scope: None,
                    file_pattern: None,
                    kind: None,
                    limit: args.limit,
                    query_mode: None,
                    context_lines: None,
                    max_tokens: None,
                    fuzzy: None,
                    fuzzy_threshold: None,
                    max_edit_distance: None,
                    repo_path: args.repo_path.clone(),
                    semantic: None,
                    semantic_weight: None,
                    semantic_only: None,
                    sort_by: None,
                };
                let search_res = self.search_catalog(serde_json::to_value(search_json)?)?;
                let items = search_res["data"]["items"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let mut packed = Vec::new();
                for item in items.into_iter().take(args.limit) {
                    let ref_obj = item.get("ref").cloned().unwrap_or(Value::Null);
                    let kind = ref_obj
                        .get("kind")
                        .and_then(|x| x.as_str())
                        .unwrap_or("symbol");
                    let sid = ref_obj
                        .get("stable_id")
                        .or_else(|| ref_obj.get("id"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("");
                    if sid.is_empty() {
                        continue;
                    }
                    let read_args = ReadContentArgs {
                        source_id: args.source_id.clone(),
                        refs: vec![ContentRef {
                            id: Some(sid.to_string()),
                            stable_id: Some(sid.to_string()),
                            kind: kind.to_string(),
                            path: ref_obj
                                .get("path")
                                .and_then(|x| x.as_str())
                                .map(|s| s.to_string()),
                            source_id: None,
                            start_line: None,
                            end_line: None,
                            new_name: None,
                        }],
                        include: Some(vec!["body".to_string(), "summary".to_string()]),
                        verify: false,
                        context_lines: 0,
                        repo_path: args.repo_path.clone(),
                    };
                    if let Ok(read_res) = self.read_content(serde_json::to_value(&read_args)?) {
                        packed.push(json!({ "hit": item, "read": read_res["data"] }));
                    }
                }
                let raw_bytes = packed
                    .iter()
                    .map(|p| {
                        serde_json::to_string(p)
                            .map(|s| s.len() as u64)
                            .unwrap_or(0)
                    })
                    .sum::<u64>()
                    .max(1);
                let timer = MetaBuilder::new(raw_bytes);
                self.success_response(
                    &timer,
                    json!({
                        "mode": "bundle",
                        "items": packed,
                    }),
                )
            }
            "ranked" => {
                let Some(q) = args.query.clone() else {
                    return self.error_response(
                        &timer,
                        ToolError {
                            code: "INVALID_REQUEST",
                            message: "assemble_context ranked requires query.".to_string(),
                            detail: json!({}),
                        },
                    );
                };
                let trimmed = q.trim();
                if trimmed.is_empty() {
                    return self.error_response(
                        &timer,
                        ToolError {
                            code: "INVALID_REQUEST",
                            message: "assemble_context ranked query is empty.".to_string(),
                            detail: json!({}),
                        },
                    );
                }
                if trimmed.len() > RANKED_CONTEXT_MAX_QUERY_LEN {
                    return self.error_response(
                        &timer,
                        ToolError {
                            code: "INVALID_REQUEST",
                            message: format!(
                                "Query exceeds ranked context limit of {RANKED_CONTEXT_MAX_QUERY_LEN} characters."
                            ),
                            detail: json!({ "query_len": trimmed.len() }),
                        },
                    );
                }
                if args.max_tokens == Some(0) {
                    return self.error_response(
                        &timer,
                        ToolError {
                            code: "INVALID_REQUEST",
                            message: "token_budget must be greater than zero for ranked context."
                                .to_string(),
                            detail: json!({ "token_budget": 0 }),
                        },
                    );
                }

                let strategy_label = args
                    .strategy
                    .as_deref()
                    .unwrap_or("bm25")
                    .to_ascii_lowercase();

                let kind = args
                    .kind_filter
                    .as_ref()
                    .and_then(|kinds| kinds.first())
                    .map(|s| s.trim().to_ascii_lowercase())
                    .filter(|s| !s.is_empty());

                let lim = args.limit.clamp(1, 50);
                let search_json = SearchCatalogArgs {
                    source_id: args.source_id.clone(),
                    query: trimmed.to_string(),
                    domain: Some("code".to_string()),
                    unit_kinds: Some(vec!["symbol".to_string()]),
                    scope: None,
                    file_pattern: None,
                    kind,
                    limit: lim,
                    query_mode: None,
                    context_lines: None,
                    max_tokens: None,
                    fuzzy: None,
                    fuzzy_threshold: None,
                    max_edit_distance: None,
                    repo_path: args.repo_path.clone(),
                    semantic: None,
                    semantic_weight: None,
                    semantic_only: None,
                    sort_by: None,
                };
                let search_res = self.search_catalog(serde_json::to_value(search_json)?)?;
                if search_res.get("ok").and_then(|b| b.as_bool()) != Some(true) {
                    return Ok(search_res);
                }
                let items = search_res["data"]["items"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();

                let budget = args.max_tokens;
                let mut context_items: Vec<Value> = Vec::new();
                let mut total_tokens: u64 = 0;
                let mut warnings = Vec::new();

                'hits: for item in items.into_iter() {
                    let ref_obj = item.get("ref").cloned().unwrap_or(Value::Null);
                    let unit_kind = ref_obj.get("kind").and_then(|x| x.as_str()).unwrap_or("");
                    if unit_kind != "symbol" {
                        continue;
                    }
                    let sid = ref_obj
                        .get("stable_id")
                        .or_else(|| ref_obj.get("id"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("");
                    if sid.is_empty() {
                        continue;
                    }
                    let relevance = item.get("score").and_then(|s| s.as_f64()).unwrap_or(0.0);
                    let centrality: f64 = 0.0;
                    let combined = relevance;

                    let read_args = ReadContentArgs {
                        source_id: args.source_id.clone(),
                        refs: vec![ContentRef {
                            id: Some(sid.to_string()),
                            stable_id: Some(sid.to_string()),
                            kind: "symbol".to_string(),
                            path: ref_obj
                                .get("path")
                                .and_then(|x| x.as_str())
                                .map(|s| s.to_string()),
                            source_id: None,
                            start_line: None,
                            end_line: None,
                            new_name: None,
                        }],
                        include: Some(vec!["body".to_string()]),
                        verify: false,
                        context_lines: 0,
                        repo_path: args.repo_path.clone(),
                    };
                    let Ok(read_res) = self.read_content(serde_json::to_value(&read_args)?) else {
                        continue;
                    };
                    if read_res.get("ok").and_then(|b| b.as_bool()) != Some(true) {
                        continue;
                    }
                    let source = read_res["data"]["items"][0]["content"]["source"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    let tks = token_estimate_for_context(&source);
                    if let Some(max_t) = budget {
                        if total_tokens.saturating_add(tks as u64) > max_t as u64 {
                            break 'hits;
                        }
                    }
                    total_tokens = total_tokens.saturating_add(tks as u64);
                    context_items.push(json!({
                        "symbol_id": sid,
                        "source": source,
                        "relevance_score": relevance,
                        "centrality_score": centrality,
                        "combined_score": combined,
                        "tokens": tks,
                    }));
                    if context_items.len() >= lim {
                        break;
                    }
                }

                if !context_items.is_empty() {
                    warnings.push(json!({
                        "code": "PARTIAL_RANKED_CONTEXT",
                        "message": "centrality_score is not computed yet; values are placeholders.",
                    }));
                }

                let body = json!({
                    "mode": "ranked",
                    "context_items": context_items,
                    "total_tokens": total_tokens,
                    "strategy": strategy_label,
                });
                let raw_bytes = serde_json::to_string(&body)
                    .map(|s| s.len() as u64)
                    .unwrap_or(1)
                    .max(1);
                let timer = MetaBuilder::new(raw_bytes);
                return self.success_response_with_warnings(&timer, body, warnings);
            }
            tail => self.assemble_context_tail(&args, tail, &timer),
        }
    }
}
