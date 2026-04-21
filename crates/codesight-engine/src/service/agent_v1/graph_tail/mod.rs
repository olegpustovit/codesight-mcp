//! `inspect_graph` tail dispatcher — routes to per-mode submodules.

mod calls;
mod centrality;
mod dependency;
pub(crate) mod find_references;
mod importers;
mod layers;
mod resolve_fqn;

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(in crate::service) fn inspect_graph_tail(
        &self,
        args: &InspectGraphArgs,
        mode: &str,
        _limit: usize,
        _warnings: &mut Vec<Value>,
    ) -> anyhow::Result<Value> {
        match mode {
            "dependency_cycles" => self.graph_dependency_cycles(args, mode),
            "dependencies" => self.graph_dependencies(args, mode),
            "module_coupling" => self.graph_module_coupling(args, mode),
            "layer_violations" => self.graph_layer_violations(args, mode),
            "calls" => self.graph_calls(args),
            "centrality" => self.graph_centrality(args),
            "imports" => self.graph_imports(args),
            "importers" => self.graph_importers(args),
            "find_references" => self.graph_find_references(args),
            "resolve_fqn" => self.graph_resolve_fqn(args),
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
