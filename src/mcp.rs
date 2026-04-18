//! Async stdio JSON-RPC loop with signal-driven graceful shutdown.
//!
//! The loop is driven by a Tokio `current_thread` runtime. Each incoming
//! `tools/call` is spawned onto a [`JoinSet`] under a bounded
//! [`Semaphore`], so independent dispatches can progress concurrently up to
//! [`DISPATCH_CONCURRENCY`]. Writes to the shared output stream are
//! serialized through a [`tokio::sync::Mutex`] so framed JSON-RPC lines are
//! never interleaved.
//!
//! On `SIGINT` or `SIGTERM` we stop accepting new stdin frames, wait up to
//! [`SHUTDOWN_DRAIN_SECS`] for in-flight dispatches to finish, abort
//! anything still running past the budget, passive-checkpoint all known WAL
//! files, and return cleanly.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;
use tracing::{info, warn};

use crate::config::defaults::{DISPATCH_CONCURRENCY, SHUTDOWN_DRAIN_SECS};
use crate::service::{CodeService, dispatch_timeout::dispatch_with_timeout};

type SharedWriter<W> = Arc<Mutex<W>>;

#[derive(Debug, Deserialize)]
struct RpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Deserialize)]
struct ToolCallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

#[derive(Debug, Serialize)]
struct RpcResponse<T> {
    jsonrpc: &'static str,
    id: Value,
    result: T,
}

#[derive(Debug, Serialize)]
struct RpcErrorResponse {
    jsonrpc: &'static str,
    id: Value,
    error: RpcError,
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

/// JSON-RPC 2.0 reserved error codes we use.
const RPC_METHOD_NOT_FOUND: i32 = -32601;
const RPC_INVALID_PARAMS: i32 = -32602;
const RPC_INTERNAL_ERROR: i32 = -32603;

/// Synchronous entry point: builds a `current_thread` runtime and drives the
/// async stdio loop on it. Blocking handler work continues to run on the
/// runtime's blocking pool, so synchronous code keeps working unchanged.
pub fn run_stdio(service: Arc<CodeService>) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .context("failed to build tokio runtime")?;
    runtime.block_on(run_stdio_async(service))
}

/// Async stdio loop. Public so tests and future embedders can drive it on
/// their own runtime without paying for a nested one.
pub async fn run_stdio_async(service: Arc<CodeService>) -> anyhow::Result<()> {
    let reader = BufReader::new(tokio::io::stdin());
    let writer = tokio::io::stdout();
    run_dispatch_loop(service, reader, writer, shutdown_signal()).await
}

/// Drive the dispatch loop on a user-provided reader/writer. Extracted from
/// [`run_stdio_async`] so tests can wire fake stdin/stdout sources without
/// touching real file descriptors.
pub async fn run_dispatch_loop<R, W, S>(
    service: Arc<CodeService>,
    reader: R,
    writer: W,
    shutdown: S,
) -> anyhow::Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
    S: Future<Output = ()>,
{
    let semaphore = Arc::new(Semaphore::new(DISPATCH_CONCURRENCY));
    let writer: SharedWriter<W> = Arc::new(Mutex::new(writer));
    let mut join_set: JoinSet<()> = JoinSet::new();
    let mut lines = reader.lines();
    let mut shutdown = Box::pin(shutdown);

    // ── Phase 1: accept and spawn until shutdown or EOF ─────────────────
    let mut stop_accepting = false;
    while !stop_accepting {
        tokio::select! {
            biased;
            _ = &mut shutdown => {
                info!("shutdown signal received, draining stdio loop");
                stop_accepting = true;
            }
            joined = join_set.join_next(), if !join_set.is_empty() => {
                log_joined(joined);
            }
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if line.trim().is_empty() {
                            continue;
                        }
                        let request: RpcRequest = serde_json::from_str(&line)
                            .with_context(|| format!("invalid JSON-RPC line: {line}"))?;
                        let permit = semaphore.clone().acquire_owned().await?;
                        let svc = service.clone();
                        let out = writer.clone();
                        join_set.spawn(async move {
                            let _permit = permit;
                            if let Err(err) = handle_request(&svc, out, request).await {
                                tracing::error!(error = %err, "dispatch task failed");
                            }
                        });
                    }
                    Ok(None) => { stop_accepting = true; }
                    Err(err) => return Err(err.into()),
                }
            }
        }
    }

    // ── Phase 2: drain with timeout, then abort ─────────────────────────
    let drain = tokio::time::timeout(Duration::from_secs(SHUTDOWN_DRAIN_SECS), async {
        while let Some(joined) = join_set.join_next().await {
            log_joined(Some(joined));
        }
    })
    .await;
    if drain.is_err() {
        warn!(
            drain_secs = SHUTDOWN_DRAIN_SECS,
            "shutdown drain exceeded budget, aborting in-flight dispatches"
        );
        join_set.abort_all();
        while let Some(joined) = join_set.join_next().await {
            log_joined(Some(joined));
        }
    }

    service.config().checkpoint_all_wal();
    info!("shutdown_complete");
    Ok(())
}

