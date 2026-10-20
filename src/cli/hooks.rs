//! Claude Code hook subcommands: `hook-pretooluse`, `hook-posttooluse`, `hook-precompact`.
//!
//! All three read JSON from stdin (Claude Code hook protocol), decide on an
//! action, and write JSON/stderr. They must complete in <50ms with no network
//! I/O. The only runtime dependency is the codesight-mcp binary on PATH.
//!
//! Policy:
//!   - `Read` is **never** intercepted. Claude's Edit/Write tools require a
//!     prior Read in session state, so blocking Read breaks the edit workflow.
//!   - `Grep`, `Glob`, and exploration-style `Bash` commands (grep/rg/find/
//!     cat/head/tail/less/more/wc/awk/sed) are hard-blocked via structured
//!     `permissionDecision: deny`, with `additionalContext` telling the model
//!     which codesight-mcp tool to use instead.
//!   - Known build/VCS/runtime Bash prefixes (`SAFE_BASH_PREFIXES`) pass
//!     through without inspection.

use rustc_hash::FxHashMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use codesight_engine::server_support::agent_readiness_cache;
use codesight_engine::service::savings::SavingsLedger;

// ── Constants ────────────────────────────────────────────────────────────

/// Commands that are always safe (build tools, VCS, runtimes).
const SAFE_BASH_PREFIXES: &[&str] = &[
    "git", "cargo", "npm", "yarn", "docker", "kubectl", "pip", "brew", "make", "cmake", "python",
    "node", "bun", "deno", "pnpm", "rustup", "rustc", "go ", "mvn", "gradle",
];

/// Bash commands that are really code-exploration tools.
const EXPLORE_BASH_CMDS: &[&str] = &[
    "grep", "rg", "find", "cat", "head", "tail", "less", "more", "wc", "awk", "sed",
];

// ── Input structs ────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct HookInput {
    #[serde(default)]
    tool_name: String,
    #[serde(default)]
    tool_input: Value,
}

// ── Public entry points ──────────────────────────────────────────────────

/// `codesight-mcp hook-pretooluse` — reads hook JSON from stdin, classifies,
/// and emits structured `permissionDecision: deny` for exploration calls.
/// Exit 0 always; Claude reads the decision from stdout JSON.
pub fn run_pretooluse() -> i32 {
    let input = match read_hook_input() {
        Some(v) => v,
        None => return 0, // malformed → pass through silently
    };

    match input.tool_name.as_str() {
        "Grep" => handle_grep(&input.tool_input),
        "Glob" => handle_glob(&input.tool_input),
        "Bash" => handle_bash(&input.tool_input),
        "Edit" | "Write" => handle_edit_probe(&input.tool_input),
        // Read is intentionally NOT intercepted: Edit/Write require a prior
        // Read in Claude's session state, so blocking it breaks edit flows.
        _ => 0,
    }
}

/// `codesight-mcp hook-posttooluse` — fires after Edit/Write to auto-reindex
/// and surface refactor-shaped edit hints.
pub fn run_posttooluse() -> i32 {
    let input = match read_hook_input() {
        Some(v) => v,
        None => return 0,
    };

    let file_path = input
        .tool_input
        .get("file_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if file_path.is_empty() {
        return 0;
    }

    let path = Path::new(file_path);
    if !path.exists() {
        return 0;
    }

    // Reindex is handled in-process by the long-lived MCP server's file watcher
    // (see `WatcherManager` in `crates/codesight-engine/src/service/file_watcher.rs`).
    // The watcher activates after the first `ingest_sources` call for a workspace
    // and auto-reindexes touched files with a 500ms notify-debounce.

    // Count the double-read: Claude Code's Edit contract forces a native
    // Read of `file_path` before the Edit fires. If we had already returned
    // that file via `read_content`, those bytes are now superseded.
    let ledger_path = index_storage_dir().join("_savings.json");
    let ledger = SavingsLedger::new(ledger_path);
    let _ = ledger.record_edit_and_check_double_read(file_path);

    if let Some(prefix) = detect_use_prefix_rewrite(&input.tool_input)
        && should_emit_refactor_hint(&prefix)
    {
        emit_additional_context(&format!(
            "PostToolUse: this edit rewrote `use {prefix}::…` imports across 5+ files. \
             Run `impact analysis=blast_radius ref=\"{prefix}::…\"` to preview the change \
             surface before hand-editing every caller."
        ));
    }

    0
}

/// `codesight-mcp hook-precompact` — returns session snapshot as additionalContext.
pub fn run_precompact() -> i32 {
    let _input = read_hook_input(); // consume stdin even if unused

    let snapshot = build_session_snapshot();

    let response = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreCompact",
            "additionalContext": snapshot
        }
    });

    let _ = serde_json::to_writer(io::stdout().lock(), &response);
    0
}

