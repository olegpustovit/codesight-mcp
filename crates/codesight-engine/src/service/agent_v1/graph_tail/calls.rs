//! `inspect_graph graph=calls` — text-heuristic call hierarchy over indexed symbols.

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(super) fn graph_calls(&self, args: &InspectGraphArgs) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &MetaBuilder::new(0));
        let Some(symbol_id) = args.symbol_id.as_deref() else {
            return self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "calls mode requires symbol_id.".into(),
                    detail: json!({}),
                },
            );
        };

        let sym_infos = store.all_symbols_with_source()?;

        let Some(target) = sym_infos.iter().find(|s| s.id == symbol_id) else {
            return self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    category: ToolErrorCategory::NotFound,
                    code: "NOT_FOUND",
                    message: format!("Symbol '{symbol_id}' not found in index."),
                    detail: json!({ "symbol_id": symbol_id }),
                },
            );
        };

        let edges = codesight_analysis::graph::build_call_edges(&sym_infos);
        let direction = args.direction.as_deref().unwrap_or("both");
        let depth = args.depth.unwrap_or(3);

        let caller_tuples = if direction == "callees" {
            vec![]
        } else {
            codesight_analysis::graph::find_callers(symbol_id, &edges, depth)
        };
        let callee_tuples = if direction == "callers" {
            vec![]
        } else {
            codesight_analysis::graph::find_callees(symbol_id, &edges, depth)
        };

        // Compact row: drop `id` (reconstructible from `file::name#kind`).
        // Keep `kind` only when non-default; "function" is the dominant kind
        // and omitting it saves ~20 chars per row (PLAN2 §3.5a).
        let to_row = |tuples: &[(String, String, String, u32, String, usize)]| -> Vec<Value> {
            tuples
                .iter()
                .map(|(_id, name, file, line, kind, depth)| {
                    let mut m = serde_json::Map::new();
                    m.insert("name".to_string(), json!(name));
                    m.insert("file".to_string(), json!(file));
                    m.insert("line".to_string(), json!(line));
                    m.insert("depth".to_string(), json!(depth));
                    if kind != "function" {
                        m.insert("kind".to_string(), json!(kind));
                    }
                    Value::Object(m)
                })
                .collect()
        };

        // If every row in a side (callers / callees) shares a single file,
        // hoist `file` to a sibling field and strip it per row (PLAN2 §3.5b).
        let hoist_shared_file = |rows: &mut Vec<Value>| -> Option<String> {
            let first_file = rows.first()?.get("file")?.as_str()?.to_string();
            if rows
                .iter()
                .all(|r| r.get("file").and_then(|v| v.as_str()) == Some(first_file.as_str()))
            {
                for r in rows.iter_mut() {
                    if let Some(obj) = r.as_object_mut() {
                        obj.remove("file");
                    }
                }
                Some(first_file)
            } else {
                None
            }
        };

        let mut callers_json = to_row(&caller_tuples);
        let mut callees_json = to_row(&callee_tuples);
        let callers_file = hoist_shared_file(&mut callers_json);
        let callees_file = hoist_shared_file(&mut callees_json);
        let max_depth = caller_tuples
            .iter()
            .map(|t| t.5)
            .chain(callee_tuples.iter().map(|t| t.5))
            .max()
            .unwrap_or(0);

        // Token budget (default 8k). Without this, high-fan-in symbols
        // returned 200k+ char responses that blew past the harness cap.
        const DEFAULT_CALLS_TOKEN_BUDGET: u32 = 8000;
        let budget = args.max_tokens.unwrap_or(DEFAULT_CALLS_TOKEN_BUDGET);
        let (callers_json, callers_truncated) =
            crate::service::agent_v1::util::pack_catalog_items_to_token_budget(
                callers_json,
                budget / 2,
            );
        let (callees_json, callees_truncated) =
            crate::service::agent_v1::util::pack_catalog_items_to_token_budget(
                callees_json,
                budget / 2,
            );
        let truncated = callers_truncated || callees_truncated;

        // `build_call_edges` is a whole-word name match over every symbol
        // body in the index. For short/common names (`new`, `get`, `as_str`,
        // `take`) the edge set is near-random, so every response must carry a
        // prominent warning. The inner `_meta.source` field duplicated that
        // signal and bloated every row; rely on the warning alone.
        let warnings = vec![json!({
            "code": "CALL_GRAPH_HEURISTIC",
            "message": "graph=calls uses a text-based whole-word heuristic over symbol bodies; edges may include unrelated symbols that share a common method name (e.g. `new`, `get`, `take`). Use this for exploration only — not for rename_safety or blast_radius decisions.",
        })];

        let mut body = serde_json::Map::new();
        body.insert("graph".to_string(), json!("calls"));
        body.insert(
            "symbol".to_string(),
            json!({
                "id": target.id,
                "name": target.name,
                "kind": target.kind,
                "file": target.file,
                "line": target.line,
            }),
        );
        body.insert("callers".to_string(), json!(callers_json));
        body.insert("callees".to_string(), json!(callees_json));
        if let Some(f) = callers_file {
            body.insert("callers_file".to_string(), json!(f));
        }
        if let Some(f) = callees_file {
            body.insert("callees_file".to_string(), json!(f));
        }
        body.insert("caller_count".to_string(), json!(caller_tuples.len()));
        body.insert("callee_count".to_string(), json!(callee_tuples.len()));
        body.insert("depth_reached".to_string(), json!(max_depth));
        body.insert("truncated".to_string(), json!(truncated));

        self.success_response_with_warnings(
            &MetaBuilder::new(0),
            Value::Object(body),
            warnings,
        )
    }
}
