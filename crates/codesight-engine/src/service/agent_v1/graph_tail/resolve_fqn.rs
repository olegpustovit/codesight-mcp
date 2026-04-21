use std::collections::HashSet;

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(super) fn graph_resolve_fqn(&self, args: &InspectGraphArgs) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &MetaBuilder::new(0));
        let Some(fqn) = args.fqn.as_deref() else {
            return self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "resolve_fqn mode requires fqn parameter.".into(),
                    detail: json!({}),
                },
            );
        };

        let Some(root) = store.root_path().ok() else {
            return self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    category: ToolErrorCategory::NotFound,
                    code: "NOT_FOUND",
                    message: "Source root not available for FQN resolution.".into(),
                    detail: json!({}),
                },
            );
        };

        let source_files: HashSet<String> = store.all_file_paths()?.into_iter().collect();
        match codesight_language_support::resolve_fqn(fqn, &root, &source_files) {
            Some(symbol_id) => self.success_response(
                &MetaBuilder::new(0),
                json!({
                    "graph": "resolve_fqn",
                    "fqn": fqn,
                    "resolved": symbol_id,
                }),
            ),
            None => self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    category: ToolErrorCategory::NotFound,
                    code: "NOT_FOUND",
                    message: format!(
                        "FQN '{}' could not be resolved against the indexed source files.",
                        fqn
                    ),
                    detail: json!({ "fqn": fqn }),
                },
            ),
        }
    }
}
