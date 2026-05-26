//! `inspect_graph` — three structurally-resolved modes only:
//! `find_references`, `imports`, `importers`. Per CUTPLAN, every other mode
//! (`calls`, `centrality`, `dependency_*`, `layer_violations`, `references`,
//! `related`, `resolve_fqn`) was retired because it text-scanned through
//! symbol bodies and could not satisfy the `confidence: safe` invariant.

pub(crate) mod find_references;
mod importers;

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(in crate::service) fn inspect_graph(&self, v: Value) -> anyhow::Result<Value> {
        let args: InspectGraphArgs = ValidatedArgs::from_validated(v)?.into_inner();
        let mode = args.graph.to_ascii_lowercase();

        match mode.as_str() {
            "imports" => self.graph_imports(&args),
            "importers" => self.graph_importers(&args),
            "find_references" => self.graph_find_references(&args),
            _ => self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: format!("Unknown inspect_graph graph mode '{mode}'."),
                    detail: json!({ "graph": args.graph }),
                },
            ),
        }
    }
}
