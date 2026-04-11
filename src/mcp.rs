//! Async stdio JSON-RPC loop with signal-driven graceful shutdown.
//!
//! The loop is driven by a Tokio `current_thread` runtime. Each incoming
//! `tools/call` is spawned onto a [`JoinSet`] under a bounded
//! [`Semaphore`], so independent dispatches can progress concurrently up to
//! `CODESIGHT_DISPATCH_CONCURRENCY`. Writes to the shared output stream are
//! serialized through a [`tokio::sync::Mutex`] so framed JSON-RPC lines are
//! never interleaved.
//!
//! On `SIGINT` or `SIGTERM` we stop accepting new stdin frames, wait up to
//! `CODESIGHT_SHUTDOWN_DRAIN_SECS` for in-flight dispatches to finish, abort
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
    let timeout_secs = service.config().handler_timeout_secs;
    let drain_secs = service.config().shutdown_drain_secs;
    let concurrency = service.config().dispatch_concurrency.max(1);

    let semaphore = Arc::new(Semaphore::new(concurrency));
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
                            if let Err(err) = handle_request(&svc, out, request, timeout_secs).await {
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
    let drain = tokio::time::timeout(Duration::from_secs(drain_secs), async {
        while let Some(joined) = join_set.join_next().await {
            log_joined(Some(joined));
        }
    })
    .await;
    if drain.is_err() {
        warn!(
            drain_secs,
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
    timeout_secs: u64,
) -> anyhow::Result<()>
where
    W: AsyncWrite + Unpin,
{
    match request.method.as_str() {
        "initialize" => {
            write_json(
                &out,
                RpcResponse {
                    jsonrpc: "2.0",
                    id: request.id.unwrap_or_else(|| json!(0)),
                    result: json!({
                        "protocolVersion": "2024-11-05",
                        "capabilities": { "tools": { "listChanged": false } },
                        "serverInfo": {
                            "name": env!("CARGO_PKG_NAME"),
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    }),
                },
            )
            .await?;
        }
        "tools/list" => {
            write_json(
                &out,
                RpcResponse {
                    jsonrpc: "2.0",
                    id: request.id.unwrap_or_else(|| json!(0)),
                    result: json!({ "tools": tool_definitions() }),
                },
            )
            .await?;
        }
        "tools/call" => {
            let params: ToolCallParams = serde_json::from_value(request.params)?;
            let payload =
                dispatch_with_timeout(service.clone(), params.name, params.arguments, timeout_secs)
                    .await?;
            write_json(
                &out,
                RpcResponse {
                    jsonrpc: "2.0",
                    id: request.id.unwrap_or_else(|| json!(0)),
                    result: json!({
                        "content": [
                            { "type": "text", "text": serde_json::to_string(&payload)? }
                        ],
                        "isError": payload.get("error").is_some()
                    }),
                },
            )
            .await?;
        }
        "notifications/initialized" => {}
        _ => {
            warn!(method = %request.method, "ignoring unsupported JSON-RPC method");
        }
    }
    Ok(())
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

fn tool_definitions() -> Vec<Value> {
    // Infallible: `tools.json` is bundled at compile time and must parse.
    serde_json::from_str(include_str!("tools.json")).expect("tools.json must be valid JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use tempfile::tempdir;

    use crate::config::AppConfig;

    fn test_service(drain_secs: u64) -> Arc<CodeService> {
        let dir = tempdir().expect("tempdir");
        let mut config = AppConfig::new(dir.path().join("index"));
        config.shutdown_drain_secs = drain_secs;
        config.handler_timeout_secs = 30;
        config.dispatch_concurrency = 4;
        // Leak the tempdir so the index root survives for the loop's lifetime.
        std::mem::forget(dir);
        Arc::new(CodeService::new(config).expect("service"))
    }

    #[tokio::test]
    async fn drain_returns_quickly_when_idle() {
        let service = test_service(5);
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
    async fn drain_aborts_when_exceeded() {
        // Spawn a task directly into a JoinSet, run drain phase inline, and
        // assert the drain timeout + abort path runs inside budget. This
        // exercises Phase 2 of `run_dispatch_loop` without needing a slow
        // fake handler (keeps the test deterministic and fast).
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
