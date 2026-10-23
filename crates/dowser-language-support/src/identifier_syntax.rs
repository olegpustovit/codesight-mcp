//! Identifier tokens from a language grammar, without name-binding claims.
use std::path::Path;

/// Return 1-based positions of exact identifier syntax. Unsupported grammars
/// and erroneous parses return `None`, so callers retain only weak evidence.
/// Traversal follows the tree, preserving identifiers inside interpolations
/// while excluding comment and string-content leaves.
pub fn positions(path: &str, source: &str, name: &str) -> Option<Vec<(u32, u32)>> {
    let registry = crate::builtin_registry();
    let language = registry.get(registry.identify_path(Path::new(path)))?;
    let tree = crate::extract_tree_sitter::parse_tree(language.grammar()?, source).ok()?;
    if tree.root_node().has_error() {
        return None;
    }
    let mut out = Vec::new();
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        if language.is_identifier_node(node.kind())
            && node.utf8_text(source.as_bytes()).ok() == Some(name)
        {
            let point = node.start_position();
            out.push((point.row as u32 + 1, point.column as u32 + 1));
        }
        if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return Some(out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_mentions_and_interpolated_code() {
        let source = "# target()\ns = 'target'\nx = f'{target(1)}'\ny = target(2)\n";
        assert_eq!(
            positions("x.py", source, "target"),
            Some(vec![(3, 8), (4, 5)])
        );
        assert_eq!(positions("x.unknown", source, "target"), None);
        assert_eq!(positions("x.py", "def broken(:", "target"), None);
        assert_eq!(
            positions(
                "x.rs",
                "// target()\nfn run() { let _s = \"target\"; target(); }",
                "target"
            ),
            Some(vec![(2, 31)])
        );
        assert_eq!(
            positions(
                "x.js",
                "/* target() */\nconst x = `target ${target()}`;",
                "target"
            ),
            Some(vec![(2, 21)])
        );
    }
}
