//! Rust-specific path-rename engine — rewrites `use`/scoped-path prefixes
//! across the workspace, preserving Rust comment and string context, and
//! understanding grouped `use PREFIX::{A, B};` shapes.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::Context;
use ignore::WalkBuilder;

use crate::path_rename::{
    PathRenameChange, PathRenameCollision, PathRenameEngine, PathRenameFilePlan, PathRenamePlan,
    PathRenameUnhandled,
};

pub struct RustPathRenameEngine;

pub static RUST_PATH_RENAME_ENGINE: RustPathRenameEngine = RustPathRenameEngine;

impl PathRenameEngine for RustPathRenameEngine {
    fn preview(
        &self,
        root: &Path,
        from: &str,
        to: &str,
        scope: Option<&str>,
        partial_mode: &str,
    ) -> anyhow::Result<PathRenamePlan> {
        let mut files_affected = Vec::new();
        let mut unhandled = Vec::new();
        let mut collisions = Vec::new();

        for rel_path in rust_files(root, scope)? {
            let abs_path = root.join(&rel_path);
            let source = fs::read_to_string(&abs_path)
                .with_context(|| format!("read '{}'", abs_path.display()))?;
            if !source.contains(from)
                && !looks_like_partial_group_use(&source, from)
                && !looks_like_group_head_match(&source, from)
            {
                continue;
            }

            let file_unhandled =
                collect_partial_group_warnings(&source, &rel_path, from, partial_mode);
            if partial_mode != "skip" {
                unhandled.extend(file_unhandled);
            }

            let mut replacements = collect_path_replacements(&source, from);
            replacements.extend(collect_group_use_replacements(&source, from, to));
            // Sort by start so `apply_replacements` (which walks in reverse) keeps
            // byte offsets valid even when the two scanners emit overlapping
            // regions. Dedupe identical spans as a safety net — the scanners are
            // disjoint by construction (plain occurrences vs. inside-brace items),
            // but identical `(start, end)` pairs would double-apply.
            replacements.sort_by_key(|r| (r.start, r.end));
            replacements.dedup_by(|a, b| a.start == b.start && a.end == b.end);
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
    /// When set, this is spliced into the source at `start..end` instead of
    /// the global `to`. Used for grouped `use` items where only a single
    /// item inside `{ ... }` gets rewritten; the splice value is the tail
    /// of `to` relative to the group's shared prefix.
    custom_text: Option<String>,
    /// When set, the `PathRenameChange.after` field renders this string
    /// verbatim instead of `before.replacen(from, to, 1)`. Needed for
    /// grouped edits because the full `from` path doesn't appear on the
    /// matched line.
    after_override: Option<String>,
}

impl Replacement {
    fn after(&self, from: &str, to: &str) -> String {
        if let Some(ref forced) = self.after_override {
            return forced.clone();
        }
        self.before.replacen(from, to, 1)
    }

    fn replacement_text<'a>(&'a self, to: &'a str) -> &'a str {
        self.custom_text.as_deref().unwrap_or(to)
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
                custom_text: None,
                after_override: None,
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
        updated.replace_range(
            replacement.start..replacement.end,
            replacement.replacement_text(to),
        );
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

/// Expand `use PREFIX::{A, B, sub::C};` into per-item edits when `from` lives
/// inside the brace group. Emits a [`Replacement`] per matching item.
fn collect_group_use_replacements(source: &str, from: &str, to: &str) -> Vec<Replacement> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();

    let mut line_start = 0usize;
    while line_start < bytes.len() {
        let line_end = bytes[line_start..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|off| line_start + off)
            .unwrap_or(bytes.len());

        let next_line_start = line_end + 1;

        let mut cursor = line_start;
        while cursor < line_end && matches!(bytes[cursor], b' ' | b'\t') {
            cursor += 1;
        }
        let body_start = if bytes.get(cursor..cursor + 4) == Some(b"use ") {
            Some(cursor + 4)
        } else if bytes.get(cursor..cursor + 8) == Some(b"pub use ") {
            Some(cursor + 8)
        } else {
            None
        };
        let Some(body_start) = body_start else {
            line_start = next_line_start;
            continue;
        };

        let mut brace_open: Option<usize> = None;
        let mut i = body_start;
        let mut depth_angle: i32 = 0;
        while i < line_end {
            match bytes[i] {
                b'<' => depth_angle += 1,
                b'>' => depth_angle -= 1,
                b'{' if depth_angle == 0 => {
                    brace_open = Some(i);
                    break;
                }
                b';' => break,
                _ => {}
            }
            i += 1;
        }
        let Some(brace_open) = brace_open else {
            line_start = next_line_start;
            continue;
        };

        if brace_open < 2 || &bytes[brace_open - 2..brace_open] != b"::" {
            line_start = next_line_start;
            continue;
        }

        let head_raw = match std::str::from_utf8(&bytes[body_start..brace_open - 2]) {
            Ok(s) => s.trim(),
            Err(_) => {
                line_start = next_line_start;
                continue;
            }
        };
        if head_raw.is_empty() {
            line_start = next_line_start;
            continue;
        }
        let head_prefix = format!("{head_raw}::");

        let wanted_item = match from.strip_prefix(&head_prefix) {
            Some(s) => s,
            None => {
                line_start = next_line_start;
                continue;
            }
        };
        let to_tail = match to.strip_prefix(&head_prefix) {
            Some(s) => s,
            None => {
                line_start = next_line_start;
                continue;
            }
        };

        let Some(brace_close) = match_brace(bytes, brace_open) else {
            line_start = next_line_start;
            continue;
        };

        let inside_start = brace_open + 1;
        let inside_end = brace_close;
        let mut cursor = inside_start;
        while cursor < inside_end {
            let item_end = next_top_level_comma(bytes, cursor, inside_end).unwrap_or(inside_end);
            let raw = &source[cursor..item_end];
            let leading_ws = raw.len() - raw.trim_start().len();
            let trailing_ws = raw.len() - raw.trim_end().len();
            let item_start = cursor + leading_ws;
            let item_stop = item_end - trailing_ws;
            let item_text = &source[item_start..item_stop];
            if !item_text.is_empty() {
                let ident = strip_as_alias(item_text);
                if ident == wanted_item {
                    let (line, col) = byte_pos_to_line_col(source, item_start);
                    let before = source
                        .lines()
                        .nth((line - 1) as usize)
                        .unwrap_or("")
                        .to_string();
                    let after = before.replacen(ident, to_tail, 1);
                    out.push(Replacement {
                        start: item_start,
                        end: item_start + ident.len(),
                        line,
                        col,
                        before,
                        node_kind: "grouped_use_item".to_string(),
                        custom_text: Some(to_tail.to_string()),
                        after_override: Some(after),
                    });
                }
            }
            cursor = if item_end < inside_end {
                item_end + 1
            } else {
                inside_end
            };
        }

        let jumped_past = brace_close + 1;
        while line_start < jumped_past {
            let ln_end = bytes[line_start..]
                .iter()
                .position(|&b| b == b'\n')
                .map(|off| line_start + off)
                .unwrap_or(bytes.len());
            if ln_end >= jumped_past {
                line_start = jumped_past.max(next_line_start);
                break;
            }
            line_start = ln_end + 1;
        }
        if line_start <= line_end {
            line_start = next_line_start;
        }
    }

    out
}

fn match_brace(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn next_top_level_comma(bytes: &[u8], start: usize, end: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = start;
    while i < end {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b',' if depth == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

fn strip_as_alias(item: &str) -> &str {
    match item.split_once(" as ") {
        Some((ident, _)) => ident.trim(),
        None => item.trim(),
    }
}

fn byte_pos_to_line_col(source: &str, pos: usize) -> (u32, u32) {
    let mut line = 1u32;
    let mut col = 1u32;
    for (i, b) in source.bytes().enumerate() {
        if i >= pos {
            break;
        }
        if b == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn looks_like_group_head_match(source: &str, from: &str) -> bool {
    let Some((head, _)) = from.rsplit_once("::") else {
        return false;
    };
    let needle = format!("{head}::{{");
    source.contains(&needle)
}

fn looks_like_partial_group_use(source: &str, from: &str) -> bool {
    let tail = from.rsplit("::").next().unwrap_or(from);
    let needle = format!("{tail}::");
    source.lines().any(|line| {
        line.trim_start().starts_with("use ") && line.contains('{') && line.contains(&needle)
    })
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
            "use dowser_storage::DocStore;\nuse dowser_storage::DocStore;\n",
            "src/lib.rs",
        );
        assert_eq!(collisions.len(), 1);
    }
}
