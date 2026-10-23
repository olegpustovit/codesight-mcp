//! **SCSS / Sass / Less / Stylus** overrides: preprocessor-specific constructs and dialects need
//! dedicated tree-sitter passes rather than the plain CSS query-pack patterns alone.
use anyhow::Result;
use tree_sitter::Node;

use dowser_core::symbols::SymbolKind;

use crate::extract_tree_sitter::{first_line, node_text, parse_tree, push_symbol, source_slice};

pub struct ScssExtractor;

impl ScssExtractor {
    pub fn language_name(&self) -> &'static str {
        "scss"
    }

    pub fn supported_extensions(&self) -> &'static [&'static str] {
        &["scss"]
    }

    pub fn extract(
        &self,
        file_path: &str,
        source: &str,
    ) -> anyhow::Result<Vec<dowser_core::StoredSymbol>> {
        let tree = parse_tree(
            crate::builtin_grammar("scss").expect("scss grammar registered with language-support"),
            source,
        )?;
        let mut symbols = Vec::new();
        visit(tree.root_node(), file_path, source, &mut symbols)?;
        symbols.sort_by_key(|s| (s.start_byte, s.end_byte));
        Ok(symbols)
    }
}

/// Extract a symbol name from a node given the handler strategy. Returns
/// `Ok(None)` when this node should be skipped without descending.
type NameExtractor = fn(Node<'_>, &str) -> Result<Option<String>>;

struct NodeRule {
    kind: &'static str,
    symbol_kind: SymbolKind,
    extractor: NameExtractor,
}

const RULES: &[NodeRule] = &[
    NodeRule {
        kind: "declaration",
        symbol_kind: SymbolKind::Constant,
        extractor: extract_declaration_variable,
    },
    NodeRule {
        kind: "mixin_statement",
        symbol_kind: SymbolKind::Function,
        extractor: extract_prefixed_field_name,
    },
    NodeRule {
        kind: "function_statement",
        symbol_kind: SymbolKind::Function,
        extractor: extract_prefixed_field_name,
    },
    NodeRule {
        kind: "placeholder",
        symbol_kind: SymbolKind::Class,
        extractor: extract_placeholder_head,
    },
    NodeRule {
        kind: "rule_set",
        symbol_kind: SymbolKind::Class,
        extractor: extract_selectors_head,
    },
    NodeRule {
        kind: "media_statement",
        symbol_kind: SymbolKind::Type,
        extractor: extract_first_line_signature,
    },
    NodeRule {
        kind: "supports_statement",
        symbol_kind: SymbolKind::Type,
        extractor: extract_first_line_signature,
    },
    NodeRule {
        kind: "keyframes_statement",
        symbol_kind: SymbolKind::Function,
        extractor: extract_keyframes_name,
    },
];

fn visit(
    node: Node<'_>,
    file_path: &str,
    source: &str,
    symbols: &mut Vec<dowser_core::StoredSymbol>,
) -> Result<()> {
    if let Some(rule) = RULES.iter().find(|r| r.kind == node.kind()) {
        if let Some(name) = (rule.extractor)(node, source)? {
            push_symbol(
                symbols,
                file_path,
                "scss",
                &[],
                &name,
                rule.symbol_kind,
                node,
                source,
                None,
            )?;
        }
        // Matched rules never descend — SCSS symbols are leaf-scoped.
        return Ok(());
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(child, file_path, source, symbols)?;
    }
    Ok(())
}

fn extract_declaration_variable(node: Node<'_>, source: &str) -> Result<Option<String>> {
    if let Some(prop) = find_child_of_kind(node, "property_name") {
        let raw = node_text(prop, source)?;
        if raw.starts_with('$') {
            return Ok(Some(raw.to_string()));
        }
    }
    if let Some(var) = find_child_of_kind(node, "variable") {
        return Ok(Some(node_text(var, source)?.to_string()));
    }
    Ok(None)
}

fn extract_prefixed_field_name(node: Node<'_>, source: &str) -> Result<Option<String>> {
    let prefix = match node.kind() {
        "mixin_statement" => "@mixin ",
        "function_statement" => "@function ",
        _ => return Ok(None),
    };
    let Some(name_node) = node.child_by_field_name("name") else {
        return Ok(None);
    };
    let raw = node_text(name_node, source)?;
    Ok(Some(format!("{prefix}{raw}")))
}

fn extract_placeholder_head(node: Node<'_>, source: &str) -> Result<Option<String>> {
    let raw = source_slice(node, source)?
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .split('{')
        .next()
        .unwrap_or("")
        .trim();
    if raw.is_empty() {
        return Ok(None);
    }
    Ok(Some(raw.to_string()))
}

fn extract_selectors_head(node: Node<'_>, source: &str) -> Result<Option<String>> {
    let Some(sel) = find_child_of_kind(node, "selectors") else {
        return Ok(None);
    };
    let name = source_slice(sel, source)?
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return Ok(None);
    }
    Ok(Some(name))
}

