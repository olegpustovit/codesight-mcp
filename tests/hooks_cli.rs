//! Integration tests for the `hook-pretooluse`, `hook-posttooluse`, and
//! `hook-precompact` CLI subcommands.
//!
//! These tests invoke the binary via `cargo run` with JSON piped to stdin,
//! verifying exit codes, stderr messages, and stdout JSON responses.

use std::io::Write;
use std::process::{Command, Stdio};

fn cargo_bin() -> Command {
    let mut cmd = Command::new(env!("CARGO"));
    cmd.args(["run", "--quiet", "--"]);
    cmd
}

fn run_hook(subcommand: &str, input: &str) -> (i32, String, String) {
    let mut child = cargo_bin()
        .arg(subcommand)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn codesight-mcp");

    if let Some(ref mut stdin) = child.stdin {
        stdin.write_all(input.as_bytes()).unwrap();
    }
    drop(child.stdin.take());

    let output = child.wait_with_output().expect("failed to wait");
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (code, stdout, stderr)
}

// ── hook-pretooluse: Read ────────────────────────────────────────────

#[test]
fn pretooluse_read_large_code_file_warns() {
    // Create a temp file large enough to trigger the warning.
    let dir = tempfile::tempdir().unwrap();
    let large_file = dir.path().join("big.rs");
    std::fs::write(&large_file, "a".repeat(8192)).unwrap();

    let input = serde_json::json!({
        "tool_name": "Read",
        "tool_input": {
            "file_path": large_file.to_string_lossy()
        }
    });

    let (code, _stdout, stderr) = run_hook("hook-pretooluse", &input.to_string());
    assert_eq!(code, 0);
    assert!(
        stderr.contains("codesight-hooks"),
        "expected stderr warning, got: {stderr}"
    );
    assert!(
        stderr.contains("inspect_structure") || stderr.contains("search_catalog"),
        "expected codesight suggestion in stderr: {stderr}"
    );
}

#[test]
fn pretooluse_read_small_code_file_silent() {
    let dir = tempfile::tempdir().unwrap();
    let small_file = dir.path().join("tiny.rs");
    std::fs::write(&small_file, "fn main() {}").unwrap();

    let input = serde_json::json!({
        "tool_name": "Read",
        "tool_input": {
            "file_path": small_file.to_string_lossy()
        }
    });

    let (code, stdout, stderr) = run_hook("hook-pretooluse", &input.to_string());
    assert_eq!(code, 0);
    // Filter out tracing noise — only check for hook-specific output.
    let hook_stderr: String = stderr
        .lines()
        .filter(|l| l.contains("codesight-hooks"))
        .collect();
    assert!(
        hook_stderr.is_empty(),
        "small file should be silent: {stderr}"
    );
    assert!(stdout.is_empty(), "no stdout for small file: {stdout}");
}

#[test]
fn pretooluse_read_noncode_file_silent() {
    let dir = tempfile::tempdir().unwrap();
    let md_file = dir.path().join("README.md");
    std::fs::write(&md_file, "a".repeat(10000)).unwrap();

    let input = serde_json::json!({
        "tool_name": "Read",
        "tool_input": {
            "file_path": md_file.to_string_lossy()
        }
    });

    let (code, stdout, stderr) = run_hook("hook-pretooluse", &input.to_string());
    assert_eq!(code, 0);
    let hook_stderr: String = stderr
        .lines()
        .filter(|l| l.contains("codesight-hooks"))
        .collect();
    assert!(
        hook_stderr.is_empty(),
        "non-code file should be silent: {stderr}"
    );
    assert!(stdout.is_empty(), "no stdout for non-code: {stdout}");
}

#[test]
fn pretooluse_read_with_offset_limit_silent() {
    let dir = tempfile::tempdir().unwrap();
    let big_file = dir.path().join("large.rs");
    std::fs::write(&big_file, "a".repeat(8192)).unwrap();

    let input = serde_json::json!({
        "tool_name": "Read",
        "tool_input": {
            "file_path": big_file.to_string_lossy(),
            "offset": 10,
            "limit": 20
        }
    });

    let (code, stdout, stderr) = run_hook("hook-pretooluse", &input.to_string());
    assert_eq!(code, 0);
    let hook_stderr: String = stderr
        .lines()
        .filter(|l| l.contains("codesight-hooks"))
        .collect();
    assert!(
        hook_stderr.is_empty(),
        "targeted read should be silent: {stderr}"
    );
    assert!(stdout.is_empty(), "no stdout for targeted read: {stdout}");
}

