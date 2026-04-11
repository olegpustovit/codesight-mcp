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
/// A `timeout_secs` of `0` disables the deadline and waits forever.
pub async fn dispatch_with_timeout(
    service: Arc<CodeService>,
    tool_name: String,
    arguments: Value,
    timeout_secs: u64,
) -> anyhow::Result<Value> {
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
    run_with_deadline(work, timeout_secs).await
}

/// Deadline helper extracted for hermetic tests: resolves `work` normally, or
/// yields a categorized `TIMEOUT` error if the deadline elapses first.
pub(crate) async fn run_with_deadline<F>(work: F, timeout_secs: u64) -> anyhow::Result<Value>
where
    F: Future<Output = anyhow::Result<Value>>,
{
    if timeout_secs == 0 {
        return work.await;
    }
    match timeout(Duration::from_secs(timeout_secs), work).await {
        Ok(inner) => inner,
        Err(_) => Err(anyhow::anyhow!(ToolError::new(
            ToolErrorCategory::Transient,
            "TIMEOUT",
            format!("handler exceeded {timeout_secs}s timeout"),
            json!({ "timeout_secs": timeout_secs }),
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
        // `run_with_deadline` takes whole-second budgets, so give the test 1 s
        // budget and push the slow work past it. `start_paused` makes tokio
        // use virtual time — `sleep` auto-advances whenever no real I/O is
        // pending, so the test finishes in microseconds of wall-clock time.
        let slow = async {
            tokio::time::sleep(Duration::from_secs(3)).await;
            Ok(json!({}))
        };
        let err = run_with_deadline(slow, 1)
            .await
            .expect_err("expected timeout");
        let tool_err = err.downcast::<ToolError>().expect("tool error");
        assert_eq!(tool_err.category, ToolErrorCategory::Transient);
        assert_eq!(tool_err.code, "TIMEOUT");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn zero_timeout_disables_deadline() {
        let fast = async { Ok(json!({ "ok": true })) };
        let out = run_with_deadline(fast, 0).await.expect("resolves");
        assert_eq!(out["ok"].as_bool(), Some(true));
    }
}
