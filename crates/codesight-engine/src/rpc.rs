//! Bridge from codesight engine to the generic `mcp-rpc` server. The engine
//! owns `CodeService` and knows how to wrap it as an `mcp_rpc::Dispatcher` so
//! the binary can hand it to `run_stdio`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::task;
use tokio::time::timeout;

use crate::config::BUILD_VERSION;
use crate::service::CodeService;
use crate::service::agent_v1::registry;
use crate::service::agent_v1::validate::tool_definitions;
use crate::service::helpers::{ToolError, ToolErrorCategory};

#[async_trait]
impl mcp_rpc::Dispatcher for CodeService {
    fn server_name(&self) -> &str {
        env!("CARGO_PKG_NAME")
    }

    fn server_version(&self) -> &str {
        BUILD_VERSION
    }

    fn tool_definitions(&self) -> Value {
        json!(tool_definitions())
    }

    async fn call_tool(
        self: Arc<Self>,
        name: String,
        arguments: Value,
        _id: Option<Value>,
    ) -> anyhow::Result<Value> {
        let tool = name.clone();
        let budget_ms = registry::timeout_ms(&tool);
        let svc = self.clone();
        let work = task::spawn_blocking(move || svc.dispatch(&name, arguments));
        let joined = if budget_ms == 0 {
            work.await
        } else {
            match timeout(Duration::from_millis(budget_ms), work).await {
                Ok(inner) => inner,
                Err(_) => {
                    return Err(anyhow::anyhow!(ToolError::new(
                        ToolErrorCategory::Transient,
                        "TIMEOUT",
                        format!("handler '{tool}' exceeded {budget_ms}ms timeout"),
                        json!({
                            "tool": tool,
                            "budget_ms": budget_ms,
                            "hint": "narrow the query (scope, unit_kinds, limit), or if a reindex is in progress wait for it to settle and retry.",
                        }),
                    )));
                }
            }
        };
        joined.map_err(|join_err| {
            anyhow::anyhow!(ToolError::new(
                ToolErrorCategory::Internal,
                "DISPATCH_PANIC",
                format!("dispatch worker crashed: {join_err}"),
                json!({ "tool": tool }),
            ))
        })?
    }

    async fn shutdown(&self) {
        self.config().checkpoint_all_wal();
    }
}
