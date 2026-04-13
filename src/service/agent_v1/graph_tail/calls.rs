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

        let edges = crate::analysis::graph::build_call_edges(&sym_infos);
        let direction = args.direction.as_deref().unwrap_or("both");
        let depth = args.depth.unwrap_or(3);

        let caller_tuples = if direction == "callees" {
            vec![]
        } else {
            crate::analysis::graph::find_callers(symbol_id, &edges, depth)
        };
        let callee_tuples = if direction == "callers" {
            vec![]
        } else {
            crate::analysis::graph::find_callees(symbol_id, &edges, depth)
        };

        let to_json = |tuples: &[(String, String, String, u32, String, usize)]| -> Vec<Value> {
            tuples
                .iter()
                .map(|(id, name, file, line, kind, depth)| {
                    json!({
                        "id": id, "name": name, "kind": kind,
                        "file": file, "line": line, "depth": depth,
                    })
                })
                .collect()
        };

        let callers_json = to_json(&caller_tuples);
        let callees_json = to_json(&callee_tuples);
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
        // prominent warning — `_meta.source: "text_heuristic"` alone is too
        // easy for clients to miss.
        let warnings = vec![json!({
            "code": "CALL_GRAPH_HEURISTIC",
            "message": "graph=calls uses a text-based whole-word heuristic over symbol bodies; edges may include unrelated symbols that share a common method name (e.g. `new`, `get`, `take`). Use this for exploration only — not for rename_safety or blast_radius decisions.",
        })];

        self.success_response_with_warnings(
            &MetaBuilder::new(0),
            json!({
                "graph": "calls",
                "symbol": {
                    "id": target.id,
                    "name": target.name,
                    "kind": target.kind,
                    "file": target.file,
                    "line": target.line,
                },
                "callers": callers_json,
                "callees": callees_json,
                "caller_count": caller_tuples.len(),
                "callee_count": callee_tuples.len(),
                "depth_reached": max_depth,
                "truncated": truncated,
                "_meta": { "source": "text_heuristic" },
            }),
            warnings,
        )
    }
}
