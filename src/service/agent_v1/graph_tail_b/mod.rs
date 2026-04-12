mod find_references;
mod resolve_fqn;

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(in crate::service) fn inspect_graph_tail_b(
        &self,
        args: &InspectGraphArgs,
        mode: &str,
        _limit: usize,
        _warnings: &mut Vec<Value>,
    ) -> anyhow::Result<Value> {
        match mode {
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
