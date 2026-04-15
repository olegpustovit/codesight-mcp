//! Line-based Rust import fallback for files where tree-sitter can't parse.
//!
//! Handles the common shapes emitted by the tree-sitter walker:
//!
//! - `use foo;`
//! - `use foo::Bar;`
//! - `use foo::bar::Baz;`
//! - `use foo::{Bar, Baz};` (single-line only — nested braces are treated as
//!   a single edge with the prefix as specifier, matching the "tree-sitter
//!   failed, best effort" contract)
//! - `use foo::bar::*;`
//! - `use foo::Bar as Alias;`
//! - `pub use …;` / `pub(crate) use …;`
//! - `extern crate foo;`

use crate::imports::ImportEdge;

pub(crate) fn extract_imports(content: &str) -> Vec<ImportEdge> {
    let mut edges = Vec::new();
    for raw in content.lines() {
        let line = strip_inline_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let Some(body) = line_body(line) else {
            continue;
        };
        if let Some(rest) = body.strip_prefix("extern crate ") {
            if let Some(name) = first_ident(rest.trim()) {
                edges.push(ImportEdge {
                    specifier: name.to_string(),
                    names: Vec::new(),
                });
            }
            continue;
        }
        if let Some(rest) = body.strip_prefix("use ") {
            parse_use_line(rest, &mut edges);
            continue;
        }
        // `mod foo;` declarations — skip inline `mod foo { ... }` (anything
        // that isn't terminated with `;` on the same line).
        if let Some(rest) = body.strip_prefix("mod ")
            && !rest.contains('{')
            && let Some(name) = first_ident(rest.trim())
        {
            edges.push(ImportEdge {
                specifier: format!("self::{name}"),
                names: Vec::new(),
            });
        }
    }
    edges
}

/// Strip an `//` line comment without touching `/* */` block comments
/// (which a line-oriented scanner can't sanely handle anyway).
fn strip_inline_comment(line: &str) -> &str {
    match line.find("//") {
        Some(idx) => &line[..idx],
        None => line,
    }
}

/// Strip visibility prefixes (`pub`, `pub(crate)`, `pub(super)`, …) and the
/// trailing semicolon so the remaining body starts with `use …` or
/// `extern crate …`.
fn line_body(line: &str) -> Option<&str> {
    let stripped = line.strip_suffix(';').unwrap_or(line).trim();
    // Peel optional visibility modifier.
    let after_vis = if let Some(rest) = stripped.strip_prefix("pub") {
        let rest = rest.trim_start();
        if let Some(rest) = rest.strip_prefix('(') {
            let close = rest.find(')')?;
            rest[close + 1..].trim_start()
        } else {
            rest
        }
    } else {
        stripped
    };
    Some(after_vis)
}

fn parse_use_line(rest: &str, edges: &mut Vec<ImportEdge>) {
    let rest = rest.trim();
    // Truncate any trailing `as Alias`: the alias is the local binding, not
    // the dependency target.
    let path_end = match rest.find(" as ") {
        Some(i) => &rest[..i],
        None => rest,
    };
    // Multi-import: split at the last `::{` and treat the contents as a list.
    if let Some(brace_open) = path_end.rfind("::{") {
        let prefix = &path_end[..brace_open];
        let after = &path_end[brace_open + 3..];
        let brace_close = after.rfind('}').unwrap_or(after.len());
        let inner = &after[..brace_close];
        let mut any = false;
        for part in inner.split(',') {
            let leaf = part.trim();
            if leaf.is_empty() || leaf.contains('{') {
                // Nested braces aren't parseable line-by-line; fall through.
                continue;
            }
            let (_, last) = split_last_segment(leaf);
            edges.push(ImportEdge {
                specifier: prefix.to_string(),
                names: vec![last.to_string()],
            });
            any = true;
        }
        if any {
            return;
        }
        edges.push(ImportEdge {
            specifier: prefix.to_string(),
            names: Vec::new(),
        });
        return;
    }
    // Wildcard: `foo::bar::*`
    if let Some(prefix) = path_end.strip_suffix("::*") {
        edges.push(ImportEdge {
            specifier: prefix.to_string(),
            names: Vec::new(),
        });
        return;
    }
    // Plain path: `foo::bar::Baz` → specifier `foo::bar`, name `Baz`.
    let (prefix, last) = split_last_segment(path_end);
    if prefix.is_empty() {
        edges.push(ImportEdge {
            specifier: last.to_string(),
            names: Vec::new(),
        });
    } else {
        edges.push(ImportEdge {
            specifier: prefix.to_string(),
            names: vec![last.to_string()],
        });
    }
}

