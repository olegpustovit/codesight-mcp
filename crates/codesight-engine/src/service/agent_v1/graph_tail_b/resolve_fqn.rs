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

        let root = store.root_path().ok();
        let psr4_map = root.as_ref().and_then(|r| {
            let composer = r.join("composer.json");
            let content = std::fs::read_to_string(composer).ok()?;
            let parsed: Value = serde_json::from_str(&content).ok()?;
            let autoload = parsed.get("autoload")?.get("psr-4")?.as_object()?;
            let mut map = std::collections::HashMap::new();
            for (ns, dir) in autoload {
                if let Some(d) = dir.as_str() {
                    map.insert(ns.clone(), d.to_string());
                }
            }
            Some(map)
        });
        let Some(psr4_map) = psr4_map else {
            return self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    category: ToolErrorCategory::NotFound,
                    code: "NOT_FOUND",
                    message: "No PSR-4 autoload map found (composer.json missing or invalid)."
                        .into(),
                    detail: json!({}),
                },
            );
        };

        let source_files: HashSet<String> = store.all_file_paths()?.into_iter().collect();
        match codesight_analysis::graph::fqn_to_symbol(fqn, &psr4_map, &source_files) {
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
