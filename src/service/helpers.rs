use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::{Map, Value, json};
use walkdir::DirEntry;

#[derive(Debug, thiserror::Error)]
#[error("[{code}] {message}")]
pub struct ToolError {
    pub code: &'static str,
    pub message: String,
    pub detail: Value,
}

pub fn index_error(error: anyhow::Error) -> ToolError {
    ToolError {
        code: "INDEX_ERROR",
        message: error.to_string(),
        detail: json!({}),
    }
}

pub fn validate_id(symbol_id: &str) -> Result<(), ToolError> {
    if symbol_id.contains("::") && symbol_id.contains('#') {
        Ok(())
    } else {
        Err(ToolError {
            code: "INVALID_ID",
            message: format!("Stable identifier '{symbol_id}' is malformed."),
            detail: json!({ "id": symbol_id }),
        })
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

pub fn should_descend(entry: &DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    !crate::db::SKIP_DIRS.iter().any(|skip| *skip == name)
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

    let start_index = start_line
        .saturating_sub(1)
        .saturating_sub(context_lines as u32) as usize;
    let end_index = (end_line as usize + context_lines).min(lines.len());
    let expanded = lines[start_index..end_index].join("\n");
    (start_index as u32 + 1, end_index as u32, expanded)
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
