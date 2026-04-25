//! Context assembly — unified dispatcher for every `assemble_context` mode.

mod bundle;
mod gather;
mod query_suggestions;
mod rename;
mod section;
mod shape;

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::RANKED_CONTEXT_MAX_QUERY_LEN;
use crate::service::agent_v1::args::{
    AssembleContextArgs, ContentRef, ReadContentArgs, SearchCatalogArgs,
};
use crate::service::agent_v1::util::*;
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::savings::{Confidence, MetaBuilder, sum_code_file_sizes};

/// Build a minimal `SearchCatalogArgs` with only the essential fields set.
fn search_args(source_id: &str, query: &str, repo_path: Option<&str>) -> SearchCatalogArgs {
    SearchCatalogArgs {
        source_id: source_id.to_string(),
        query: query.to_string(),
        domain: None,
        unit_kinds: None,
        scope: None,
        exclude: None,
        file_pattern: None,
        kind: None,
        limit: 10,
        query_mode: None,
        context_lines: None,
        include_body: None,
        max_tokens: None,
        fuzzy: None,
        repo_path: repo_path.map(|s| s.to_string()),
        semantic: None,
        sort_by: None,
        output_mode: None,
        pattern: None,
        offset: 0,
        include_tests: None,
        name_only: None,
        include_vendored: None,
    }
}

/// Extract stable_id from a search hit's `ref` block.
fn ref_stable_id(ref_obj: &Value) -> &str {
    ref_obj
        .get("stable_id")
        .or_else(|| ref_obj.get("id"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
}

/// Build a single-symbol `ReadContentArgs` from a search hit's ref block.
fn read_args_for_ref(
    ref_obj: &Value,
    source_id: &str,
    repo_path: Option<&str>,
    include: &[&str],
) -> ReadContentArgs {
    let sid = ref_stable_id(ref_obj);
    let kind = ref_obj
        .get("kind")
        .and_then(|x| x.as_str())
        .unwrap_or("symbol");
    ReadContentArgs {
        source_id: source_id.to_string(),
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
            symbol_name: None,
        }],
        include: Some(include.iter().map(|s| s.to_string()).collect()),
        verify: false,
        context_lines: 0,
        repo_path: repo_path.map(|s| s.to_string()),
        max_tokens: None,
        wait_for_fresh: None,
        max_wait_ms: None,
    }
}

fn invalid_request(msg: String) -> ToolError {
    ToolError {
        category: ToolErrorCategory::InvalidArgument,
        code: "INVALID_REQUEST",
        message: msg,
        detail: json!({}),
    }
}

