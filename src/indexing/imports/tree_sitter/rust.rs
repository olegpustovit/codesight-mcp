//! Rust import extraction via tree-sitter (primary path).
//!
//! Covers `use`, `pub use`, and `extern crate` declarations. The output shape
//! mirrors Python: `specifier` is the module path up to (but excluding) the
//! final leaf, and `names` holds the leaf identifiers being brought into
//! scope. Example mappings:
//!
//! | Source                              | ImportEdge                                |
//! |-------------------------------------|-------------------------------------------|
//! | `use serde::Serialize;`             | `{ serde, ["Serialize"] }`                |
//! | `use std::collections::HashMap;`    | `{ std::collections, ["HashMap"] }`       |
//! | `use tokio::{sync::Mutex, task};`   | `{ tokio, ["sync::Mutex", "task"] }`      |
//! | `use rayon::prelude::*;`            | `{ rayon::prelude, [] }`                  |
//! | `use crate::foo::Bar;`              | `{ crate::foo, ["Bar"] }`                 |
//! | `extern crate proc_macro;`          | `{ proc_macro, [] }`                      |

use tree_sitter::Node;

use crate::extractors::common::{parse_tree, source_slice};

use super::super::ImportEdge;

pub fn extract_rust_imports_ts(source: &str) -> Option<Vec<ImportEdge>> {
    let tree = parse_tree(tree_sitter_rust::LANGUAGE, source).ok()?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }
    let mut edges = Vec::new();
    walk(root, source, &mut edges);
    Some(edges)
}

fn walk(node: Node<'_>, source: &str, edges: &mut Vec<ImportEdge>) {
    match node.kind() {
        "use_declaration" => {
            if let Some(arg) = node.child_by_field_name("argument") {
                collect_use(arg, source, "", edges);
            }
            return;
        }
        "extern_crate_declaration" => {
            if let Some(name_n) = node.child_by_field_name("name")
                && let Ok(name) = source_slice(name_n, source)
            {
                edges.push(ImportEdge {
                    specifier: name.to_string(),
                    names: Vec::new(),
                });
            }
            return;
        }
        "mod_item" => {
            // `mod foo;` (no body) declares that `foo` lives in a sibling
            // file — emit a `self::foo` edge so the resolver maps it to
            // `foo.rs` / `foo/mod.rs` under the current file's module dir.
            // `mod foo { … }` (has body) is inline and generates no edge,
            // but we still descend into it to collect nested `use` edges.
            let has_body = node.child_by_field_name("body").is_some();
            if !has_body
                && let Some(name_n) = node.child_by_field_name("name")
                && let Ok(name) = source_slice(name_n, source)
            {
                edges.push(ImportEdge {
                    specifier: format!("self::{name}"),
                    names: Vec::new(),
                });
                return;
            }
        }
        _ => {}
    }
    let mut i = 0u32;
    while let Some(c) = node.child(i) {
        walk(c, source, edges);
        i += 1;
    }
}

/// Recursive descent over a use-tree argument, accumulating the outer path
/// `prefix` (as `a::b::c`) as it crosses `scoped_use_list` boundaries.
fn collect_use(node: Node<'_>, source: &str, prefix: &str, edges: &mut Vec<ImportEdge>) {
    match node.kind() {
        "scoped_use_list" => {
            let path_segment = node
                .child_by_field_name("path")
                .and_then(|p| path_text(p, source));
            let new_prefix = join_prefix(prefix, path_segment.as_deref());
            if let Some(list) = node.child_by_field_name("list") {
                collect_use(list, source, &new_prefix, edges);
            }
        }
        "use_list" => {
            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i as u32) {
                    collect_use(child, source, prefix, edges);
                }
            }
        }
        "identifier" | "crate" | "self" | "super" => {
            if let Ok(name) = source_slice(node, source) {
                edges.push(ImportEdge {
                    specifier: prefix.to_string(),
                    names: vec![name.to_string()],
                });
            }
        }
        "scoped_identifier" => {
            // Split into (inner_prefix, leaf). Example: std::collections::HashMap
            // inner_prefix = std::collections, leaf = HashMap.
            let leaf = node
                .child_by_field_name("name")
                .and_then(|n| source_slice(n, source).ok())
                .map(|s| s.to_string());
            let inner_prefix = node
                .child_by_field_name("path")
                .and_then(|p| path_text(p, source));
            let full_prefix = join_prefix(prefix, inner_prefix.as_deref());
            if let Some(leaf) = leaf {
                edges.push(ImportEdge {
                    specifier: full_prefix,
                    names: vec![leaf],
                });
            }
        }
        "use_as_clause" => {
            // `use foo::Bar as Baz` — track the real path; the alias matters
            // for name resolution, not dependency direction.
            if let Some(path_n) = node.child_by_field_name("path") {
                collect_use(path_n, source, prefix, edges);
            }
        }
        "use_wildcard" => {
            // `use foo::bar::*` — the leading path is a sibling of the star
            // inside this node, or is captured via prefix. Walk named children
            // to recover it.
            let mut path = None;
            for i in 0..node.named_child_count() {
                if let Some(c) = node.named_child(i as u32)
                    && matches!(
                        c.kind(),
                        "identifier" | "scoped_identifier" | "crate" | "self" | "super"
                    )
                {
                    path = path_text(c, source);
                    break;
                }
            }
            let full = join_prefix(prefix, path.as_deref());
            edges.push(ImportEdge {
                specifier: full,
                names: Vec::new(),
            });
        }
        _ => {}
    }
}