// ── hook-pretooluse: Grep ────────────────────────────────────────────

#[test]
fn pretooluse_grep_suggests_search_catalog() {
    let input = serde_json::json!({
        "tool_name": "Grep",
        "tool_input": {
            "pattern": "validate_args",
            "path": "src/"
        }
    });

    let (code, _stdout, stderr) = run_hook("hook-pretooluse", &input.to_string());
    assert_eq!(code, 0);
    assert!(
        stderr.contains("search_catalog"),
        "expected search_catalog suggestion: {stderr}"
    );
}

// ── hook-pretooluse: Glob ────────────────────────────────────────────

#[test]
fn pretooluse_glob_suggests_file_tree() {
    let input = serde_json::json!({
        "tool_name": "Glob",
        "tool_input": {
            "pattern": "**/*.rs"
        }
    });

    let (code, _stdout, stderr) = run_hook("hook-pretooluse", &input.to_string());
    assert_eq!(code, 0);
    assert!(
        stderr.contains("inspect_structure"),
        "expected inspect_structure suggestion: {stderr}"
    );
}

// ── hook-pretooluse: Bash ────────────────────────────────────────────

#[test]
fn pretooluse_bash_safe_command_silent() {
    let input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {
            "command": "cargo test --lib"
        }
    });

    let (code, stdout, stderr) = run_hook("hook-pretooluse", &input.to_string());
    assert_eq!(code, 0);
    let hook_stderr: String = stderr
        .lines()
        .filter(|l| l.contains("codesight-hooks"))
        .collect();
    assert!(
        hook_stderr.is_empty(),
        "safe command should be silent: {stderr}"
    );
    assert!(stdout.is_empty(), "no stdout for safe command: {stdout}");
}

#[test]
fn pretooluse_bash_grep_suggests_search_catalog() {
    let input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {
            "command": "grep -r 'validate' src/"
        }
    });

    let (code, _stdout, stderr) = run_hook("hook-pretooluse", &input.to_string());
    assert_eq!(code, 0);
    assert!(
        stderr.contains("search_catalog"),
        "expected search_catalog suggestion for bash grep: {stderr}"
    );
}

// ── hook-pretooluse: malformed input ─────────────────────────────────

#[test]
fn pretooluse_malformed_input_exits_zero() {
    let (code, stdout, stderr) = run_hook("hook-pretooluse", "not valid json {{{");
    assert_eq!(code, 0, "malformed input should exit 0");
    assert!(stdout.is_empty(), "no stdout on malformed: {stdout}");
    // stderr may have tracing logs but should not crash
    assert!(
        !stderr.contains("panic"),
        "should not panic on malformed input: {stderr}"
    );
}

// ── hook-precompact ──────────────────────────────────────────────────

#[test]
fn precompact_returns_session_snapshot() {
    let input = serde_json::json!({
        "tool_name": "",
        "tool_input": {}
    });

    let (code, stdout, _stderr) = run_hook("hook-precompact", &input.to_string());
    assert_eq!(code, 0);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|_| panic!("precompact stdout should be valid JSON: {stdout}"));
    assert!(
        parsed
            .pointer("/hookSpecificOutput/additionalContext")
            .is_some(),
        "should have additionalContext: {parsed}"
    );
    let ctx = parsed["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        ctx.contains("codesight session snapshot"),
        "snapshot should contain header: {ctx}"
    );
}

// ── hook-posttooluse ─────────────────────────────────────────────────

#[test]
fn posttooluse_exits_zero_for_nonexistent_file() {
    let input = serde_json::json!({
        "tool_name": "Edit",
        "tool_input": {
            "file_path": "/tmp/nonexistent_file_12345.rs"
        }
    });

    let (code, _stdout, _stderr) = run_hook("hook-posttooluse", &input.to_string());
    assert_eq!(code, 0);
}

#[test]
fn posttooluse_exits_zero_for_empty_path() {
    let input = serde_json::json!({
        "tool_name": "Write",
        "tool_input": {
            "file_path": ""
        }
    });

    let (code, _stdout, _stderr) = run_hook("hook-posttooluse", &input.to_string());
    assert_eq!(code, 0);
}
