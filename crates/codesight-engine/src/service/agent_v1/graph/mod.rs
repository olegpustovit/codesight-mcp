//! `inspect_graph` — unified dispatcher over all graph modes.

mod calls;
mod centrality;
mod dependency;
pub(crate) mod find_references;
mod importers;
mod layers;
mod references;
mod resolve_fqn;

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
        let limit = (if args.limit == 0 { 25 } else { args.limit }).clamp(1, 50);

        match mode.as_str() {
            "references" => self.graph_references(&args, &mode, limit),
            "related" => self.graph_related(&args, &mode, limit),
            "dependency_cycles" => self.graph_dependency_cycles(&args, &mode),
            "dependencies" => self.graph_dependencies(&args, &mode),
            "module_coupling" => self.graph_module_coupling(&args, &mode),
            "layer_violations" => self.graph_layer_violations(&args, &mode),
            "calls" => self.graph_calls(&args),
            "centrality" => self.graph_centrality(&args),
            "imports" => self.graph_imports(&args),
            "importers" => self.graph_importers(&args),
            "find_references" => self.graph_find_references(&args),
            "resolve_fqn" => self.graph_resolve_fqn(&args),
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
