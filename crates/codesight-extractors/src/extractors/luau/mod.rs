//! **Luau** (Roblox dialect): uses the Luau grammar but needs Luau-specific capture/qualification
//! rules beyond what the generic Lua query-pack path would express.
use anyhow::Result;
use tree_sitter::Node;

use crate::core::symbols::SymbolKind;

use crate::extractors::common::{node_text, parse_tree, push_symbol, push_symbol_prequalified};

pub struct LuauExtractor;

impl crate::extractors::LanguageExtractor for LuauExtractor {
    fn language_name(&self) -> &'static str {
        "luau"
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["luau"]
    }

    fn extract(
        &self,
        file_path: &str,
        source: &str,
    ) -> anyhow::Result<Vec<crate::core::pipeline::StoredSymbol>> {
        let tree = parse_tree(
            codesight_language_support::builtin_grammar("luau")
                .expect("luau grammar registered with language-support"),
            source,
        )?;
        let mut symbols = Vec::new();
        visit(
            tree.root_node(),
            file_path,
            source,
            &mut Vec::new(),
            &mut symbols,
        )?;
        symbols.sort_by_key(|s| (s.start_byte, s.end_byte));
        Ok(symbols)
    }
}

fn visit(
    node: Node<'_>,
    file_path: &str,
    source: &str,
    scopes: &mut Vec<String>,
    symbols: &mut Vec<crate::core::pipeline::StoredSymbol>,
) -> Result<()> {
    match node.kind() {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name_text = node_text(name_node, source)?.to_string();
                let docstring = extract_comment_before(node, source);

                if name_text.contains('.') || name_text.contains(':') {
                    let separator = if name_text.contains(':') { ':' } else { '.' };
                    let parts: Vec<&str> = name_text.splitn(2, separator).collect();
                    if parts.len() == 2 {
                        let parent = parts[0].to_string();
                        let method_name = parts[1].to_string();
                        let qualified_name = format!("{parent}{separator}{method_name}");
                        push_symbol_prequalified(
                            symbols,
                            file_path,
                            "luau",
                            qualified_name,
                            &method_name,
                            SymbolKind::Method,
                            node,
                            source,
                            docstring,
                        )?;
                    }
                } else {
                    push_symbol(
                        symbols,
                        file_path,
                        "luau",
                        scopes,
                        &name_text,
                        SymbolKind::Function,
                        node,
                        source,
                        docstring,
                    )?;
                }
            }
            return Ok(());
        }
        "type_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source)?.to_string();
                push_symbol(
                    symbols,
                    file_path,
                    "luau",
                    scopes,
                    &name,
                    SymbolKind::Type,
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
        visit(child, file_path, source, scopes, symbols)?;
    }
    Ok(())
}

fn extract_comment_before(node: Node<'_>, source: &str) -> Option<String> {
    // Walk previous siblings looking for comments
    let mut prev = node.prev_named_sibling();
    let mut comments = Vec::new();
    while let Some(p) = prev {
        if p.kind() == "comment" {
            let text = node_text(p, source).ok()?;
            let cleaned = text.trim_start_matches('-').trim();
            comments.push(cleaned.to_string());
        } else {
            break;
        }
        prev = p.prev_named_sibling();
    }
    if comments.is_empty() {
        return None;
    }
    comments.reverse();
    Some(comments.join(" "))
}

#[cfg(test)]
mod tests {
    use super::LuauExtractor;
    use crate::core::symbols::SymbolKind;
    use crate::extractors::LanguageExtractor;

    #[test]
    fn luau_extracts_functions_types_methods() {
        let src = "export type RaidConfig = {\n    maxPlayers: number,\n    difficulty: string,\n}\n\ntype State = \"idle\" | \"active\" | \"complete\"\n\n-- Start the raid\nfunction RaidService:Start(player: Player, multiplier: number): number\n    return 0\nend\n\nfunction RaidService.new(config: RaidConfig)\n    return {}\nend\n\nlocal function helperFunction()\n    return true\nend\n";
        let s = LuauExtractor
            .extract("RaidService.luau", src)
            .expect("luau");
        let items: Vec<(&str, &SymbolKind)> =
            s.iter().map(|x| (x.name.as_str(), &x.kind)).collect();
        assert!(
            s.iter()
                .any(|x| x.name == "RaidConfig" && x.kind == SymbolKind::Type),
            "missing RaidConfig: {items:?}"
        );
        assert!(
            s.iter()
                .any(|x| x.name == "Start" && x.kind == SymbolKind::Method),
            "missing Start method: {items:?}"
        );
        assert!(
            s.iter()
                .any(|x| x.name == "new" && x.kind == SymbolKind::Method),
            "missing new method: {items:?}"
        );
        assert!(
            s.iter()
                .any(|x| x.name == "helperFunction" && x.kind == SymbolKind::Function),
            "missing helperFunction: {items:?}"
        );
    }
}
