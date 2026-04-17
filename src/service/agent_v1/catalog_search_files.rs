//! File-level text search used by `search_catalog`.
//!
//! Walks the repo once per call and returns both the capped item list and the
//! exact pre-cap counts. Uses `ignore::WalkBuilder` so it respects
//! `.gitignore` + SKIP_DIRS exactly like the ingest walker — the regex path
//! must never descend into vendored code, build output, or the codesight
//! index dir. Skips binary files (large or null-byte-containing) before
//! reading them into memory so a 30 MB `model.safetensors` doesn't cost us
//! 100 ms on every regex query.

use std::fs;
use std::path::Path;

use ignore::WalkBuilder;
use regex::Regex;
use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::helpers::{normalize_search_scope, to_repo_relative};
use crate::storage::IndexStore;

/// Hard cap on individual file size for text search. Anything above this is
/// treated as binary regardless of contents: loading a multi-MB blob into a
/// `String` just to scan one regex over it is a dispatch-budget black hole.
/// 2 MB comfortably covers every source/doc file we index and still excludes
/// vendored snapshots, model weights, and generated artifacts.
const MAX_TEXT_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Leading bytes we inspect for a NUL byte before trusting a file as text.
/// Chosen to cover the header of typical binary formats (PNG, safetensors,
/// wasm, etc.) without reading far into a large file on the unhappy path.
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

fn path_is_excluded(path: &str, exclude: &[String]) -> bool {
    exclude
        .iter()
        .any(|pattern| path.starts_with(pattern) || path.contains(pattern))
}

/// Read the file as text, returning `None` for anything that looks binary or
/// is too large to scan cheaply. Does a cheap size probe, a null-byte sniff on
/// the header, and only then pays the full read + UTF-8 validation. IO errors
/// (unreadable file, permission denied) collapse to `None` so the walker
/// skips them silently.
fn read_text_file(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    if meta.len() > MAX_TEXT_FILE_BYTES {
        return None;
    }

    let bytes = fs::read(path).ok()?;
    let head = &bytes[..bytes.len().min(BINARY_SNIFF_BYTES)];
    if head.contains(&0u8) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn scope_matches(rel: &str, scope: Option<&str>) -> bool {
    match scope {
        None => true,
        Some(sc) => {
            let norm = normalize_search_scope(sc);
            rel.contains(&norm) || rel.starts_with(&norm)
        }
    }
}

fn build_walker(root: &Path) -> ignore::Walk {
    WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(|entry| {
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                let name = entry.file_name().to_string_lossy();
                return !crate::db::SKIP_DIRS.iter().any(|skip| *skip == name);
            }
            true
        })
        .build()
}

fn line_matches(query_mode: &str, regex: Option<&Regex>, query_lower: &str, line: &str) -> bool {
    match (regex, query_mode) {
        (Some(r), _) => r.is_match(line),
        (None, _) => line.to_ascii_lowercase().contains(query_lower),
    }
}

/// Pre-cap result aggregated across one full walk.
pub(in crate::service) struct FileTextSearchResult {
    pub items: Vec<Value>,
    pub total_matches: usize,
    pub files_with_matches: usize,
    /// True when we observed more file-level hits than the caller's cap.
    /// Used to annotate `truncated_by=limit` without a second pass.
    pub more_file_hits: bool,
}

fn push_hit(
    rel: &str,
    source_id: &str,
    lines: &[&str],
    idx: usize,
    line: &str,
    ctx: usize,
) -> Value {
    let line_num = idx + 1;
    let mut row = json!({
        "ref": {
            "id": rel,
            "stable_id": rel,
            "source_id": source_id,
            "domain": "code",
            "kind": "file",
            "path": rel,
            "start_line": line_num,
            "title": format!("line {}", line_num),
            "summary": line.trim().chars().take(200).collect::<String>(),
        },
        "snippet": line.trim(),
        "score": 1.0,
        "line": line_num,
    });
    if ctx > 0 {
        let start = idx.saturating_sub(ctx);
        let before: Vec<String> = lines[start..idx].iter().map(|s| (*s).to_string()).collect();
        let end = (idx + ctx + 1).min(lines.len());
        let after: Vec<String> = lines[idx + 1..end]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        if let Some(obj) = row.as_object_mut() {
            obj.insert("text".to_string(), json!(line.trim()));
            obj.insert("before".to_string(), json!(before));
            obj.insert("after".to_string(), json!(after));
        }
    }
    row
}