// ── Grep handler ─────────────────────────────────────────────────────────

fn handle_grep(tool_input: &Value) -> i32 {
    let pattern = tool_input
        .get("pattern")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if pattern.is_empty() {
        return 0;
    }

    let target_path = tool_input
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Classify target: code → deny with code hint, docs → deny with docs hint,
    // truly non-indexable (binary/lock/image) → pass through.
    let domain = if target_path.is_empty() {
        IndexableDomain::Code
    } else {
        match indexable_domain(target_path) {
            Some(d) => d,
            None => return 0,
        }
    };

    let suggestion = match domain {
        IndexableDomain::Docs => format!(
            "Grep for '{pattern}' on docs → use: search mode=regex query=\"{pattern}\" scope=\"*.md\""
        ),
        IndexableDomain::Code if is_identifier_pattern(pattern) => {
            format!("Grep for '{pattern}' → use: search query=\"{pattern}\"")
        }
        IndexableDomain::Code => {
            format!("Grep for '{pattern}' → use: search mode=regex query=\"{pattern}\"")
        }
    };

    emit_deny(&suggestion);
    0
}

// ── Glob handler ─────────────────────────────────────────────────────────

fn handle_glob(tool_input: &Value) -> i32 {
    let pattern = tool_input
        .get("pattern")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if pattern.is_empty() {
        return 0;
    }

    let suggestion = if pattern.contains("**") || pattern.ends_with('*') {
        format!("Glob '{}' → use: outline view=repo", pattern)
    } else {
        format!("Glob '{}' → use: search query=\"{}\"", pattern, pattern)
    };

    emit_deny(&suggestion);
    0
}

// ── Bash handler ─────────────────────────────────────────────────────────

fn handle_bash(tool_input: &Value) -> i32 {
    let command = tool_input
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if command.is_empty() {
        return 0;
    }

    let trimmed = command.trim();

    // Whitelist safe commands.
    for prefix in SAFE_BASH_PREFIXES {
        if trimmed.starts_with(prefix) {
            return 0;
        }
    }

    // Exploration-style commands → hard block.
    let first_word = trimmed.split_whitespace().next().unwrap_or("");
    // Strip path prefix (e.g. /usr/bin/grep → grep).
    let cmd_name = first_word.rsplit('/').next().unwrap_or(first_word);

    if !EXPLORE_BASH_CMDS.contains(&cmd_name) {
        return 0;
    }

    let suggestion = match cmd_name {
        "grep" | "rg" => format!(
            "Bash '{}' → use: search mode=regex query=<pattern>",
            cmd_name
        ),
        "find" => "Bash 'find' → use: outline view=repo".to_string(),
        "cat" | "head" | "tail" | "less" | "more" => format!(
            "Bash '{}' → use: read refs=[\"path:N-M\"] (or outline view=file path=... first)",
            cmd_name
        ),
        _ => format!("Bash '{}' → use codesight-mcp tools instead", cmd_name),
    };

    emit_deny(&suggestion);
    0
}

fn handle_edit_probe(tool_input: &Value) -> i32 {
    let file_path = tool_input
        .get("file_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if file_path.is_empty() || !is_code_file(file_path) {
        return 0;
    }

    // Pre-edit hot-prefix nudge: if this Edit touches a `use foo::bar::…`
    // prefix we've already seen rewritten ≥3 times across ≥3 distinct
    // files this session, suggest `evaluate_impact analysis=path_rename`
    // *before* the N+1 hand edit lands. Reads the rolling counter that
    // PostToolUse already maintains — no new persistence.
    if let Some(prefix) = detect_use_prefix_rewrite(tool_input)
        && hot_prefix_count(&prefix) >= 3
    {
        emit_allow(&format!(
            "PreToolUse: `use {prefix}::…` has already been rewritten by hand 3+ times this session. \
             Run `impact analysis=blast_radius ref=\"{prefix}::…\"` to scope the change before \
             editing more files."
        ));
        // Intentionally fall through: the nudge is advisory, not a block.
    }

    let Some(entry) = load_agent_readiness_probe(file_path) else {
        return 0;
    };
    if entry.score >= entry.threshold {
        return 0;
    }

    let message = format!(
        "Editing `{}`: file agent_readiness={:.2} (threshold {:.2}), weakest={}. Consider refactor before adding code. Run `health analysis=quality_signal scope={}` for detail.",
        file_path, entry.score, entry.threshold, entry.weakest_dimension, file_path
    );
    emit_allow(&message);
    0
}

