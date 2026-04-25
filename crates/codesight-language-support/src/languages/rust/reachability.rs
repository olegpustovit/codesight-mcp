//! Rust-specific reachability extraction. Emits structural edges for
//! `pub use` re-exports, `mod`/`pub mod` declarations, `crate::a::b::fn`
//! / `self::child::fn` path references, and `=> self.method(...)` /
//! `=> Self::method(...)` enum-match-arm dispatch.
//!
//! Replaces the file-scanning augmentation passes that used to live in
//! `codesight_analysis::dead_code::reachability` — same patterns, run
//! once per file at ingest time and persisted instead of re-scanning the
//! whole repo on every dead-code call.

use codesight_core::{EdgeKind, StructuralEdge};

use crate::imports_types::ImportEdge;

pub(super) fn reachability_edges(content: &str, source_file: &str) -> Vec<StructuralEdge> {
    let mut out: Vec<StructuralEdge> = Vec::new();
    extract_pub_use_reexports(content, source_file, &mut out);
    extract_mod_declarations(content, source_file, &mut out);
    extract_path_references(content, source_file, &mut out);
    extract_match_arm_dispatch(content, source_file, &mut out);
    extract_imports_as_symbol_edges(content, source_file, &mut out);
    out
}

/// `pub use mod::Symbol` / `pub use mod::{A, B}` / `pub use mod::*`.
/// Emits one `ReExport` edge per re-exported item with the bare item
/// name as `target_symbol`, plus one with the leaf module segment as
/// `target_symbol` for `mod::*` glob form.
fn extract_pub_use_reexports(content: &str, source_file: &str, out: &mut Vec<StructuralEdge>) {
    for line in content.lines() {
        let trimmed = line.trim_start();
        let body = match strip_pub_use_prefix(trimmed) {
            Some(body) => body,
            None => continue,
        };
        let body = body.trim_end_matches(';').trim();
        let (prefix_path, items_inside_braces, has_braces) = match body.find('{') {
            Some(idx) => {
                let prefix = &body[..idx];
                let inside = body[idx + 1..].trim_end_matches('}');
                (
                    prefix.trim_end_matches("::").trim(),
                    inside.to_string(),
                    true,
                )
            }
            None => (body, String::new(), false),
        };
        let prefix_segments: Vec<&str> = prefix_path
            .split("::")
            .filter(|s| !s.is_empty() && *s != "crate" && *s != "self" && *s != "super")
            .collect();

        let item_iter: Vec<&str> = if has_braces {
            items_inside_braces
                .split(',')
                .map(|s| s.trim().split(" as ").next().unwrap_or("").trim())
                .filter(|s| !s.is_empty() && *s != "self" && *s != "*")
                .collect()
        } else if prefix_segments.len() >= 2
            && !prefix_segments.last().is_some_and(|s| *s == "*")
        {
            vec![prefix_segments[prefix_segments.len() - 1]]
        } else if prefix_segments.last().is_some_and(|s| *s == "*") {
            // `pub use mod::*` — fold the module segment as a symbol edge so
            // the module itself reads as referenced.
            if prefix_segments.len() >= 2 {
                vec![prefix_segments[prefix_segments.len() - 2]]
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        for item in item_iter {
            out.push(StructuralEdge::new_symbol(
                source_file,
                item,
                EdgeKind::ReExport,
            ));
        }
        // Always also emit an edge for the *module* segment so file-level
        // resolution downstream can chain through. We emit the leaf module
        // as a symbol edge — `dead_code` joins by name to find files
        // owning a `mod_name` symbol.
        if let Some(module_leaf) = prefix_segments.last() {
            if *module_leaf != "*" {
                out.push(StructuralEdge::new_symbol(
                    source_file,
                    *module_leaf,
                    EdgeKind::ReExport,
                ));
            }
        }
    }
}

fn strip_pub_use_prefix(line: &str) -> Option<&str> {
    line.strip_prefix("pub use ")
        .or_else(|| line.strip_prefix("pub(crate) use "))
        .or_else(|| line.strip_prefix("pub(super) use "))
}

/// `mod child;` / `pub mod child;`. Emits a `ModuleDecl` edge with
/// `target_symbol = child`. Inline modules (`mod foo { ... }`) are
/// skipped — they don't reference a separate file.
fn extract_mod_declarations(content: &str, source_file: &str, out: &mut Vec<StructuralEdge>) {
    for line in content.lines() {
        let trimmed = line.trim_start();
        let after_mod = trimmed
            .strip_prefix("mod ")
            .or_else(|| trimmed.strip_prefix("pub mod "))
            .or_else(|| trimmed.strip_prefix("pub(crate) mod "))
            .or_else(|| trimmed.strip_prefix("pub(super) mod "));
        let Some(rest) = after_mod else {
            continue;
        };
        let name = rest.trim_end_matches(';').trim();
        if name.contains('{') || name.contains(' ') || name.is_empty() {
            continue;
        }
        out.push(StructuralEdge::new_symbol(
            source_file,
            name,
            EdgeKind::ModuleDecl,
        ));
    }
}

/// `crate::a::b::fn`, `self::child::fn`, `super::sib::Type`. Emits a
/// `PathReference` edge per resolved leaf segment (the module name); the
/// dead-code reader resolves names back to defining files.
fn extract_path_references(content: &str, source_file: &str, out: &mut Vec<StructuralEdge>) {
    for token in iter_path_tokens(content) {
        let parts: Vec<&str> = token.split("::").collect();
        if parts.len() < 2 {
            continue;
        }
        let head = parts[0];
        if head != "self" && head != "super" && head != "crate" {
            continue;
        }
        // `crate::a::b::fn` — module segments are everything between head
        // and the trailing identifier.
        let segments = &parts[1..parts.len().saturating_sub(1)];
        if segments.is_empty() {
            continue;
        }
        // Emit the *deepest* module name as a symbol edge — that's the
        // module-owning file the original Rust pass tried to resolve.
        let leaf = segments[segments.len() - 1];
        if !leaf.is_empty() {
            out.push(StructuralEdge::new_symbol(
                source_file,
                leaf,
                EdgeKind::PathReference,
            ));
        }
    }
}

/// `=> self.NAME(`, `=> Self::NAME(`, `=> self.NAME?` match-arm method
/// dispatch.
fn extract_match_arm_dispatch(content: &str, source_file: &str, out: &mut Vec<StructuralEdge>) {
    for line in content.lines() {
        let line = match line.find("//") {
            Some(idx) => &line[..idx],
            None => line,
        };
        let bytes = line.as_bytes();
        let mut i = 0usize;
        while i + 1 < bytes.len() {
            if bytes[i] == b'=' && bytes[i + 1] == b'>' {
                let mut j = i + 2;
                while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j += 1;
                }
                let after = &line[j..];
                let candidate = after
                    .strip_prefix("self.")
                    .or_else(|| after.strip_prefix("Self::"));
                if let Some(rest) = candidate {
                    let end = rest
                        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .unwrap_or(rest.len());
                    if end > 0 {
                        let name = &rest[..end];
                        let next_char = rest[end..].chars().next();
                        if matches!(next_char, Some('(') | Some('?') | None | Some(',')) {
                            out.push(StructuralEdge::new_symbol(
                                source_file,
                                name,
                                EdgeKind::MatchDispatch,
                            ));
                        }
                    }
                }
                i = j;
            } else {
                i += 1;
            }
        }
    }
}

/// Translate every Rust import edge into one `ImportStatement` edge per
/// imported name (so the dead-code reader sees the symbol as referenced)
/// plus one for the root crate / module segment.
fn extract_imports_as_symbol_edges(
    content: &str,
    source_file: &str,
    out: &mut Vec<StructuralEdge>,
) {
    let edges: Vec<ImportEdge> = super::imports::extract(content);
    for e in &edges {
        // Root crate / module name (before the first `::`).
        let root = e.specifier.split("::").next().unwrap_or("");
        if !root.is_empty() && root != "crate" && root != "self" && root != "super" {
            out.push(StructuralEdge::new_symbol(
                source_file,
                root,
                EdgeKind::ImportStatement,
            ));
        }
        for name in &e.names {
            if !name.is_empty() && name != "*" {
                out.push(StructuralEdge::new_symbol(
                    source_file,
                    name,
                    EdgeKind::ImportStatement,
                ));
            }
        }
        // Trailing leaf when no brace (`use foo::Bar;`)
        if e.names.is_empty() {
            if let Some(leaf) = e.specifier.rsplit("::").next() {
                if !leaf.is_empty() && leaf != "*" && leaf != root {
                    out.push(StructuralEdge::new_symbol(
                        source_file,
                        leaf,
                        EdgeKind::ImportStatement,
                    ));
                }
            }
        }
    }
}

/// Yield Rust-shaped `head::seg::seg` path tokens. Same logic as the
/// pre-redesign `iter_path_tokens` — we keep it local because it's the
/// only consumer.
fn iter_path_tokens(source: &str) -> Vec<&str> {
    let bytes = source.as_bytes();
    let mut out: Vec<&str> = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if !(b.is_ascii_alphabetic() || b == b'_') {
            i += 1;
            continue;
        }
        if i > 0 {
            let prev = bytes[i - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                i += 1;
                continue;
            }
        }
        let start = i;
        while i < bytes.len() {
            let c = bytes[i];
            if c.is_ascii_alphanumeric() || c == b'_' {
                i += 1;
            } else {
                break;
            }
        }
        let mut had_colon = false;
        loop {
            if i + 1 < bytes.len() && bytes[i] == b':' && bytes[i + 1] == b':' {
                let after = i + 2;
                if after < bytes.len()
                    && (bytes[after].is_ascii_alphabetic() || bytes[after] == b'_')
                {
                    i = after;
                    while i < bytes.len() {
                        let c = bytes[i];
                        if c.is_ascii_alphanumeric() || c == b'_' {
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    had_colon = true;
                    continue;
                }
            }
            break;
        }
        if had_colon {
            out.push(&source[start..i]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(edges: &[StructuralEdge]) -> Vec<(EdgeKind, &str)> {
        edges
            .iter()
            .map(|e| (e.edge_kind, e.target_symbol.as_deref().unwrap_or("")))
            .collect()
    }

    #[test]
    fn pub_use_extracts_named_item() {
        let edges = reachability_edges("pub use foo::Bar;\n", "x.rs");
        let pairs = kinds(&edges);
        assert!(pairs.contains(&(EdgeKind::ReExport, "Bar")));
    }

    #[test]
    fn mod_decl_emits_module_decl_edge() {
        let edges = reachability_edges("pub mod child;\nmod helper;\n", "x.rs");
        let pairs = kinds(&edges);
        assert!(pairs.contains(&(EdgeKind::ModuleDecl, "child")));
        assert!(pairs.contains(&(EdgeKind::ModuleDecl, "helper")));
    }

    #[test]
    fn match_arm_self_method_emits_match_dispatch() {
        let src = "
            match x {
                A => self.do_a(),
                B => Self::do_b(),
            }
        ";
        let edges = reachability_edges(src, "x.rs");
        let pairs = kinds(&edges);
        assert!(pairs.contains(&(EdgeKind::MatchDispatch, "do_a")));
        assert!(pairs.contains(&(EdgeKind::MatchDispatch, "do_b")));
    }

    #[test]
    fn crate_path_reference_picks_module_segment() {
        let edges = reachability_edges("let _ = crate::a::b::fn_name();\n", "x.rs");
        let pairs = kinds(&edges);
        assert!(pairs.contains(&(EdgeKind::PathReference, "b")));
    }

    #[test]
    fn glob_pub_use_records_module_leaf() {
        let edges = reachability_edges("pub use foo::bar::*;\n", "x.rs");
        let pairs = kinds(&edges);
        assert!(pairs.iter().any(|(k, _)| *k == EdgeKind::ReExport));
    }
}
