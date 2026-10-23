//! Default-impl support for `Language::reachability_edges` — runs a
//! per-edge-kind regex set over file content and emits symbol-only edges.
//!
//! Used by languages that don't ship a tree-sitter grammar. Languages with
//! a grammar override `reachability_edges` directly and ignore this
//! module.

use std::sync::OnceLock;

use dashmap::DashMap;

use symslice_core::EdgeKind;
use symslice_core::StructuralEdge;

use crate::imports_types::ImportEdge;
use crate::traits::Language;

/// Translate a language's structural [`ImportEdge`] set into symbol-shaped
/// reachability edges. Each import contributes:
///
/// - one `ImportStatement` edge per named import (e.g. `from x import A, B`
///   yields `A` and `B`)
/// - one `ImportStatement` edge for the root package segment (e.g. `serde`
///   from `serde::de::T`)
/// - for bare imports without explicit names, the trailing leaf segment
///   of the specifier (e.g. `Bar` from `use foo::Bar;`)
///
/// Used by `Language::reachability_edges`'s default impl. Languages with
/// richer reachability shapes (Rust) override the trait method instead.
pub fn edges_from_imports<L: Language + ?Sized>(
    lang: &L,
    source_file: &str,
    imports: &[ImportEdge],
) -> Vec<StructuralEdge> {
    let mut out: Vec<StructuralEdge> = Vec::new();
    for e in imports {
        let root = lang.root_package_from_specifier(&e.specifier);
        if !root.is_empty() && root != e.specifier {
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
        if e.names.is_empty() {
            // Use the specifier itself as a symbol edge — many languages
            // (Go, Python `import foo`) reference the whole module by its
            // name rather than naming children.
            if !e.specifier.is_empty() {
                out.push(StructuralEdge::new_symbol(
                    source_file,
                    &e.specifier,
                    EdgeKind::ImportStatement,
                ));
            }
        }
    }
    out
}

/// Compile each `(EdgeKind, pattern)` pair lazily and run them across
/// `content`. The first capture group of each regex is taken as the
/// target *symbol name*; lines starting with `//` / `#` are skipped.
pub fn extract_with_patterns(
    source_file: &str,
    content: &str,
    patterns: &[(EdgeKind, &'static str)],
) -> Vec<StructuralEdge> {
    let compiled = compile_patterns(patterns);
    let mut out: Vec<StructuralEdge> = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with('#') {
            continue;
        }
        for (kind, re) in &compiled {
            for caps in re.captures_iter(line) {
                if let Some(m) = caps.get(1) {
                    let name = m.as_str();
                    if !name.is_empty() {
                        out.push(StructuralEdge::new_symbol(source_file, name, *kind));
                    }
                }
            }
        }
    }
    out
}

type CompiledSet = Vec<(EdgeKind, regex::Regex)>;

fn compile_patterns(patterns: &[(EdgeKind, &'static str)]) -> CompiledSet {
    // Cache compiled regex sets keyed by the slice's static address. Same
    // language returns the same `&'static [...]`, so this never recompiles.
    static CACHE: OnceLock<DashMap<usize, CompiledSet>> = OnceLock::new();
    let cache = CACHE.get_or_init(DashMap::new);
    let key = patterns.as_ptr() as usize;
    if let Some(existing) = cache.get(&key) {
        return existing.iter().map(|(k, r)| (*k, r.clone())).collect();
    }
    let built: CompiledSet = patterns
        .iter()
        .filter_map(|(k, p)| regex::Regex::new(p).ok().map(|r| (*k, r)))
        .collect();
    cache.entry(key).or_insert_with(|| built.clone());
    built
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruby_style_require_emits_symbol_edge() {
        let patterns: &[(EdgeKind, &str)] = &[(
            EdgeKind::ImportStatement,
            r#"^\s*require\s+['"]([^'"]+)['"]"#,
        )];
        let edges = extract_with_patterns("a.rb", "require 'foo/bar'\n", patterns);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].target_symbol.as_deref(), Some("foo/bar"));
        assert_eq!(edges[0].edge_kind, EdgeKind::ImportStatement);
    }

    #[test]
    fn comments_are_skipped() {
        let patterns: &[(EdgeKind, &str)] =
            &[(EdgeKind::ImportStatement, r#"require\s+['"]([^'"]+)['"]"#)];
        let edges = extract_with_patterns("a.rb", "// require 'foo'\n# require 'bar'", patterns);
        assert!(edges.is_empty());
    }
}
