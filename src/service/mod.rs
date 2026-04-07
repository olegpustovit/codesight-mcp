mod agent_v1;
mod args;
mod code_tools;
mod doc_tools;
mod helpers;

use serde_json::{Value, json};

use crate::{
    config::AppConfig,
    extractors::ExtractorRegistry,
    savings::{MetaBuilder, SavingsLedger},
};
use helpers::{ToolError, attach_meta};

const MAX_QUERY_LEN: usize = 1000;

pub struct CodeService {
    config: AppConfig,
    extractors: ExtractorRegistry,
    ledger: SavingsLedger,
}

impl CodeService {
    pub fn new(config: AppConfig) -> anyhow::Result<Self> {
        config.ensure_layout()?;
        let ledger = SavingsLedger::new(config.savings_file());
        Ok(Self {
            config,
            extractors: ExtractorRegistry::default(),
            ledger,
        })
    }

    pub fn dispatch(&self, tool_name: &str, arguments: Value) -> anyhow::Result<Value> {
        agent_v1::dispatch(self, tool_name, arguments)
    }

    pub(super) fn success_response(
        &self,
        timer: &MetaBuilder,
        body: Value,
    ) -> anyhow::Result<Value> {
        self.success_response_with_warnings(timer, body, Vec::new())
    }

    pub(super) fn success_response_with_warnings(
        &self,
        timer: &MetaBuilder,
        data: Value,
        warnings: Vec<Value>,
    ) -> anyhow::Result<Value> {
        self.success_response_with_meta_extra(timer, data, warnings, [])
    }

    pub(super) fn success_response_with_meta_extra(
        &self,
        timer: &MetaBuilder,
        data: Value,
        warnings: Vec<Value>,
        meta_extra: impl IntoIterator<Item = (String, Value)>,
    ) -> anyhow::Result<Value> {
        let body = json!({
            "ok": true,
            "data": data,
            "warnings": warnings,
        });
        let mut meta_val = serde_json::to_value(timer.success(&body, &self.ledger)?)?;
        if let Value::Object(ref mut map) = meta_val {
            for (k, v) in meta_extra {
                map.insert(k, v);
            }
        }
        attach_meta(body, meta_val)
    }

    pub(super) fn error_response(
        &self,
        timer: &MetaBuilder,
        error: ToolError,
    ) -> anyhow::Result<Value> {
        let meta = timer.error(&self.ledger)?;
        let body = json!({
            "ok": false,
            "data": { "detail": error.detail.clone() },
            "warnings": [],
            "error": {
                "code": error.code,
                "message": error.message,
                "detail": error.detail,
            },
        });
        attach_meta(body, serde_json::to_value(meta)?)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use super::CodeService;
    use crate::config::AppConfig;

    #[test]
    fn indexes_searches_and_retrieves_python_symbols() {
        let workspace = tempdir().expect("tempdir");
        let repo_root = workspace.path().join("mini-repo");
        fs::create_dir_all(&repo_root).unwrap();
        fs::write(
            repo_root.join("api.py"),
            r#"
MAX_RETRIES = 5

class SessionManager:
    """Database session dependency container."""

    def get_db(self):
        """Dependency injection database session."""
        return "db"
"#,
        )
        .unwrap();

        let index_root = workspace.path().join("index");
        let service = CodeService::new(AppConfig::new(index_root)).expect("service");

        let indexed = service
            .dispatch(
                "ingest_sources",
                json!({
                    "path": repo_root.to_string_lossy(),
                    "domains": "code",
                    "incremental": true,
                }),
            )
            .expect("indexing");
        assert!(indexed["ok"].as_bool().unwrap());
        let repo = indexed["data"]["source_id"].as_str().unwrap().to_string();

        let searched = service
            .dispatch(
                "search_catalog",
                json!({
                    "source_id": repo,
                    "query": "dependency injection database session",
                    "domain": "code",
                    "limit": 5,
                }),
            )
            .expect("searching");
        assert!(searched["ok"].as_bool().unwrap());
        let first_id = searched["data"]["items"][0]["ref"]["stable_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(first_id, "api.py::SessionManager.get_db#method");

        let retrieved = service
            .dispatch(
                "read_content",
                json!({
                    "source_id": repo,
                    "refs": [{ "kind": "symbol", "stable_id": first_id }],
                }),
            )
            .expect("retrieving");
        assert!(retrieved["ok"].as_bool().unwrap());
        let src = retrieved["data"]["items"][0]["content"]["source"]
            .as_str()
            .unwrap();
        assert!(src.contains("Dependency injection database session."));
    }
}
