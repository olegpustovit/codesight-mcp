//! `inspect_graph` tail dispatcher — routes to per-mode submodules.

mod calls;
mod centrality;
mod dependency;
mod importers;
mod layers;

use serde_json::Value;

use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;

impl CodeService {
    pub(in crate::service) fn inspect_graph_tail(
        &self,
        args: &InspectGraphArgs,
        mode: &str,
        limit: usize,
        warnings: &mut Vec<Value>,
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
            tail => self.inspect_graph_tail_b(args, tail, limit, warnings),
        }
    }
}
