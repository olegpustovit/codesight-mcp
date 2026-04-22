//! `evaluate_impact analysis=rename_safety` — refs-driven rename validator.

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateImpactArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;
use codesight_analysis::impact;

impl CodeService {
    pub(in crate::service) fn evaluate_impact_rename_safety(
        &self,
        args: &EvaluateImpactArgs,
    ) -> anyhow::Result<Value> {
        let timer = MetaBuilder::new(0);
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &timer);

        let r = args.refs.as_ref().and_then(|refs| refs.first());

        let Some(r) = r else {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "rename_safety requires refs[0].".to_string(),
                    detail: json!({}),
                },
            );
        };

        let sym_id = r.stable_id.as_deref().or(r.id.as_deref()).unwrap_or("");

        let new_name = r.new_name.as_deref().unwrap_or("");

        if new_name.is_empty() {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "rename_safety requires new_name on the ref.".to_string(),
                    detail: json!({}),
                },
            );
        }
        let data = match impact::rename_safety_payload(&store, sym_id, new_name)? {
            Some(data) => data,
            None => {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::NotFound,
                        code: "NOT_FOUND",
                        message: format!("Symbol '{sym_id}' not found in index."),
                        detail: json!({ "stable_id": sym_id }),
                    },
                );
            }
        };
        let raw_bytes = store.repo_total_bytes().unwrap_or(0);
        let timer = MetaBuilder::new(raw_bytes);
        self.success_response(&timer, data)
    }
}
