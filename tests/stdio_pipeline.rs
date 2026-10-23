//! Process-level stdio framing regression: no sleeps between notification and request.
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

struct Server(Child);

struct ObservedStdout(std::process::ChildStdout, Arc<Mutex<Vec<u8>>>);
impl Read for ObservedStdout {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.0.read(buffer)?;
        self.1.lock().unwrap().extend_from_slice(&buffer[..count]);
        Ok(count)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn initialized_notification_followed_by_request_does_not_stall() {
    pipeline(false);
}

#[cfg(all(target_os = "linux", debug_assertions))]
#[test]
fn blocking_stdio_initialized_notification_followed_by_request_does_not_stall() {
    pipeline(true);
}

fn pipeline(blocking_stdio: bool) {
    let workspace = tempfile::tempdir().unwrap();
    for iteration in 0..25 {
        let mut stderr = tempfile::tempfile().unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_dowser-mcp"));
        command.env_remove("DOWSER_TEST_BLOCKING_STDIO");
        if blocking_stdio {
            command.env("DOWSER_TEST_BLOCKING_STDIO", "1");
        }
        let mut server = Server(
            command
                .args(["serve", "--workspace", workspace.path().to_str().unwrap()])
                .env("DOWSER_INDEX_DIR", workspace.path().join("index"))
                .env(
                    "RUST_LOG",
                    "rmcp=trace,dowser_mcp=info,dowser_mcp::stdio_io=trace",
                )
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::from(stderr.try_clone().unwrap()))
                .spawn()
                .unwrap(),
        );
        let stdout = server.0.stdout.take().unwrap();
        let raw_stdout = Arc::new(Mutex::new(Vec::new()));
        let observed = ObservedStdout(stdout, raw_stdout.clone());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(observed).lines() {
                let response = line
                    .map_err(|e| e.to_string())
                    .and_then(|s| serde_json::from_str::<Value>(&s).map_err(|e| e.to_string()));
                if tx.send(response).is_err() {
                    break;
                }
            }
        });
        let mut stdin = server.0.stdin.take().unwrap();
        let mut prior_responses = Vec::new();
        let mut receive = |stage| {
            let response = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|error| {
                let status = server.0.try_wait();
                let _ = server.0.kill();
                let _ = server.0.wait();
                stderr.seek(SeekFrom::Start(0)).unwrap();
                let mut logs = String::new();
                stderr.read_to_string(&mut logs).unwrap();
                let bytes = raw_stdout.lock().unwrap();
                panic!(
                    "{stage}: {error}; iteration={iteration}; child_status={status:?}; \
                     prior_responses={prior_responses:?}; stdout_bytes={}; raw_stdout={:?}; stderr={logs}",
                    bytes.len(), String::from_utf8_lossy(&bytes)
                );
            }).unwrap();
            prior_responses.push(response.clone());
            response
        };
        writeln!(stdin, "{}", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"stdio-regression","version":"1"}
        }})).unwrap();
        stdin.flush().unwrap();
        assert_eq!(receive("initialize stalled")["id"], 1);
        for message in [
            json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        ] {
            writeln!(stdin, "{message}").unwrap();
            stdin.flush().unwrap();
        }
        let response = receive("pipelined tools/list stalled");
        assert_eq!(response["id"], 2);
        assert_eq!(response["result"]["tools"].as_array().unwrap().len(), 6);
    }
}
