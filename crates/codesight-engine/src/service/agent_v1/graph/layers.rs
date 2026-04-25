//! `inspect_graph` coupling + layer-violation modes.

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::{Confidence, MetaBuilder};

impl CodeService {
    pub(super) fn graph_module_coupling(
        &self,
        args: &InspectGraphArgs,
        mode: &str,
    ) -> anyhow::Result<Value> {
        let module_path = match args.module_path.clone() {
            Some(p) => p,
            None => {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
                        code: "INVALID_REQUEST",
                        message: "module_coupling requires module_path.".to_string(),
                        detail: json!({}),
                    },
                );
            }
        };
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &MetaBuilder::new(0));
        if !store.file_exists(&module_path)? {
            return self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    category: ToolErrorCategory::NotFound,
                    code: "NOT_FOUND",
                    message: format!("File '{module_path}' not found in index."),
                    detail: json!({ "module_path": module_path }),
                },
            );
        }
        // ROADMAP P1.1: structural analysis — no native-tool counterfactual
        // that maps to `repo_total_bytes`. Null baseline is the honest signal.
        let timer = MetaBuilder::new(0).with_confidence(Confidence::Safe);
        let adj = store.dependency_graph_strict()?;
        let metrics = codesight_analysis::graph::compute_coupling(&module_path, &adj);
        self.success_response(
            &timer,
            json!({
                "graph": mode,
                "repo": args.source_id,
                "module": metrics.module,
                "ca": metrics.ca,
                "ce": metrics.ce,
                "instability": metrics.instability,
                "assessment": metrics.assessment,
                "importers": metrics.importers,
                "dependencies": metrics.dependencies,
            }),
        )
    }

    pub(super) fn graph_layer_violations(
        &self,
        args: &InspectGraphArgs,
        mode: &str,
    ) -> anyhow::Result<Value> {
        let rules = args.rules.clone().unwrap_or_default();
        if rules.is_empty() {
            let _store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &MetaBuilder::new(0));
            return self.success_response(
                &MetaBuilder::new(0).with_confidence(Confidence::Safe),
                json!({
                    "graph": mode,
                    "repo": args.source_id,
                    "layer_count": 0,
                    "violation_count": 0,
                    "violations": [],
                    "note": "No layer rules provided.",
                }),
            );
        }
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &MetaBuilder::new(0));
        // ROADMAP P1.1: structural analysis — no native-tool counterfactual
        // that maps to `repo_total_bytes`. Null baseline is the honest signal.
        let timer = MetaBuilder::new(0).with_confidence(Confidence::Safe);
        let adj = store.dependency_graph_strict()?;
        let violations = codesight_analysis::graph::find_layer_violations(&adj, &rules);
        self.success_response(
            &timer,
            json!({
                "graph": mode,
                "repo": args.source_id,
                "layer_count": rules.len(),
                "violation_count": violations.len(),
                "violations": violations,
            }),
        )
    }
}
