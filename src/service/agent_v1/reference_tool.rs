//! Reference checks (`check_references`).

use serde::Deserialize;
use serde_json::{Value, json};

use crate::analysis::reference_check;
use crate::savings::MetaBuilder;
use crate::service::CodeService;
use crate::service::agent_v1::args::ContentRef;
use crate::service::args::parse_args;
use crate::service::helpers::ToolError;

impl CodeService {
    pub(in crate::service) fn check_references(&self, v: Value) -> anyhow::Result<Value> {
        #[derive(Deserialize)]
        struct Args {
            source_id: String,
            #[serde(default)]
            identifier: Option<String>,
            #[serde(default)]
            identifiers: Option<Vec<String>>,
            #[serde(default)]
            refs: Option<Vec<ContentRef>>,
            #[serde(default = "default_search_content")]
            search_content: bool,
        }
        fn default_search_content() -> bool {
            true
        }
        let args: Args = parse_args(v)?;
        let timer = MetaBuilder::new(0);

        let store = match self.ensure_repo_store(&args.source_id, None) {
            Ok(s) => s,
            Err(e) => return self.error_response(&timer, e),
        };

        if args.identifier.is_some() && args.identifiers.is_some() {
            return self.error_response(
                &timer,
                ToolError {
                    code: "INVALID_REQUEST",
                    message: "Cannot provide both 'identifier' and 'identifiers'.".into(),
                    detail: json!({}),
                },
            );
        }

        if let Some(ref identifiers) = args.identifiers {
            let data =
                reference_check::check_identifiers_batch(&store, identifiers, args.search_content)?;
            return self.success_response(&timer, data);
        }

        if let Some(ref identifier) = args.identifier {
            let data = reference_check::check_single_identifier(
                &store,
                identifier.as_str(),
                args.search_content,
            )?;
            return self.success_response(&timer, data);
        }

        if args.refs.is_none() {
            return self.error_response(
                &timer,
                ToolError {
                    code: "INVALID_REQUEST",
                    message: "check_references requires 'identifier', 'identifiers', or 'refs'."
                        .into(),
                    detail: json!({}),
                },
            );
        }

        let refs: Vec<(String, String)> = args
            .refs
            .unwrap_or_default()
            .into_iter()
            .map(|r| {
                let id = r.stable_id.or(r.id).unwrap_or_default();
                (r.kind, id)
            })
            .collect();
        let data = reference_check::check_refs_existence(&store, &refs)?;
        self.success_response(&timer, data)
    }
}