/// Read-only probe against the session ledger. Returns the current
/// `use_prefix_edits[prefix]` count (0 if the ledger can't be loaded or the
/// prefix is cold) — used by [`handle_edit_probe`] to fire a nudge before
/// the next hand edit lands.
fn hot_prefix_count(prefix: &str) -> u64 {
    let ledger_path = index_storage_dir().join("_savings.json");
    let ledger = SavingsLedger::new(ledger_path);
    ledger
        .current_session()
        .ok()
        .and_then(|s| s.use_prefix_edits.get(prefix).copied())
        .unwrap_or(0)
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn read_hook_input() -> Option<HookInput> {
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf).ok()?;
    serde_json::from_str(&buf).ok()
}

fn is_code_file(path: &str) -> bool {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    codesight_language_support::is_code_extension(ext)
}

#[derive(Copy, Clone)]
enum IndexableDomain {
    Code,
    Docs,
}

/// Classify a target path. `None` means truly non-indexable (binary, lock,
/// image) — the hook should pass through. `Some(_)` means codesight can
/// handle it and the hook should deny with the matching suggestion.
fn indexable_domain(path: &str) -> Option<IndexableDomain> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if ext.is_empty() {
        return Some(IndexableDomain::Code);
    }
    if codesight_language_support::is_code_extension(ext) {
        return Some(IndexableDomain::Code);
    }
    if codesight_language_support::is_doc_extension(ext) {
        return Some(IndexableDomain::Docs);
    }
    None
}

fn is_identifier_pattern(pattern: &str) -> bool {
    !pattern.is_empty()
        && pattern
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == ':' || c == '.')
}

fn emit_deny(message: &str) {
    record_native_tool_fallback();
    let deny = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "additionalContext": message
        }
    });
    let _ = serde_json::to_writer(io::stdout().lock(), &deny);
}

fn emit_allow(message: &str) {
    let allow = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "additionalContext": message
        }
    });
    let _ = serde_json::to_writer(io::stdout().lock(), &allow);
}

fn emit_additional_context(message: &str) {
    let response = json!({
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "additionalContext": message
        }
    });
    let _ = serde_json::to_writer(io::stdout().lock(), &response);
}

/// Parse an Edit/Write `tool_input` and, if the change rewrites one or more
/// `use foo::bar::…;` lines, return the common source prefix so the
/// PostToolUse hook can track rename-shaped edits.
fn detect_use_prefix_rewrite(tool_input: &Value) -> Option<String> {
    let (old_src, new_src) = if let Some(old) = tool_input.get("old_string").and_then(Value::as_str)
    {
        (
            old.to_string(),
            tool_input
                .get("new_string")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        )
    } else {
        let edits = tool_input.get("edits").and_then(Value::as_array)?;
        let mut old = String::new();
        let mut new = String::new();
        for edit in edits {
            if let Some(o) = edit.get("old_string").and_then(Value::as_str) {
                old.push_str(o);
                old.push('\n');
            }
            if let Some(n) = edit.get("new_string").and_then(Value::as_str) {
                new.push_str(n);
                new.push('\n');
            }
        }
        (old, new)
    };
    let mut old_prefixes = FxHashMap::default();
    let mut new_prefixes = FxHashMap::default();
    for line in old_src.lines() {
        if let Some(prefix) = use_path_prefix(line) {
            *old_prefixes.entry(prefix).or_default() += 1;
        }
    }
    for line in new_src.lines() {
        if let Some(prefix) = use_path_prefix(line) {
            *new_prefixes.entry(prefix).or_default() += 1;
        }
    }
    for (prefix, count) in old_prefixes {
        if count >= 1 && new_prefixes.get(&prefix).copied().unwrap_or(0) < count {
            return Some(prefix);
        }
    }
    None
}

/// Return the leading two path segments of a `use foo::bar::…;` statement,
/// e.g. `use crate::storage::X;` → `Some("crate::storage")`.
fn use_path_prefix(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = trimmed
        .strip_prefix("pub use")
        .or_else(|| trimmed.strip_prefix("use"))?;
    let rest = rest.trim_start();
    let head = rest
        .split(|c: char| c == ';' || c == '{' || c.is_whitespace())
        .next()?;
    let segments: Vec<&str> = head.split("::").collect();
    if segments.len() < 2 {
        return None;
    }
    Some(format!("{}::{}", segments[0], segments[1]))
}

/// Record the rewrite in the session ledger and decide whether the hint
/// should fire. Emits when the running count for a prefix reaches 5 and
/// `evaluate_impact analysis=blast_radius` has not yet been called in this
/// session. Returns `false` on any I/O error so the hook stays silent.
fn index_storage_dir() -> PathBuf {
    std::env::var_os(codesight_engine::config::env::INDEX_DIR)
        .map(PathBuf::from)
        .unwrap_or_else(codesight_engine::config::default_index_root)
}

