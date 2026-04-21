use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Serialize;
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorCategory {
    InvalidArgument,
    NotFound,
    /// Retryable I/O, timeouts, rate limits. Surfaced on `_meta.error_category`
    /// when a handler exceeds its per-tool timeout (see
    /// `service::agent_v1::registry::TimeoutClass`) or otherwise fails in a
    /// retryable way.
    Transient,
    Internal,
}

#[derive(Debug, thiserror::Error)]
#[error("[{code}] {message}")]
pub struct ToolError {
    pub category: ToolErrorCategory,
    pub code: &'static str,
    pub message: String,
    pub detail: Value,
}

impl ToolError {
    pub fn new(
        category: ToolErrorCategory,
        code: &'static str,
        message: impl Into<String>,
        detail: Value,
    ) -> Self {
        Self {
            category,
            code,
            message: message.into(),
            detail,
        }
    }
}

pub fn index_error(error: anyhow::Error) -> ToolError {
    ToolError::new(
        ToolErrorCategory::Internal,
        "INDEX_ERROR",
        error.to_string(),
        json!({}),
    )
}

pub fn validate_id(symbol_id: &str) -> Result<(), ToolError> {
    if symbol_id.contains("::") && symbol_id.contains('#') {
        Ok(())
    } else {
        Err(ToolError::new(
            ToolErrorCategory::InvalidArgument,
            "INVALID_ID",
            format!("Stable identifier '{symbol_id}' is malformed."),
            json!({ "id": symbol_id }),
        ))
    }
}

pub fn canonicalize_root(path: &str) -> anyhow::Result<PathBuf> {
    let root = Path::new(path);
    let canonical = root
        .canonicalize()
        .with_context(|| format!("path '{}' does not exist", root.display()))?;
    if !canonical.is_dir() {
        anyhow::bail!("path '{}' is not a directory", canonical.display());
    }
    Ok(canonical)
}

pub fn to_repo_relative(path: &Path, root: &Path) -> anyhow::Result<String> {
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("{} is not under {}", path.display(), root.display()))?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

/// Turn UI scopes like `src/*.py` into a repo-relative path prefix (`src`) so fts-style
/// globs match gitignore-style glob behavior.
pub fn normalize_search_scope(scope: &str) -> String {
    let s = scope.replace('\\', "/");
    // Infallible: split always yields at least one (possibly empty) segment.
    let head = s.split('*').next().unwrap_or("").trim_matches('/').trim();
    head.to_string()
}

/// 1-based line semantics (matches prior retrieval tests `get_file_content` tests).
/// For an empty file, returns `(0, 0, 0, "")`.
/// With no range, returns the **original** `content` (preserves `\r\n`) and `(line_count, 1, line_count, content)`.
/// When slicing, `line_count` is always the **total** lines in the file; `start_line`/`end_line` are clamped.
pub fn slice_file_lines(
    content: &str,
    start_line: Option<u32>,
    end_line: Option<u32>,
) -> (u32, u32, u32, String) {
    let lines: Vec<&str> = content.lines().collect();
    let n = lines.len() as u32;
    if n == 0 {
        return (0, 0, 0, String::new());
    }
    if start_line.is_none() && end_line.is_none() {
        return (n, 1, n, content.to_string());
    }

    let mut s = start_line.unwrap_or(1);
    let mut e = end_line.unwrap_or(n);
    if s > e {
        e = s;
    }
    s = s.clamp(1, n);
    e = e.clamp(1, n);
    if e < s {
        e = s;
    }
    let body = lines[(s - 1) as usize..e as usize].join("\n");
    (n, s, e, body)
}

pub fn expand_context(
    source: &str,
    start_line: u32,
    end_line: u32,
    context_lines: usize,
) -> (u32, u32, String) {
    let lines = source.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return (start_line, end_line, String::new());
    }

    // Indexed line numbers can outlive the on-disk file when a record
    // refers to a range that shrank after reindex — clamp everything to
    // `lines.len()` so an out-of-range slice can never panic the handler.
    let len = lines.len();
    let raw_start = start_line.saturating_sub(1).saturating_sub(context_lines as u32) as usize;
    let raw_end = (end_line as usize).saturating_add(context_lines);
    let start_index = raw_start.min(len);
    let end_index = raw_end.min(len).max(start_index);
    let expanded = lines[start_index..end_index].join("\n");
    let start_1based = if start_index == len {
        len as u32
    } else {
        start_index as u32 + 1
    };
    (start_1based, end_index as u32, expanded)
}

pub fn attach_meta(body: Value, meta: Value) -> anyhow::Result<Value> {
    match body {
        Value::Object(mut object) => {
            object.insert("_meta".to_string(), meta);
            Ok(Value::Object(object))
        }
        other => {
            let mut object = Map::new();
            object.insert("result".to_string(), other);
            object.insert("_meta".to_string(), meta);
            Ok(Value::Object(object))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_context_clamps_when_lines_are_stale_past_eof() {
        // Simulates the panic we hit in the wild: indexed record says
        // the symbol starts at line 105_756, but the on-disk file has
        // only 3 lines. Must not panic; must return empty expansion.
        let src = "a\nb\nc";
        let (start, end, body) = expand_context(src, 105_756, 105_800, 20);
        assert_eq!(start, 3, "start clamped to len");
        assert_eq!(end, 3);
        assert_eq!(body, "");
        assert!(start <= end, "returned range must never invert");
    }

    #[test]
    fn expand_context_clamps_when_end_past_eof_but_start_in_range() {
        let src = "one\ntwo\nthree\nfour";
        let (start, end, body) = expand_context(src, 3, 99, 0);
        assert_eq!(start, 3);
        assert_eq!(end, 4);
        assert_eq!(body, "three\nfour");
    }

    #[test]
    fn expand_context_handles_empty_source() {
        let (start, end, body) = expand_context("", 1, 5, 10);
        assert_eq!((start, end, body.as_str()), (1, 5, ""));
    }
}
