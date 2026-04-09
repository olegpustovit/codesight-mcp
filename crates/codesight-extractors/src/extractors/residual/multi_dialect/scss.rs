//! **SCSS / Sass / Less / Stylus** overrides: preprocessor-specific constructs and dialects need
//! dedicated tree-sitter passes rather than the plain CSS query-pack patterns alone.
use anyhow::Result;
use tree_sitter::Node;

use crate::core::symbols::SymbolKind;

use crate::extractors::common::{first_line, node_text, parse_tree, push_symbol, source_slice};

pub struct ScssExtractor;

impl crate::extractors::LanguageExtractor for ScssExtractor {
    fn language_name(&self) -> &'static str {
        "scss"
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["scss"]
    }

    fn extract(
        &self,
        file_path: &str,
        source: &str,
    ) -> anyhow::Result<Vec<crate::core::pipeline::StoredSymbol>> {
        let tree = parse_tree(tree_sitter_scss::language(), source)?;
        let mut symbols = Vec::new();
        visit(tree.root_node(), file_path, source, &mut symbols)?;
        symbols.sort_by_key(|s| (s.start_byte, s.end_byte));
        Ok(symbols)
    }
}

fn visit(
    node: Node<'_>,
    file_path: &str,
    source: &str,
    symbols: &mut Vec<crate::core::pipeline::StoredSymbol>,
) -> Result<()> {
    match node.kind() {
        "declaration" => {
            // SCSS variables: $name: value; — property_name is a child
            if let Some(prop) = find_child_of_kind(node, "property_name") {
                let raw = node_text(prop, source)?;
                if raw.starts_with('$') {
                    push_symbol(
                        symbols,
                        file_path,
                        "scss",
                        &[],
                        raw,
                        SymbolKind::Constant,
                        node,
                        source,
                        None,
                    )?;
                    return Ok(());
                }
            }
            // Also check for variable child directly
            if let Some(var) = find_child_of_kind(node, "variable") {
                let raw = node_text(var, source)?;
                push_symbol(
                    symbols,
                    file_path,
                    "scss",
                    &[],
                    raw,
                    SymbolKind::Constant,
                    node,
                    source,
                    None,
                )?;
                return Ok(());
            }
        }
        "mixin_statement" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let raw = node_text(name_node, source)?;
                let name = format!("@mixin {raw}");
                push_symbol(
                    symbols,
                    file_path,
                    "scss",
                    &[],
                    &name,
                    SymbolKind::Function,
                    node,
                    source,
                    None,
                )?;
            }
            return Ok(());
        }
        "function_statement" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let raw = node_text(name_node, source)?;
                let name = format!("@function {raw}");
                push_symbol(
                    symbols,
                    file_path,
                    "scss",
                    &[],
                    &name,
                    SymbolKind::Function,
                    node,
                    source,
                    None,
                )?;
            }
            return Ok(());
        }
        "placeholder" => {
            // %name — the identifier child has the name without %
            let raw = source_slice(node, source)?
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .split('{')
                .next()
                .unwrap_or("")
                .trim();
            if !raw.is_empty() {
                push_symbol(
                    symbols,
                    file_path,
                    "scss",
                    &[],
                    raw,
                    SymbolKind::Class,
                    node,
                    source,
                    None,
                )?;
            }
            return Ok(());
        }
        "rule_set" => {
            // selectors is a child type, not a field
            if let Some(sel) = find_child_of_kind(node, "selectors") {
                let name = source_slice(sel, source)?
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !name.is_empty() {
                    push_symbol(
                        symbols,
                        file_path,
                        "scss",
                        &[],
                        &name,
                        SymbolKind::Class,
                        node,
                        source,
                        None,
                    )?;
                }
            }
            return Ok(());
        }
        "media_statement" => {
            let sig = first_line(node, source)?;
            let name = sig.trim_end_matches('{').trim().to_string();
            push_symbol(
                symbols,
                file_path,
                "scss",
                &[],
                &name,
                SymbolKind::Type,
                node,
                source,
                None,
            )?;
            return Ok(());
        }
        "supports_statement" => {
            let sig = first_line(node, source)?;
            let name = sig.trim_end_matches('{').trim().to_string();
            push_symbol(
                symbols,
                file_path,
                "scss",
                &[],
                &name,
                SymbolKind::Type,
                node,
                source,
                None,
            )?;
            return Ok(());
        }
        "keyframes_statement" => {
            if let Some(kn) = find_child_of_kind(node, "keyframes_name") {
                let raw = node_text(kn, source)?;
                let name = format!("@keyframes {raw}");
                push_symbol(
                    symbols,
                    file_path,
                    "scss",
                    &[],
                    &name,
                    SymbolKind::Function,
                    node,
                    source,
                    None,
                )?;
            }
            return Ok(());
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(child, file_path, source, symbols)?;
    }
    Ok(())
}

fn find_child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).find(|c| c.kind() == kind)
}

/// SassExtractor returns empty symbols (text-search only, no tree-sitter grammar).
pub struct SassExtractor;
impl crate::extractors::LanguageExtractor for SassExtractor {
    fn language_name(&self) -> &'static str {
        "sass"
    }
    fn supported_extensions(&self) -> &'static [&'static str] {
        &["sass"]
    }
    fn extract(
        &self,
        _: &str,
        _: &str,
    ) -> anyhow::Result<Vec<crate::core::pipeline::StoredSymbol>> {
        Ok(Vec::new())
    }
}

/// LessExtractor returns empty symbols (text-search only).
pub struct LessExtractor;
impl crate::extractors::LanguageExtractor for LessExtractor {
    fn language_name(&self) -> &'static str {
        "less"
    }
    fn supported_extensions(&self) -> &'static [&'static str] {
        &["less"]
    }
    fn extract(
        &self,
        _: &str,
        _: &str,
    ) -> anyhow::Result<Vec<crate::core::pipeline::StoredSymbol>> {
        Ok(Vec::new())
    }
}

/// StylExtractor returns empty symbols (text-search only).
pub struct StylExtractor;
impl crate::extractors::LanguageExtractor for StylExtractor {
    fn language_name(&self) -> &'static str {
        "styl"
    }
    fn supported_extensions(&self) -> &'static [&'static str] {
        &["styl"]
    }
    fn extract(
        &self,
        _: &str,
        _: &str,
    ) -> anyhow::Result<Vec<crate::core::pipeline::StoredSymbol>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractors::LanguageExtractor;

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
