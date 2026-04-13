//! `evaluate_impact analysis=diff` — line-oriented text diff between two indexed symbol spans.

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateImpactArgs;
use crate::service::agent_v1::util_graph::simple_text_diff;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(super) fn evaluate_impact_diff(&self, args: &EvaluateImpactArgs) -> anyhow::Result<Value> {
        let timer = MetaBuilder::new(0);

        let Some(refs) = args.refs.as_ref() else {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "diff requires refs array.".to_string(),
                    detail: json!({}),
                },
            );
        };
        if refs.len() < 2 {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "diff requires two symbol refs.".to_string(),
                    detail: json!({}),
                },
            );
        }
        let id_a = refs[0]
            .stable_id
            .clone()
            .or(refs[0].id.clone())
            .filter(|_| refs[0].kind.eq_ignore_ascii_case("symbol"));
        let id_b = refs[1]
            .stable_id
            .clone()
            .or(refs[1].id.clone())
            .filter(|_| refs[1].kind.eq_ignore_ascii_case("symbol"));
        let (Some(id_a), Some(id_b)) = (id_a, id_b) else {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "diff requires two symbol refs with kind symbol.".to_string(),
                    detail: json!({}),
                },
            );
        };
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &timer);
        let ra = store.symbol_record(&id_a)?;
        let rb = store.symbol_record(&id_b)?;
        let (Some(ra), Some(rb)) = (ra, rb) else {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::NotFound,
                    code: "NOT_FOUND",
                    message: "One or both symbols missing from index.".to_string(),
                    detail: json!({ "id_a": id_a, "id_b": id_b }),
                },
            );
        };
        if ra.file_path != rb.file_path {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "diff is supported for symbols in the same file only.".to_string(),
                    detail: json!({ "file_a": ra.file_path, "file_b": rb.file_path }),
                },
            );
        }
        let raw_bytes = ra.file_size_bytes.max(rb.file_size_bytes);
        let timer = MetaBuilder::new(raw_bytes);
        let patch = simple_text_diff(&ra.stored_source, &rb.stored_source);
        self.success_response(
            &timer,
            json!({
                "analysis": "diff",
                "findings": [{
                    "finding_id": "diff:symbol_pair",
                    "analysis": "diff",
                    "severity": "info",
                    "confidence": 1.0,
                    "summary": "line-oriented text diff of indexed symbol spans",
                    "evidence": patch,
                    "related_refs": [id_a, id_b],
                }],
            }),
        )
    }
}
