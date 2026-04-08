//! File-level text search used by `search_catalog`.

use std::fs;

use regex::Regex;
use serde_json::{Value, json};
use walkdir::WalkDir;

use crate::service::CodeService;
use crate::service::helpers::{normalize_search_scope, should_descend, to_repo_relative};
use crate::storage::IndexStore;

impl CodeService {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::service) fn search_code_files_text(
        &self,
        store: &IndexStore,
        source_id: &str,
        query: &str,
        scope: Option<String>,
        max_results: usize,
        query_mode: &str,
        context_lines: usize,
    ) -> anyhow::Result<Vec<Value>> {
        let root = store.root_path()?;
        let max = max_results.clamp(1, 50);
        let ctx = context_lines.min(50);
        let mut results = Vec::new();
        let regex = if query_mode == "regex" {
            match Regex::new(query) {
                Ok(r) => Some(r),
                Err(e) => {
                    return Ok(vec![json!({
                        "ref": {
                            "id": format!("match::{}", results.len()),
                            "stable_id": format!("text_match::{}", results.len()),
                            "source_id": source_id,
                            "domain": "code",
                            "kind": "file",
                            "path": "",
                            "title": "regex_error",
                            "summary": e.to_string(),
                        },
                    })]);
                }
            }
        } else {
            None
        };

        // Deterministic path order (matches prior retrieval tests retrieval tests: `include/` before `src/`).
        let mut indexed_files: Vec<(String, std::path::PathBuf)> = Vec::new();
        for entry in WalkDir::new(&root)
            .into_iter()
            .filter_entry(should_descend)
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            let rel = match to_repo_relative(path, &root) {
                Ok(r) => r,
                Err(_) => continue,
            };
            if let Some(ref sc) = scope {
                let norm = normalize_search_scope(sc);
                if !rel.contains(&norm) && !rel.starts_with(&norm) {
                    continue;
                }
            }
            indexed_files.push((rel, path.to_path_buf()));
        }
        indexed_files.sort_by(|a, b| a.0.cmp(&b.0));

        for (rel, path) in indexed_files {
            if results.len() >= max {
                break;
            }
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let lines: Vec<&str> = content.lines().collect();
            for (idx, line) in lines.iter().enumerate() {
                if results.len() >= max {
                    break;
                }
                let hit = match (&regex, query_mode) {
                    (Some(r), _) => r.is_match(line),
                    (None, _) => line
                        .to_ascii_lowercase()
                        .contains(&query.to_ascii_lowercase()),
                };
                if hit {
                    let line_num = idx + 1;
                    let mut row = json!({
                        "ref": {
                            "id": format!("{rel}#L{}", line_num),
                            "stable_id": format!("{rel}#L{}", line_num),
                            "source_id": source_id,
                            "domain": "code",
                            "kind": "file",
                            "path": rel,
                            "title": format!("line {}", line_num),
                            "summary": line.trim().chars().take(200).collect::<String>(),
                        },
                        "snippet": line.trim(),
                        "line": line_num,
                    });
                    if ctx > 0 {
                        let start = idx.saturating_sub(ctx);
                        let before: Vec<String> =
                            lines[start..idx].iter().map(|s| (*s).to_string()).collect();
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
                    results.push(row);
                }
            }
        }
        Ok(results)
    }
}
