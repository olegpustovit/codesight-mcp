//! Per-handler timeout wrapper.
//!
//! MCP handlers are synchronous — they run as blocking work on a Tokio
//! blocking thread. `dispatch_with_timeout` waits on that work with a bounded
//! deadline. When the deadline elapses the caller receives a
//! [`ToolError::Transient`] tagged `TIMEOUT`; the underlying work is not
//! cancelled (Rust threads are not cooperatively interruptible), but the
//! operator surface stays responsive and categorized.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::task;
use tokio::time::timeout;

use super::CodeService;
use super::helpers::{ToolError, ToolErrorCategory};

/// Run `service.dispatch(tool_name, arguments)` under a wall-clock deadline.
///
/// A `timeout_ms` of `0` disables the deadline and waits forever.
pub async fn dispatch_with_timeout(
    service: Arc<CodeService>,
    tool_name: String,
    arguments: Value,
    timeout_ms: u64,
) -> anyhow::Result<Value> {
    let tool_label = tool_name.clone();
    let work = async move {
        task::spawn_blocking(move || service.dispatch(&tool_name, arguments))
            .await
            .map_err(|join_err| {
                anyhow::anyhow!(ToolError::new(
                    ToolErrorCategory::Internal,
                    "DISPATCH_PANIC",
                    format!("dispatch worker crashed: {join_err}"),
                    json!({}),
                ))
            })?
    };
    run_with_deadline(work, timeout_ms, &tool_label).await
}

/// Deadline helper extracted for hermetic tests: resolves `work` normally, or
/// yields a categorized `TIMEOUT` error if the deadline elapses first.
pub(crate) async fn run_with_deadline<F>(
    work: F,
    timeout_ms: u64,
    tool_name: &str,
) -> anyhow::Result<Value>
where
    F: Future<Output = anyhow::Result<Value>>,
{
    if timeout_ms == 0 {
        return work.await;
    }
    match timeout(Duration::from_millis(timeout_ms), work).await {
        Ok(inner) => inner,
        Err(_) => Err(anyhow::anyhow!(ToolError::new(
            ToolErrorCategory::Transient,
            "TIMEOUT",
            format!("handler '{tool_name}' exceeded {timeout_ms}ms timeout"),
            json!({
                "tool": tool_name,
                "budget_ms": timeout_ms,
                "hint": "narrow the query (scope, unit_kinds, limit) — the handler budget is a compile-time constant.",
            }),
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::super::helpers::{ToolError, ToolErrorCategory};
    use super::run_with_deadline;

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn elapsed_deadline_returns_transient_timeout() {
        // Give the test a 50 ms budget and push the slow work past it.
        // `start_paused` makes tokio use virtual time — `sleep` auto-advances
        // whenever no real I/O is pending, so the test finishes in microseconds
        // of wall-clock time.
        let slow = async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok(json!({}))
        };
        let err = run_with_deadline(slow, 50, "search_catalog")
            .await
            .expect_err("expected timeout");
        let tool_err = err.downcast::<ToolError>().expect("tool error");
        assert_eq!(tool_err.category, ToolErrorCategory::Transient);
        assert_eq!(tool_err.code, "TIMEOUT");
        assert_eq!(tool_err.detail["tool"], json!("search_catalog"));
        assert_eq!(tool_err.detail["budget_ms"], json!(50));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn zero_timeout_disables_deadline() {
        let fast = async { Ok(json!({ "ok": true })) };
        let out = run_with_deadline(fast, 0, "search_catalog")
            .await
            .expect("resolves");
        assert_eq!(out["ok"].as_bool(), Some(true));
    }
}
