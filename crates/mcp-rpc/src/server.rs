//! Async stdio JSON-RPC server with signal-driven graceful shutdown.
//!
//! The loop is driven by a Tokio `current_thread` runtime. Each incoming
//! `tools/call` is spawned onto a [`JoinSet`] under a bounded [`Semaphore`],
//! so independent dispatches can progress concurrently up to
//! [`DispatchConfig::concurrency`]. Writes to the shared output stream are
//! serialized through a [`tokio::sync::Mutex`] so framed JSON-RPC lines are
//! never interleaved.
//!
//! On `SIGINT` or `SIGTERM` we stop accepting new stdin frames, wait up to
//! [`DispatchConfig::shutdown_drain`] for in-flight dispatches to finish,
//! abort anything still running past the budget, run the optional shutdown
//! hook on the dispatcher, and return cleanly.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;
use tracing::{info, warn};

type SharedWriter<W> = Arc<Mutex<W>>;

/// Contract between the RPC layer and a host engine.
///
/// Implementors translate JSON-RPC method calls into domain work. The trait
/// is intentionally narrow: everything the RPC server needs is here, and
/// everything else (persistence, background jobs, config) stays private to
/// the implementor.
#[async_trait]
pub trait Dispatcher: Send + Sync + 'static {
    /// Server name for `initialize` / `serverInfo`.
    fn server_name(&self) -> &str;

    /// Server version for `initialize` / `serverInfo`.
    fn server_version(&self) -> &str;

    /// Tool catalog for `tools/list`. Return an already-JSON-serialized tool
    /// array — the shape comes from the MCP spec.
    fn tool_definitions(&self) -> Value;

    /// Handle one `tools/call`. Return the payload that becomes the
    /// JSON-RPC `result` (wrapped into `{content: [{type:text,text}]}` by the
    /// server).
    async fn call_tool(
        self: Arc<Self>,
        name: String,
        arguments: Value,
        id: Option<Value>,
    ) -> anyhow::Result<Value>;

    /// Optional cleanup after the drain phase: checkpoint, flush, sync.
    /// Default is a no-op.
    async fn shutdown(&self) {}
}

#[derive(Debug, Clone, Copy)]
pub struct DispatchConfig {
    /// Bound on in-flight `tools/call` dispatches.
    pub concurrency: usize,
    /// How long shutdown waits for in-flight dispatches before aborting.
    pub shutdown_drain: Duration,
}

impl DispatchConfig {
    pub const fn new(concurrency: usize, shutdown_drain: Duration) -> Self {
        Self { concurrency, shutdown_drain }
    }
}

impl Default for DispatchConfig {
    fn default() -> Self {
        Self::new(4, Duration::from_secs(5))
    }
}

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

const RPC_METHOD_NOT_FOUND: i32 = -32601;
const RPC_INVALID_PARAMS: i32 = -32602;
const RPC_INTERNAL_ERROR: i32 = -32603;

/// Synchronous entry point: builds a `current_thread` runtime and drives the
/// async stdio loop on it.
pub fn run_stdio<D: Dispatcher>(
    dispatcher: Arc<D>,
    config: DispatchConfig,
) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .context("failed to build tokio runtime")?;
    runtime.block_on(run_stdio_async(dispatcher, config))
}

/// Async stdio loop. Public so tests and future embedders can drive it on
/// their own runtime without paying for a nested one.
pub async fn run_stdio_async<D: Dispatcher>(
    dispatcher: Arc<D>,
    config: DispatchConfig,
) -> anyhow::Result<()> {
    let reader = BufReader::new(tokio::io::stdin());
    let writer = tokio::io::stdout();
    run_dispatch_loop(dispatcher, config, reader, writer, shutdown_signal()).await
}

/// Drive the dispatch loop on a user-provided reader/writer.
pub async fn run_dispatch_loop<D, R, W, S>(
    dispatcher: Arc<D>,
    config: DispatchConfig,
    reader: R,
    writer: W,
    shutdown: S,
) -> anyhow::Result<()>
where
    D: Dispatcher,
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
    S: Future<Output = ()>,
{
    let semaphore = Arc::new(Semaphore::new(config.concurrency));
    let writer: SharedWriter<W> = Arc::new(Mutex::new(writer));
    let mut join_set: JoinSet<()> = JoinSet::new();
    let mut lines = reader.lines();
    let mut shutdown = Box::pin(shutdown);

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
                        let disp = dispatcher.clone();
                        let out = writer.clone();
                        join_set.spawn(async move {
                            let _permit = permit;
                            if let Err(err) = handle_request(disp, out, request).await {
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

    let drain = tokio::time::timeout(config.shutdown_drain, async {
        while let Some(joined) = join_set.join_next().await {
            log_joined(Some(joined));
        }
    })
    .await;
    if drain.is_err() {
        warn!(
            drain_secs = config.shutdown_drain.as_secs(),
            "shutdown drain exceeded budget, aborting in-flight dispatches"
        );
        join_set.abort_all();
        while let Some(joined) = join_set.join_next().await {
            log_joined(Some(joined));
        }
    }

    dispatcher.shutdown().await;
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

async fn handle_request<D, W>(
    dispatcher: Arc<D>,
    out: SharedWriter<W>,
    request: RpcRequest,
) -> anyhow::Result<()>
where
    D: Dispatcher,
    W: AsyncWrite + Unpin,
{
    let id = request.id.clone().unwrap_or_else(|| json!(0));
    let method = request.method.clone();
    let is_notification = method.starts_with("notifications/");
    match dispatch_method(dispatcher, request).await {
        Ok(Some(result)) => {
            write_json(&out, RpcResponse { jsonrpc: "2.0", id, result }).await?;
        }
        Ok(None) => {}
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

fn classify_error(method: &str, err: &anyhow::Error) -> (i32, Option<Value>) {
    if err
        .downcast_ref::<MethodNotFound>()
        .map(|m| m.method == method)
        .unwrap_or(false)
    {
        return (RPC_METHOD_NOT_FOUND, Some(json!({ "method": method })));
    }
    if err.downcast_ref::<InvalidParams>().is_some() || err.is::<serde_json::Error>() {
        return (RPC_INVALID_PARAMS, Some(json!({ "method": method })));
    }
    (
        RPC_INTERNAL_ERROR,
        Some(json!({ "method": method, "cause": err.to_string() })),
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

async fn dispatch_method<D: Dispatcher>(
    dispatcher: Arc<D>,
    request: RpcRequest,
) -> anyhow::Result<Option<Value>> {
    match request.method.as_str() {
        "initialize" => Ok(Some(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": {
                "name": dispatcher.server_name(),
                "version": dispatcher.server_version(),
            }
        }))),
        "tools/list" => Ok(Some(json!({ "tools": dispatcher.tool_definitions() }))),
        "tools/call" => {
            let id = request.id.clone();
            let params: ToolCallParams =
                serde_json::from_value(request.params).map_err(|e| InvalidParams {
                    method: "tools/call".into(),
                    reason: e.to_string(),
                })?;
            let payload = dispatcher
                .clone()
                .call_tool(params.name, params.arguments, id)
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