impl CodeService {
    fn assemble_bundle(
        &self,
        args: &AssembleContextArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let q = match args.query.as_deref().filter(|q| !q.trim().is_empty()) {
            Some(q) => q,
            None => {
                return self.error_response(
                    timer,
                    invalid_request("assemble_context bundle requires query.".into()),
                );
            }
        };
        let mut sa = search_args(&args.source_id, q, args.repo_path.as_deref());
        sa.domain = Some("hybrid".to_string());
        sa.limit = args.limit;

        let search_res = self.search_catalog(serde_json::to_value(sa)?)?;
        let items = search_res["data"]["items"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let repo_path = args.repo_path.as_deref();

        let mut packed = Vec::new();
        for item in items.into_iter().take(args.limit) {
            let ref_obj = item.get("ref").cloned().unwrap_or(Value::Null);
            if ref_stable_id(&ref_obj).is_empty() {
                continue;
            }
            let ra = read_args_for_ref(&ref_obj, &args.source_id, repo_path, &["body", "summary"]);
            if let Ok(read_res) = self.read_content(serde_json::to_value(&ra)?) {
                packed.push(json!({ "hit": item, "read": read_res["data"] }));
            }
        }
        // ROADMAP P1.1: baseline = sum of full-file reads for every
        // included ref. The naive alternative to `assemble_context bundle`
        // is "open each file and read it end-to-end"; that's what we
        // charge against.
        let bundle_paths: Vec<String> = packed
            .iter()
            .filter_map(|p| {
                p.pointer("/hit/ref/path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .collect();
        let raw_bytes = self
            .ensure_repo_store(&args.source_id, repo_path)
            .ok()
            .and_then(|store| sum_code_file_sizes(&store, bundle_paths).ok())
            .unwrap_or(0);
        self.success_response(
            &MetaBuilder::new(raw_bytes).with_confidence(Confidence::Partial),
            json!({ "mode": "bundle", "items": packed }),
        )
    }

    fn assemble_ranked(
        &self,
        args: &AssembleContextArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let q = match args.query.as_deref().filter(|q| !q.trim().is_empty()) {
            Some(q) => q.trim(),
            None => {
                return self.error_response(
                    timer,
                    invalid_request("assemble_context ranked requires query.".into()),
                );
            }
        };
        if q.len() > RANKED_CONTEXT_MAX_QUERY_LEN {
            return self.error_response(timer, ToolError {
                category: ToolErrorCategory::InvalidArgument,
                code: "INVALID_REQUEST",
                message: format!("Query exceeds ranked context limit of {RANKED_CONTEXT_MAX_QUERY_LEN} characters."),
                detail: json!({ "query_len": q.len() }),
            });
        }
        if args.max_tokens == Some(0) {
            return self.error_response(
                timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
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

        let mut sa = search_args(&args.source_id, q, args.repo_path.as_deref());
        sa.domain = Some("code".to_string());
        sa.unit_kinds = Some(vec!["symbol".to_string()]);
        sa.kind = kind;
        sa.limit = lim;

        let search_res = self.search_catalog(serde_json::to_value(sa)?)?;
        if search_res.get("ok").and_then(|b| b.as_bool()) != Some(true) {
            return Ok(search_res);
        }
        let items = search_res["data"]["items"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let budget = args.max_tokens;
        let repo_path = args.repo_path.as_deref();

        let mut context_items: Vec<Value> = Vec::new();
        let mut included_paths: Vec<String> = Vec::new();
        let mut total_tokens: u64 = 0;

        for item in items {
            let ref_obj = item.get("ref").cloned().unwrap_or(Value::Null);
            if ref_obj.get("kind").and_then(|x| x.as_str()) != Some("symbol") {
                continue;
            }
            let sid = ref_stable_id(&ref_obj);
            if sid.is_empty() {
                continue;
            }

            let ra = read_args_for_ref(&ref_obj, &args.source_id, repo_path, &["body"]);
            let Ok(read_res) = self.read_content(serde_json::to_value(&ra)?) else {
                continue;
            };
            if read_res.get("ok").and_then(|b| b.as_bool()) != Some(true) {
                continue;
            }

            let source = read_res["data"]["items"][0]["source"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let tks = token_estimate_for_context(&source);
            if let Some(max_t) = budget
                && total_tokens.saturating_add(tks as u64) > max_t as u64
            {
                break;
            }
            total_tokens = total_tokens.saturating_add(tks as u64);
            if let Some(p) = ref_obj.get("path").and_then(|v| v.as_str()) {
                included_paths.push(p.to_string());
            }
            context_items.push(json!({
                "symbol_id": sid,
                "source": source,
                "relevance_score": item.get("score").and_then(|s| s.as_f64()).unwrap_or(0.0),
                "tokens": tks,
            }));
            if context_items.len() >= lim {
                break;
            }
        }

        // ROADMAP P1.1: baseline = sum of full-file reads for the symbol
        // files the agent asked us to rank, not the entire repo.
        let raw_bytes = self
            .ensure_repo_store(&args.source_id, repo_path)
            .ok()
            .and_then(|store| sum_code_file_sizes(&store, included_paths).ok())
            .unwrap_or(0);
        self.success_response_with_warnings(
            &MetaBuilder::new(raw_bytes).with_confidence(Confidence::Partial),
            json!({ "mode": "ranked", "context_items": context_items, "total_tokens": total_tokens, "strategy": strategy_label }),
            Vec::new(),
        )
    }

    pub(in crate::service) fn assemble_context(&self, v: Value) -> anyhow::Result<Value> {
        let args: AssembleContextArgs = ValidatedArgs::from_validated(v)?.into_inner();
        let timer = MetaBuilder::new(0).with_confidence(Confidence::Partial);
        match args.mode.to_ascii_lowercase().as_str() {
            "bundle" => self.assemble_bundle(&args, &timer),
            "ranked" => self.assemble_ranked(&args, &timer),
            "section_context" => self.context_section(&args, &timer),
            "query_suggestions" => self.context_query_suggestions(&args, &timer),
            "context_bundle" => self.assemble_context_bundle(&args, &timer),
            "rename_plan" => self.context_rename_plan(&args, &timer),
            _ => self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: format!("Unknown assemble_context mode '{}'.", args.mode),
                    detail: json!({ "mode": args.mode }),
                },
            ),
        }
    }
}