fn log_joined(joined: Option<Result<(), tokio::task::JoinError>>) {
    if let Some(Err(err)) = joined
        && !err.is_cancelled()
    {
        tracing::error!(error = %err, "dispatch task panicked");
    }
}

async fn handle_request<W>(
    service: &Arc<CodeService>,
    out: SharedWriter<W>,
    request: RpcRequest,
) -> anyhow::Result<()>
where
    W: AsyncWrite + Unpin,
{
    // Every path below MUST produce a response for non-notification requests.
    // Returning `Err` without writing causes the MCP client to hang until its
    // own watchdog fires (observed as 120s "tool call timed out" errors).
    let id = request.id.clone().unwrap_or_else(|| json!(0));
    let method = request.method.clone();
    let is_notification = matches!(method.as_str(), "notifications/initialized")
        || method.starts_with("notifications/");
    match dispatch_method(service, request).await {
        Ok(Some(result)) => {
            write_json(
                &out,
                RpcResponse {
                    jsonrpc: "2.0",
                    id,
                    result,
                },
            )
            .await?;
        }
        Ok(None) => {
            // Notification — JSON-RPC requires no response.
        }
        Err(err) if is_notification => {
            warn!(method = %method, error = %err, "notification handler failed");
        }
        Err(err) => {
            let (code, data) = classify_error(&method, &err);
            let payload = RpcErrorResponse {
                jsonrpc: "2.0",
                id,
                error: RpcError {
                    code,
                    message: format!("{err:#}"),
                    data,
                },
            };
            if let Err(write_err) = write_json(&out, payload).await {
                warn!(
                    method = %method,
                    error = %err,
                    write_error = %write_err,
                    "failed to send JSON-RPC error response"
                );
            }
        }
    }
    Ok(())
}

/// Map an internal error to a JSON-RPC error code + `data` payload so clients
/// can distinguish method-not-found, bad params, timeouts, and unexpected
/// internal failures at the protocol level.
fn classify_error(method: &str, err: &anyhow::Error) -> (i32, Option<Value>) {
    if err
        .downcast_ref::<MethodNotFound>()
        .map(|m| m.method == method)
        .unwrap_or(false)
    {
        return (
            RPC_METHOD_NOT_FOUND,
            Some(json!({ "method": method })),
        );
    }
    if err.downcast_ref::<InvalidParams>().is_some() || err.is::<serde_json::Error>() {
        return (RPC_INVALID_PARAMS, Some(json!({ "method": method })));
    }
    (
        RPC_INTERNAL_ERROR,
        Some(json!({
            "method": method,
            "cause": err.to_string(),
        })),
    )
}

#[derive(Debug, thiserror::Error)]
#[error("unsupported JSON-RPC method: {method}")]
struct MethodNotFound {
    method: String,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid params for {method}: {reason}")]
struct InvalidParams {
    method: String,
    reason: String,
}

/// Returns `Ok(Some(result))` for request methods, `Ok(None)` for notifications.
/// Any `Err` is routed through `classify_error` to become a JSON-RPC error reply.
async fn dispatch_method(
    service: &Arc<CodeService>,
    request: RpcRequest,
) -> anyhow::Result<Option<Value>> {
    match request.method.as_str() {
        "initialize" => Ok(Some(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": {
                "name": env!("CARGO_PKG_NAME"),
                "version": env!("CARGO_PKG_VERSION")
            }
        }))),
        "tools/list" => Ok(Some(json!({
            "tools": crate::service::agent_v1::validate::tool_definitions()
        }))),
        "tools/call" => {
            let params: ToolCallParams =
                serde_json::from_value(request.params).map_err(|e| InvalidParams {
                    method: "tools/call".into(),
                    reason: e.to_string(),
                })?;
            let payload =
                dispatch_with_timeout(service.clone(), params.name, params.arguments, None)
                    .await?;
            Ok(Some(json!({
                "content": [
                    { "type": "text", "text": serde_json::to_string(&payload)? }
                ],
                "isError": payload.get("error").is_some()
            })))
        }
        "notifications/initialized" => Ok(None),
        other if other.starts_with("notifications/") => Ok(None),
        other => Err(MethodNotFound { method: other.to_string() }.into()),
    }
}

