//! Process-level stdio framing regression: no sleeps between notification and request.
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn initialized_notification_followed_by_request_does_not_stall() {
    let workspace = tempfile::tempdir().unwrap();
    for _ in 0..25 {
        let mut server = Server(
            Command::new(env!("CARGO_BIN_EXE_codesight-mcp"))
                .args(["serve", "--workspace", workspace.path().to_str().unwrap()])
                .env("CODESIGHT_INDEX_DIR", workspace.path().join("index"))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let stdout = server.0.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let response = line
                    .map_err(|e| e.to_string())
                    .and_then(|s| serde_json::from_str::<Value>(&s).map_err(|e| e.to_string()));
                if tx.send(response).is_err() {
                    break;
                }
            }
        });
        let stdin = server.0.stdin.as_mut().unwrap();
        writeln!(stdin, "{}", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"stdio-regression","version":"1"}
        }})).unwrap();
        stdin.flush().unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap()["id"],
            1
        );
        for message in [
            json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        ] {
            writeln!(stdin, "{message}").unwrap();
            stdin.flush().unwrap();
        }
        let response = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("pipelined tools/list stalled")
            .unwrap();
        assert_eq!(response["id"], 2);
        assert_eq!(response["result"]["tools"].as_array().unwrap().len(), 6);
    }
}