fn should_emit_refactor_hint(prefix: &str) -> bool {
    let ledger_path = index_storage_dir().join("_savings.json");
    let ledger = SavingsLedger::new(ledger_path.clone());
    let count = match ledger.record_use_prefix_edit(prefix) {
        Ok(c) => c,
        Err(_) => return false,
    };
    if count < 5 {
        return false;
    }
    // Skip the hint when the agent already ran blast_radius in this session.
    let Ok(bytes) = std::fs::read(&ledger_path) else {
        return true;
    };
    let state: Value = serde_json::from_slice(&bytes).unwrap_or_default();
    let blast = state
        .pointer("/current_session/evaluate_impact_usage/blast_radius")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    blast == 0
}

fn build_session_snapshot() -> String {
    let savings_path = index_storage_dir().join("_savings.json");

    let state: Value = match std::fs::read(&savings_path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Value::default(),
    };

    let total_saved = state
        .get("total_tokens_saved")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let session = &state["current_session"];
    let started_at = session
        .get("started_at")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let mut tool_lines = Vec::new();
    if let Some(per_tool) = session.get("per_tool").and_then(|v| v.as_object()) {
        let mut entries: Vec<_> = per_tool.iter().collect();
        entries.sort_by(|a, b| {
            let ca = a.1.get("calls").and_then(|v| v.as_u64()).unwrap_or(0);
            let cb = b.1.get("calls").and_then(|v| v.as_u64()).unwrap_or(0);
            cb.cmp(&ca)
        });
        for (tool, stats) in entries {
            let calls = stats.get("calls").and_then(|v| v.as_u64()).unwrap_or(0);
            tool_lines.push(format!("{}({})", tool, calls));
        }
    }

    let tools_str = if tool_lines.is_empty() {
        "none".to_string()
    } else {
        tool_lines.join(", ")
    };

    format!(
        "## codesight session snapshot\n\
         - Session started: {}\n\
         - Total tokens saved: {}\n\
         - Tools: {}\n\n\
         Prefer codesight-mcp tools for code exploration.",
        started_at,
        format_number(total_saved),
        tools_str
    )
}

fn record_native_tool_fallback() {
    let ledger = SavingsLedger::new(index_storage_dir().join("_savings.json"));
    let _ = ledger.record_fallback_to_native_tool();
}

fn load_agent_readiness_probe(path: &str) -> Option<agent_readiness_cache::Entry> {
    let cache_path = index_storage_dir().join(agent_readiness_cache::FILE_NAME);
    let bytes = std::fs::read(cache_path).ok()?;
    let state: agent_readiness_cache::State = serde_json::from_slice(&bytes).ok()?;
    let entry = state.entries.get(path)?;
    let computed_at = DateTime::parse_from_rfc3339(&entry.computed_at).ok()?;
    let age = SystemTime::now()
        .duration_since(computed_at.with_timezone(&Utc).into())
        .ok()?;
    if age > agent_readiness_cache::TTL {
        return None;
    }
    Some(entry.clone())
}

fn format_number(n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_extensions_detected() {
        assert!(is_code_file("src/main.rs"));
        assert!(is_code_file("app.py"));
        assert!(is_code_file("index.tsx"));
        assert!(!is_code_file("README.md"));
        assert!(!is_code_file("image.png"));
        assert!(!is_code_file("data.csv"));
    }

    #[test]
    fn identifier_pattern_detection() {
        assert!(is_identifier_pattern("validate_args"));
        assert!(is_identifier_pattern("CodeService"));
        assert!(is_identifier_pattern("std::io::Read"));
        assert!(!is_identifier_pattern("fn\\s+\\w+"));
        assert!(!is_identifier_pattern("error|warning"));
        assert!(!is_identifier_pattern(""));
    }

    #[test]
    fn format_number_works() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(123), "123");
        assert_eq!(format_number(1234), "1,234");
        assert_eq!(format_number(1_234_567), "1,234,567");
    }

    #[test]
    fn pretooluse_allows_read() {
        // Read is never intercepted, regardless of size/extension/offset.
        let input = HookInput {
            tool_name: "Read".to_string(),
            tool_input: json!({"file_path": "/tmp/anything.rs"}),
        };
        // Directly drive the dispatcher since stdin can't be mocked here.
        assert_eq!(
            match input.tool_name.as_str() {
                "Grep" => handle_grep(&input.tool_input),
                "Glob" => handle_glob(&input.tool_input),
                "Bash" => handle_bash(&input.tool_input),
                _ => 0,
            },
            0
        );
    }

    #[test]
    fn bash_safelist_passes_through_without_deny() {
        // cargo/git/npm etc. must pass through silently.
        for safe in &["cargo build", "git status", "npm test", "docker ps"] {
            let input = json!({"command": safe});
            // A pass-through returns 0 with no stdout emission — we can only
            // check that handle_bash doesn't panic and returns 0.
            assert_eq!(handle_bash(&input), 0);
        }
    }
}
