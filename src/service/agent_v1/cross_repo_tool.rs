//! Cross-repo package / import map (`get_cross_repo_map`).

use serde::Deserialize;
use serde_json::{Value, json};

use crate::analysis::cross_repo_map::{self, BuildCrossRepoOutcome};
use crate::service::CodeService;
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(in crate::service) fn get_cross_repo_map(&self, v: Value) -> anyhow::Result<Value> {
        #[derive(Deserialize)]
        struct Args {
            #[serde(default)]
            source_id: Option<String>,
        }
        let args: Args = ValidatedArgs::from_validated(v)?.into_inner();
        let timer = MetaBuilder::new(0);

        match cross_repo_map::build_cross_repo_map(&self.config, args.source_id.as_deref())? {
            BuildCrossRepoOutcome::FilterMissing { source_id } => self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::NotFound,
                    code: "NOT_FOUND",
                    message: format!("Source '{source_id}' not found in index."),
                    detail: json!({ "source_id": source_id }),
                },
            ),
            BuildCrossRepoOutcome::Found {
                repos,
                cross_repo_edges,
            } => self.success_response(
                &timer,
                json!({
                    "repos": repos,
                    "cross_repo_edges": cross_repo_edges,
                }),
            ),
        }
    }
}
