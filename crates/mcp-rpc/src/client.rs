//! Minimal stdio MCP client: spawn an MCP server as a child process and call
//! its tools over JSON-RPC 2.0.
//!
//! Scope: one in-flight request at a time. No server-side batching,
//! streaming, or notifications beyond `notifications/initialized`.

use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

#[derive(Debug, Error)]
pub enum McpClientError {
    #[error("spawn MCP server: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("server closed stdout before responding")]
    ServerClosed,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("decode response: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("server returned JSON-RPC error: code={code} msg={message}")]
    RpcError { code: i32, message: String },
}

/// stdio MCP client. Keeps the child process alive for the lifetime of the
/// client; dropping it sends EOF to stdin and reaps the child.
pub struct McpClient {
    #[allow(dead_code)]
    child: Child,
    stdin: Mutex<ChildStdin>,
    stdout: Mutex<BufReader<ChildStdout>>,
    next_id: AtomicU64,
}

impl McpClient {
    /// Spawn the MCP server and run the `initialize` handshake.
    pub async fn spawn(program: &str, args: &[&str]) -> Result<Self, McpClientError> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(McpClientError::Spawn)?;
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = BufReader::new(child.stdout.take().expect("stdout piped"));
        let client = Self {
            child,
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(stdout),
            next_id: AtomicU64::new(1),
        };
        client
            .request::<Value>(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "mcp-rpc", "version": env!("CARGO_PKG_VERSION") }
                }),
            )
            .await?;
        client.notify("notifications/initialized", Value::Null).await?;
        Ok(client)
    }

    /// List the server's tool catalog.
    pub async fn list_tools(&self) -> Result<Value, McpClientError> {
        self.request("tools/list", Value::Null).await
    }

    /// Invoke a tool.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
    ) -> Result<Value, McpClientError> {
        self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
        .await
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, McpClientError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_frame(&payload).await?;
        let response = self.read_frame().await?;
        if let Some(err) = response.get("error") {
            return Err(McpClientError::RpcError {
                code: err.get("code").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                message: err
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
        let result = response
            .get("result")
            .cloned()
            .unwrap_or(Value::Null);
        Ok(serde_json::from_value(result)?)
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), McpClientError> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.write_frame(&payload).await
    }

    async fn write_frame(&self, value: &Value) -> Result<(), McpClientError> {
        let mut buf = serde_json::to_vec(value)?;
        buf.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(&buf).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn read_frame(&self) -> Result<Value, McpClientError> {
        let mut stdout = self.stdout.lock().await;
        let mut line = String::new();
        let n = stdout.read_line(&mut line).await?;
        if n == 0 {
            return Err(McpClientError::ServerClosed);
        }
        Ok(serde_json::from_str(&line)?)
    }
}