impl CodeService {
    /// Single-walk file text search. Produces the capped item list and the
    /// pre-cap match totals the caller needs for
    /// `total_matches` / `total_files_with_matches` without a second pass
    /// over the filesystem.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::service) fn search_code_files_text(
        &self,
        store: &IndexStore,
        source_id: &str,
        query: &str,
        scope: Option<String>,
        exclude: &[String],
        max_results: usize,
        query_mode: &str,
        context_lines: usize,
    ) -> anyhow::Result<FileTextSearchResult> {
        let root = store.root_path()?;
        let max = max_results.clamp(1, 50);
        let ctx = context_lines.min(50);

        let regex = if query_mode == "regex" {
            match Regex::new(query) {
                Ok(r) => Some(r),
                Err(e) => {
                    // Stable contract: surface the compile error as a single
                    // synthetic file hit so the MCP boundary returns a
                    // well-formed items list instead of a dispatcher-level
                    // error. Counts stay at 1/1 so the client can see exactly
                    // one result and its diagnostic summary.
                    return Ok(FileTextSearchResult {
                        items: vec![json!({
                            "ref": {
                                "id": "regex_error",
                                "stable_id": "regex_error",
                                "source_id": source_id,
                                "domain": "code",
                                "kind": "file",
                                "path": "",
                                "title": "regex_error",
                                "summary": format!(
                                    "`query` is used as a regex pattern in regex mode. Your query '{}' failed to compile: {}. Did you mean query_mode=keyword?",
                                    query, e
                                ),
                            },
                        })],
                        total_matches: 1,
                        files_with_matches: 1,
                        more_file_hits: false,
                    });
                }
            }
        } else {
            None
        };

        let query_lower = query.to_ascii_lowercase();
        let mut items: Vec<Value> = Vec::new();
        let mut total_matches = 0usize;
        let mut files_with_matches = 0usize;
        let mut more_file_hits = false;
        let scope_ref = scope.as_deref();

        // Deterministic path order — matches prior retrieval-test ordering
        // (e.g. `include/` before `src/`). `ignore::Walk` itself is not
        // guaranteed to be sorted, so we collect-then-sort rather than stream.
        let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
        for entry in build_walker(&root).flatten() {
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                continue;
            }
            let path = entry.path();
            let rel = match to_repo_relative(path, &root) {
                Ok(r) => r,
                Err(_) => continue,
            };
            if !scope_matches(&rel, scope_ref) {
                continue;
            }
            if path_is_excluded(&rel, exclude) {
                continue;
            }
            files.push((rel, path.to_path_buf()));
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));

        for (rel, path) in files {
            let Some(content) = read_text_file(&path) else {
                continue;
            };
            let lines: Vec<&str> = content.lines().collect();
            let mut file_hits = 0usize;
            for (idx, line) in lines.iter().enumerate() {
                if !line_matches(query_mode, regex.as_ref(), &query_lower, line) {
                    continue;
                }
                file_hits += 1;
                total_matches += 1;
                if items.len() < max {
                    items.push(push_hit(&rel, source_id, &lines, idx, line, ctx));
                } else {
                    more_file_hits = true;
                }
            }
            if file_hits > 0 {
                files_with_matches += 1;
            }
        }

        Ok(FileTextSearchResult {
            items,
            total_matches,
            files_with_matches,
            more_file_hits,
        })
    }
}
