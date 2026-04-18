use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::Context;
use ignore::WalkBuilder;
use serde::Serialize;

use codesight_storage::storage::IndexStore;

#[derive(Debug, Clone, Serialize)]
pub struct PathRenameChange {
    pub line: u32,
    pub col: u32,
    pub before: String,
    pub after: String,
    pub node_kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathRenameFilePlan {
    pub path: String,
    pub changes: Vec<PathRenameChange>,
    #[serde(skip_serializing)]
    pub updated_source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathRenameUnhandled {
    pub path: String,
    pub line: u32,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathRenameCollision {
    pub path: String,
    pub line: u32,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathRenamePlan {
    pub files_affected: Vec<PathRenameFilePlan>,
    pub unhandled: Vec<PathRenameUnhandled>,
    pub collisions: Vec<PathRenameCollision>,
}

pub fn preview_rust_path_rename(
    store: &IndexStore,
    from: &str,
    to: &str,
    scope: Option<&str>,
    partial_mode: &str,
) -> anyhow::Result<PathRenamePlan> {
    let root = store.root_path()?;
    let mut files_affected = Vec::new();
    let mut unhandled = Vec::new();
    let mut collisions = Vec::new();

    for rel_path in rust_files(&root, scope)? {
        let abs_path = root.join(&rel_path);
        let source = fs::read_to_string(&abs_path)
            .with_context(|| format!("read '{}'", abs_path.display()))?;
        if !source.contains(from) && !looks_like_partial_group_use(&source, from) {
            continue;
        }

        let file_unhandled = collect_partial_group_warnings(&source, &rel_path, from, partial_mode);
        if partial_mode != "skip" {
            unhandled.extend(file_unhandled);
        }

        let replacements = collect_path_replacements(&source, from);
        if replacements.is_empty() {
            continue;
        }

        let updated_source = apply_replacements(&source, &replacements, to);
        let file_collisions = duplicate_use_collisions(&updated_source, &rel_path);
        collisions.extend(file_collisions);

        let changes = replacements
            .iter()
            .map(|replacement| PathRenameChange {
                line: replacement.line,
                col: replacement.col,
                before: replacement.before.clone(),
                after: replacement.after(from, to),
                node_kind: replacement.node_kind.clone(),
            })
            .collect::<Vec<_>>();

        files_affected.push(PathRenameFilePlan {
            path: rel_path,
            changes,
            updated_source,
        });
    }

    Ok(PathRenamePlan {
        files_affected,
        unhandled,
        collisions,
    })
}

fn rust_files(root: &Path, scope: Option<&str>) -> anyhow::Result<Vec<String>> {
    let trimmed_scope = scope.map(|value| value.trim_matches('/').to_string());
    let mut files = Vec::new();
    for entry in WalkBuilder::new(root).standard_filters(true).build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        if let Some(scope) = trimmed_scope.as_deref()
            && !path_matches_scope(&rel, scope)
        {
            continue;
        }
        files.push(rel);
    }
    files.sort();
    Ok(files)
}

#[derive(Debug, Clone)]
struct Replacement {
    start: usize,
    end: usize,
    line: u32,
    col: u32,
    before: String,
    node_kind: String,
}

impl Replacement {
    fn after(&self, from: &str, to: &str) -> String {
        self.before.replacen(from, to, 1)
    }
}

fn collect_path_replacements(source: &str, from: &str) -> Vec<Replacement> {
    let bytes = source.as_bytes();
    let from_bytes = from.as_bytes();
    let mut out = Vec::new();
    let mut idx = 0usize;
    let mut line = 1u32;
    let mut col = 1u32;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut in_string = None::<u8>;
    let mut escape = false;

    while idx < bytes.len() {
        let byte = bytes[idx];
        if in_line_comment {
            if byte == b'\n' {
                in_line_comment = false;
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
            idx += 1;
            continue;
        }
        if in_block_comment {
            if byte == b'*' && bytes.get(idx + 1) == Some(&b'/') {
                in_block_comment = false;
                idx += 2;
                col += 2;
                continue;
            }
            if byte == b'\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
            idx += 1;
            continue;
        }
        if let Some(quote) = in_string {
            if escape {
                escape = false;
            } else if byte == b'\\' {
                escape = true;
            } else if byte == quote {
                in_string = None;
            }
            if byte == b'\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
            idx += 1;
            continue;
        }

        if byte == b'/' && bytes.get(idx + 1) == Some(&b'/') {
            in_line_comment = true;
            idx += 2;
            col += 2;
            continue;
        }
        if byte == b'/' && bytes.get(idx + 1) == Some(&b'*') {
            in_block_comment = true;
            idx += 2;
            col += 2;
            continue;
        }
        if matches!(byte, b'"' | b'\'') {
            in_string = Some(byte);
            idx += 1;
            col += 1;
            continue;
        }

        if bytes[idx..].starts_with(from_bytes)
            && boundary_before(bytes, idx)
            && boundary_after(bytes, idx + from_bytes.len())
        {
            let line_text = source
                .lines()
                .nth((line - 1) as usize)
                .unwrap_or("")
                .to_string();
            let node_kind = if line_text.trim_start().starts_with("use ") {
                "use_declaration".to_string()
            } else {
                "scoped_identifier".to_string()
            };
            out.push(Replacement {
                start: idx,
                end: idx + from_bytes.len(),
                line,
                col,
                before: line_text,
                node_kind,
            });
            idx += from_bytes.len();
            col += from_bytes.len() as u32;
            continue;
        }

        if byte == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
        idx += 1;
    }

    out
}

fn apply_replacements(source: &str, replacements: &[Replacement], to: &str) -> String {
    let mut updated = source.to_string();
    for replacement in replacements.iter().rev() {
        updated.replace_range(replacement.start..replacement.end, to);
    }
    updated
}

fn boundary_before(bytes: &[u8], start: usize) -> bool {
    if start == 0 {
        return true;
    }
    !is_path_ident(bytes[start - 1]) && bytes[start - 1] != b':'
}

fn boundary_after(bytes: &[u8], end: usize) -> bool {
    match bytes.get(end).copied() {
        None => true,
        Some(byte) => !is_path_ident(byte),
    }
}

fn is_path_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn path_matches_scope(path: &str, scope: &str) -> bool {
    path == scope || path.starts_with(&format!("{scope}/")) || path.ends_with(&format!("/{scope}"))
}

fn looks_like_partial_group_use(source: &str, from: &str) -> bool {
    let tail = from.rsplit("::").next().unwrap_or(from);
    let needle = format!("{tail}::");
    source
        .lines()
        .any(|line| line.trim_start().starts_with("use ") && line.contains('{') && line.contains(&needle))
}

fn collect_partial_group_warnings(
    source: &str,
    path: &str,
    from: &str,
    partial_mode: &str,
) -> Vec<PathRenameUnhandled> {
    if partial_mode == "skip" {
        return Vec::new();
    }
    let tail = from.rsplit("::").next().unwrap_or(from);
    let needle = format!("{tail}::");
    source
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| {
            let trimmed = line.trim_start();
            if !trimmed.starts_with("use ") || !trimmed.contains('{') || trimmed.contains(from) {
                return None;
            }
            if !trimmed.contains(&needle) {
                return None;
            }
            Some(PathRenameUnhandled {
                path: path.to_string(),
                line: idx as u32 + 1,
                reason: if partial_mode == "split" {
                    "partial grouped import requires manual split".to_string()
                } else {
                    "partial grouped import skipped; rerun with handle_partial_group_imports=split after review".to_string()
                },
            })
        })
        .collect()
}

fn duplicate_use_collisions(source: &str, path: &str) -> Vec<PathRenameCollision> {
    let mut seen = BTreeMap::<String, u32>::new();
    let mut collisions = Vec::new();
    for (idx, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if !trimmed.starts_with("use ") {
            continue;
        }
        if let Some(first_line) = seen.insert(trimmed.to_string(), idx as u32 + 1) {
            collisions.push(PathRenameCollision {
                path: path.to_string(),
                line: idx as u32 + 1,
                reason: format!(
                    "duplicate import after rewrite (first seen on line {first_line}): {trimmed}"
                ),
            });
        }
    }
    collisions
}

pub fn summarize_path_rename(plan: &PathRenamePlan) -> (usize, usize) {
    let file_count = plan.files_affected.len();
    let change_count = plan
        .files_affected
        .iter()
        .map(|file| file.changes.len())
        .sum::<usize>();
    (file_count, change_count)
}

pub fn affected_paths(plan: &PathRenamePlan) -> BTreeSet<String> {
    plan.files_affected
        .iter()
        .map(|file| file.path.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_prefixed_paths_outside_comments() {
        let source = "use crate::storage::DocStore;\n// crate::storage::DocStore\nlet x = crate::storage::DocStore;\n";
        let replacements = collect_path_replacements(source, "crate::storage");
        assert_eq!(replacements.len(), 2);
    }

    #[test]
    fn duplicate_imports_are_reported() {
        let collisions = duplicate_use_collisions(
            "use codesight_storage::DocStore;\nuse codesight_storage::DocStore;\n",
            "src/lib.rs",
        );
        assert_eq!(collisions.len(), 1);
    }
}