async fn write_json<W, T>(out: &SharedWriter<W>, value: T) -> anyhow::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut buf = serde_json::to_vec(&value)?;
    buf.push(b'\n');
    let mut guard = out.lock().await;
    guard.write_all(&buf).await?;
    guard.flush().await?;
    Ok(())
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    match signal(SignalKind::terminate()) {
        Ok(mut term) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
        }
        Err(err) => {
            warn!(error = %err, "failed to install SIGTERM handler, falling back to ctrl_c");
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

    use crate::config::AppConfig;

    fn test_service() -> Arc<CodeService> {
        let dir = tempdir().expect("tempdir");
        let config = AppConfig::new(dir.path().join("index"));
        // Leak the tempdir so the index root survives for the loop's lifetime.
        std::mem::forget(dir);
        let service = Arc::new(CodeService::new(config).expect("service"));
        service.install_arc_handle();
        service
    }

    /// Drive `run_dispatch_loop` with a single JSON-RPC line and collect the
    /// response(s) it writes before EOF-triggered shutdown.
    async fn roundtrip(request_line: &str) -> Vec<Value> {
        let service = test_service();
        let (mut client_in, server_in) = duplex(64 * 1024);
        let (server_out, mut client_out) = duplex(64 * 1024);

        client_in.write_all(request_line.as_bytes()).await.unwrap();
        client_in.write_all(b"\n").await.unwrap();
        drop(client_in);

        let reader = tokio::io::BufReader::new(server_in);
        // Reader task: collect everything the server writes before EOF.
        let drain = tokio::spawn(async move {
            let mut buf = Vec::new();
            client_out.read_to_end(&mut buf).await.unwrap();
            buf
        });
        // `async {}` would resolve immediately and bias-lose every select —
        // use `pending::<()>()` so the loop exits only on stdin EOF.
        run_dispatch_loop(service, reader, server_out, std::future::pending::<()>())
            .await
            .expect("dispatch loop ok");
        let buf = drain.await.expect("drain joined");

        String::from_utf8(buf)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("valid JSON-RPC frame"))
            .collect()
    }

    #[tokio::test]
    async fn drain_returns_quickly_when_idle() {
        let service = test_service();
        let reader = tokio::io::BufReader::new(&b""[..]);
        let writer = tokio::io::sink();
        let start = Instant::now();
        run_dispatch_loop(service, reader, writer, async {})
            .await
            .expect("loop");
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "idle drain should be near-instant, took {:?}",
            start.elapsed()
        );
    }

    #[tokio::test]
    async fn unknown_method_returns_method_not_found_not_hang() {
        let frames = roundtrip(
            r#"{"jsonrpc":"2.0","id":42,"method":"definitely_not_real","params":{}}"#,
        )
        .await;
        assert_eq!(frames.len(), 1, "client must see exactly one response frame");
        let f = &frames[0];
        assert_eq!(f["id"], json!(42));
        assert_eq!(f["error"]["code"], json!(RPC_METHOD_NOT_FOUND));
        assert_eq!(f["error"]["data"]["method"], json!("definitely_not_real"));
    }

    #[tokio::test]
    async fn malformed_tools_call_params_returns_invalid_params_not_hang() {
        // `params` is a string where an object is expected — serde fails;
        // pre-fix, `handle_request` swallowed the error via `?` and the
        // client hung until its own watchdog.
        let frames = roundtrip(
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":"not-an-object"}"#,
        )
        .await;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["id"], json!(7));
        assert_eq!(frames[0]["error"]["code"], json!(RPC_INVALID_PARAMS));
    }

    #[tokio::test]
    async fn unknown_notification_is_silently_accepted() {
        let frames = roundtrip(
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{}}"#,
        )
        .await;
        assert!(frames.is_empty(), "notifications must not receive a reply");
    }

    #[tokio::test(start_paused = true)]
    async fn drain_aborts_when_exceeded() {
        // Spawn a task directly into a JoinSet, run drain phase inline, and
        // assert the drain timeout + abort path runs inside budget. With
        // `start_paused`, tokio uses virtual time so the 30s sleep + 1s drain
        // timeout resolve instantly; we still assert on `Instant::now()`
        // (wall clock) to verify the abort path finishes promptly in real time.
        let mut join_set: JoinSet<()> = JoinSet::new();
        join_set.spawn(async {
            tokio::time::sleep(Duration::from_secs(30)).await;
        });
        let drain_secs = 1u64;
        let start = Instant::now();
        let drain = tokio::time::timeout(Duration::from_secs(drain_secs), async {
            while (join_set.join_next().await).is_some() {}
        })
        .await;
        assert!(drain.is_err(), "drain should time out");
        join_set.abort_all();
        while (join_set.join_next().await).is_some() {}
        assert!(
            start.elapsed() < Duration::from_secs(drain_secs + 2),
            "abort path should finish promptly after drain timeout, took {:?}",
            start.elapsed()
        );
    }
}