fn extract_first_line_signature(node: Node<'_>, source: &str) -> Result<Option<String>> {
    let sig = first_line(node, source)?;
    Ok(Some(sig.trim_end_matches('{').trim().to_string()))
}

fn extract_keyframes_name(node: Node<'_>, source: &str) -> Result<Option<String>> {
    let Some(kn) = find_child_of_kind(node, "keyframes_name") else {
        return Ok(None);
    };
    let raw = node_text(kn, source)?;
    Ok(Some(format!("@keyframes {raw}")))
}

fn find_child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).find(|c| c.kind() == kind)
}

/// SassExtractor returns empty symbols (text-search only, no tree-sitter grammar).
pub struct SassExtractor;
impl SassExtractor {
    pub fn language_name(&self) -> &'static str {
        "sass"
    }
    pub fn supported_extensions(&self) -> &'static [&'static str] {
        &["sass"]
    }
    pub fn extract(&self, _: &str, _: &str) -> anyhow::Result<Vec<dowser_core::StoredSymbol>> {
        Ok(Vec::new())
    }
}

/// LessExtractor returns empty symbols (text-search only).
pub struct LessExtractor;
impl LessExtractor {
    pub fn language_name(&self) -> &'static str {
        "less"
    }
    pub fn supported_extensions(&self) -> &'static [&'static str] {
        &["less"]
    }
    pub fn extract(&self, _: &str, _: &str) -> anyhow::Result<Vec<dowser_core::StoredSymbol>> {
        Ok(Vec::new())
    }
}

/// StylExtractor returns empty symbols (text-search only).
pub struct StylExtractor;
impl StylExtractor {
    pub fn language_name(&self) -> &'static str {
        "styl"
    }
    pub fn supported_extensions(&self) -> &'static [&'static str] {
        &["styl"]
    }
    pub fn extract(&self, _: &str, _: &str) -> anyhow::Result<Vec<dowser_core::StoredSymbol>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scss_extracts_variables_and_mixins() {
        let src = "$primary-color: #333;\n$font-size: 16px;\n\n@mixin flex-center {\n  display: flex;\n}\n\n@function px-to-rem($px) {\n  @return $px / 16 * 1rem;\n}\n\n%visually-hidden {\n  position: absolute;\n}\n\n.container { max-width: 1200px; }\n";
        let s = ScssExtractor.extract("main.scss", src).expect("scss");
        let names: Vec<&str> = s.iter().map(|x| x.name.as_str()).collect();
        assert!(
            names.iter().any(|n| n.contains("primary-color")),
            "missing $primary-color: {names:?}"
        );
        assert!(
            names.iter().any(|n| n.contains("font-size")),
            "missing $font-size: {names:?}"
        );
        assert!(
            names.iter().any(|n| n.contains("flex-center")),
            "missing mixin: {names:?}"
        );
        assert!(
            names.iter().any(|n| n.contains("px-to-rem")),
            "missing function: {names:?}"
        );
        assert!(
            names.iter().any(|n| n.contains("visually-hidden")),
            "missing placeholder: {names:?}"
        );
        assert!(names.contains(&".container"), "missing rule set: {names:?}");
    }

    #[test]
    fn scss_empty_file() {
        let s = ScssExtractor.extract("empty.scss", "").expect("scss");
        assert!(s.is_empty());
    }
}
