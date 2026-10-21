//! Integration tests for the `hook-pretooluse`, `hook-posttooluse`, and
//! `hook-precompact` CLI subcommands.
//!
//! These tests invoke the Cargo-built binary with JSON piped to stdin,
//! verifying exit codes, stderr messages, and stdout JSON responses.

use std::io::Write;
use std::process::{Command, Stdio};

fn cargo_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_codesight-mcp"))
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

/// Parse the JSON the hook writes on stdout and return the
/// `hookSpecificOutput.additionalContext` string, if any.
fn hook_additional_context(stdout: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    parsed
        .pointer("/hookSpecificOutput/additionalContext")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

#[test]
fn pretooluse_read_passes_through_regardless_of_size() {
    // Read is intentionally not intercepted: Claude's Edit/Write flow
    // requires a prior Read in session state, so the hook must stay silent
    // on Read no matter how large the target file is.
    let dir = tempfile::tempdir().unwrap();
    let large_file = dir.path().join("big.rs");
    std::fs::write(&large_file, "a".repeat(8192)).unwrap();

    let input = serde_json::json!({
        "tool_name": "Read",
        "tool_input": {
            "file_path": large_file.to_string_lossy()
        }
    });

    let (code, stdout, _stderr) = run_hook("hook-pretooluse", &input.to_string());
    assert_eq!(code, 0);
    assert!(
        stdout.trim().is_empty(),
        "Read should not emit any hook decision: {stdout}"
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
fn pretooluse_grep_suggests_search() {
    let input = serde_json::json!({
        "tool_name": "Grep",
        "tool_input": {
            "pattern": "validate_args",
            "path": "src/"
        }
    });

    let (code, stdout, _stderr) = run_hook("hook-pretooluse", &input.to_string());
    assert_eq!(code, 0);
    let ctx = hook_additional_context(&stdout)
        .unwrap_or_else(|| panic!("Grep should emit a hookSpecificOutput JSON: {stdout}"));
    assert!(
        ctx.contains("use: search "),
        "expected `search` suggestion in additionalContext: {ctx}"
    );
}

#[test]
fn pretooluse_grep_on_markdown_suggests_docs_search() {
    // Regression: `.md`, `.rst`, etc. are indexed as sections — Grep on them
    // must be redirected, not passed through.
    let input = serde_json::json!({
        "tool_name": "Grep",
        "tool_input": {
            "pattern": "plan_turn",
            "path": "REFACTOR_WORKLOG.md"
        }
    });

    let (code, stdout, _stderr) = run_hook("hook-pretooluse", &input.to_string());
    assert_eq!(code, 0);
    let ctx = hook_additional_context(&stdout).unwrap_or_else(|| {
        panic!("Grep on .md should emit a hookSpecificOutput JSON (deny): {stdout}")
    });
    assert!(
        ctx.contains("use: search ") && ctx.contains("docs"),
        "expected docs-oriented `search` suggestion: {ctx}"
    );
}

#[test]
fn pretooluse_grep_on_binary_passes_through() {
    // Non-indexable extensions (e.g. `.png`) still pass through so Grep can
    // be used for truly non-code/non-docs content.
    let input = serde_json::json!({
        "tool_name": "Grep",
        "tool_input": {
            "pattern": "anything",
            "path": "logo.png"
        }
    });

    let (code, stdout, _stderr) = run_hook("hook-pretooluse", &input.to_string());
    assert_eq!(code, 0);
    assert!(
        stdout.is_empty(),
        "Grep on non-indexable path should pass through silently: {stdout}"
    );
}

// ── hook-pretooluse: Glob ────────────────────────────────────────────

#[test]
fn pretooluse_glob_suggests_outline() {
    let input = serde_json::json!({
        "tool_name": "Glob",
        "tool_input": {
            "pattern": "**/*.rs"
        }
    });

    let (code, stdout, _stderr) = run_hook("hook-pretooluse", &input.to_string());
    assert_eq!(code, 0);
    let ctx = hook_additional_context(&stdout)
        .unwrap_or_else(|| panic!("Glob should emit a hookSpecificOutput JSON: {stdout}"));
    assert!(
        ctx.contains("outline view=repo"),
        "expected `outline view=repo` suggestion in additionalContext: {ctx}"
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
fn pretooluse_bash_grep_suggests_search() {
    let input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {
            "command": "grep -r 'validate' src/"
        }
    });

    let (code, stdout, _stderr) = run_hook("hook-pretooluse", &input.to_string());
    assert_eq!(code, 0);
    let ctx = hook_additional_context(&stdout)
        .unwrap_or_else(|| panic!("Bash grep should emit a hookSpecificOutput JSON: {stdout}"));
    assert!(
        ctx.contains("use: search "),
        "expected `search` suggestion for bash grep in additionalContext: {ctx}"
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
