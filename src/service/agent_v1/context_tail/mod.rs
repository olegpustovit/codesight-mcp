mod query_suggestions;
mod rename_plan;
mod section_context;
use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::AssembleContextArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(in crate::service) fn assemble_context_tail(
        &self,
        args: &AssembleContextArgs,
        mode: &str,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        match mode {
            "section_context" => self.context_section(args, timer),
            "query_suggestions" => self.context_query_suggestions(args, timer),
            "context_bundle" => self.assemble_context_bundle(args, timer),
            "rename_plan" => self.context_rename_plan(args, timer),
            _ => self.error_response(
                timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: format!("Unknown assemble_context mode '{}'.", args.mode),
                    detail: json!({ "mode": args.mode }),
                },
            ),
        }
    }
}