/// Render a path node (identifier / scoped_identifier / crate / self / super)
/// as a `::`-joined string. Returns `None` if the node has no usable text.
fn path_text(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "crate" | "self" | "super" | "metavariable" => {
            source_slice(node, source).ok().map(|s| s.to_string())
        }
        "scoped_identifier" => {
            let leaf = node
                .child_by_field_name("name")
                .and_then(|n| source_slice(n, source).ok())
                .map(|s| s.to_string())?;
            let prefix = node
                .child_by_field_name("path")
                .and_then(|p| path_text(p, source));
            Some(join_prefix(prefix.as_deref().unwrap_or(""), Some(&leaf)))
        }
        _ => source_slice(node, source).ok().map(|s| s.to_string()),
    }
}

fn join_prefix(outer: &str, inner: Option<&str>) -> String {
    match (outer.is_empty(), inner) {
        (true, Some(i)) => i.to_string(),
        (false, Some(i)) => format!("{outer}::{i}"),
        (_, None) => outer.to_string(),
    }
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
    fn simple_use() {
        let edges = extract_rust_imports_ts("use serde::Serialize;\n").unwrap();
        assert_eq!(
            specs(&edges),
            vec![("serde".to_string(), vec!["Serialize".to_string()])]
        );
    }

    #[test]
    fn nested_path() {
        let edges = extract_rust_imports_ts("use std::collections::HashMap;\n").unwrap();
        assert_eq!(
            specs(&edges),
            vec![(
                "std::collections".to_string(),
                vec!["HashMap".to_string()]
            )]
        );
    }

    #[test]
    fn use_list_with_nested() {
        // Nested paths flatten into their full prefix: `sync::Mutex` inside
        // `tokio::{…}` becomes specifier `tokio::sync`, name `Mutex`. This
        // mirrors the handling of `std::collections::HashMap`.
        let edges =
            extract_rust_imports_ts("use tokio::{sync::Mutex, task};\n").unwrap();
        let mut s = specs(&edges);
        s.sort();
        assert_eq!(
            s,
            vec![
                ("tokio".to_string(), vec!["task".to_string()]),
                ("tokio::sync".to_string(), vec!["Mutex".to_string()]),
            ]
        );
    }

    #[test]
    fn wildcard_import() {
        let edges = extract_rust_imports_ts("use rayon::prelude::*;\n").unwrap();
        assert_eq!(
            specs(&edges),
            vec![("rayon::prelude".to_string(), Vec::<String>::new())]
        );
    }

    #[test]
    fn extern_crate() {
        let edges = extract_rust_imports_ts("extern crate proc_macro;\n").unwrap();
        assert_eq!(
            specs(&edges),
            vec![("proc_macro".to_string(), Vec::<String>::new())]
        );
    }

    #[test]
    fn pub_use_is_captured() {
        let edges = extract_rust_imports_ts("pub use crate::foo::Bar;\n").unwrap();
        assert_eq!(
            specs(&edges),
            vec![("crate::foo".to_string(), vec!["Bar".to_string()])]
        );
    }

    #[test]
    fn use_as_preserves_real_path() {
        let edges =
            extract_rust_imports_ts("use serde_json::Value as Json;\n").unwrap();
        assert_eq!(
            specs(&edges),
            vec![("serde_json".to_string(), vec!["Value".to_string()])]
        );
    }

    #[test]
    fn parse_error_returns_none() {
        assert!(extract_rust_imports_ts("use ::::;").is_none());
    }

    #[test]
    fn mod_decl_emits_self_edge() {
        let edges = extract_rust_imports_ts("pub mod service;\nmod models;\n").unwrap();
        let mut s = specs(&edges);
        s.sort();
        assert_eq!(
            s,
            vec![
                ("self::models".to_string(), Vec::<String>::new()),
                ("self::service".to_string(), Vec::<String>::new()),
            ]
        );
    }

    #[test]
    fn inline_mod_emits_no_edge_but_walks_into_body() {
        // `mod inner { use crate::Foo; }` should emit the inner `use` edge
        // but NOT a `self::inner` edge (it's inline, not a file declaration).
        let edges =
            extract_rust_imports_ts("mod inner { use crate::Foo; }\n").unwrap();
        assert_eq!(
            specs(&edges),
            vec![("crate".to_string(), vec!["Foo".to_string()])]
        );
    }
}