fn split_last_segment(path: &str) -> (&str, &str) {
    match path.rfind("::") {
        Some(i) => (&path[..i], &path[i + 2..]),
        None => ("", path),
    }
}

fn first_ident(s: &str) -> Option<&str> {
    let end = s
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    if end == 0 { None } else { Some(&s[..end]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs(edges: &[ImportEdge]) -> Vec<(String, Vec<String>)> {
        edges
            .iter()
            .map(|e| (e.specifier.clone(), e.names.clone()))
            .collect()
    }

    #[test]
    fn simple_use_with_semicolon() {
        let edges = extract_imports("use serde::Serialize;\n");
        assert_eq!(
            specs(&edges),
            vec![("serde".to_string(), vec!["Serialize".to_string()])]
        );
    }

    #[test]
    fn pub_use_stripped() {
        let edges = extract_imports("pub use crate::foo::Bar;\n");
        assert_eq!(
            specs(&edges),
            vec![("crate::foo".to_string(), vec!["Bar".to_string()])]
        );
    }

    #[test]
    fn pub_crate_use() {
        let edges = extract_imports("pub(crate) use std::io::Read;\n");
        assert_eq!(
            specs(&edges),
            vec![("std::io".to_string(), vec!["Read".to_string()])]
        );
    }

    #[test]
    fn use_list_single_line() {
        let edges = extract_imports("use tokio::{sync::Mutex, task};\n");
        let mut out = specs(&edges);
        out.sort();
        assert_eq!(
            out,
            vec![
                ("tokio".to_string(), vec!["Mutex".to_string()]),
                ("tokio".to_string(), vec!["task".to_string()]),
            ]
        );
    }

    #[test]
    fn wildcard() {
        let edges = extract_imports("use rayon::prelude::*;\n");
        assert_eq!(
            specs(&edges),
            vec![("rayon::prelude".to_string(), Vec::<String>::new())]
        );
    }

    #[test]
    fn extern_crate_line() {
        let edges = extract_imports("extern crate proc_macro;\n");
        assert_eq!(
            specs(&edges),
            vec![("proc_macro".to_string(), Vec::<String>::new())]
        );
    }

    #[test]
    fn as_alias_drops_alias() {
        let edges = extract_imports("use serde_json::Value as Json;\n");
        assert_eq!(
            specs(&edges),
            vec![("serde_json".to_string(), vec!["Value".to_string()])]
        );
    }

    #[test]
    fn inline_comment_tolerated() {
        let edges = extract_imports("use foo::Bar; // a note\n");
        assert_eq!(
            specs(&edges),
            vec![("foo".to_string(), vec!["Bar".to_string()])]
        );
    }

    #[test]
    fn non_import_lines_ignored() {
        let edges =
            extract_imports("fn main() {\n    let x = 1;\n    println!(\"use not real\");\n}\n");
        assert!(edges.is_empty(), "got {edges:?}");
    }

    #[test]
    fn mod_decl_emits_self_edge() {
        let edges = extract_imports("pub mod service;\nmod models;\n");
        let mut out = specs(&edges);
        out.sort();
        assert_eq!(
            out,
            vec![
                ("self::models".to_string(), Vec::<String>::new()),
                ("self::service".to_string(), Vec::<String>::new()),
            ]
        );
    }

    #[test]
    fn inline_mod_block_is_skipped() {
        // `mod inner { ... }` on a single line with a body is NOT a file
        // declaration — no edge.
        let edges = extract_imports("mod inner { fn x() {} }\n");
        assert!(edges.is_empty(), "got {edges:?}");
    }
}
