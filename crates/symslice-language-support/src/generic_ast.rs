use anyhow::Result;
use tree_sitter::Node;

use symslice_core::StoredSymbol;
use symslice_core::symbols::{ExtractionTier, SymbolKind};

use crate::extract_tree_sitter::{node_text, parse_tree, push_symbol};
use crate::query_pack_catalog::GrammarDefinition;

pub struct GenericAstExtractor {
    language_name: &'static str,
    extensions: &'static [&'static str],
    grammar: tree_sitter::Language,
}

impl GenericAstExtractor {
    pub fn from_grammar(definition: GrammarDefinition) -> Self {
        Self {
            language_name: definition.language_name(),
            extensions: definition.extensions(),
            grammar: definition.language(),
        }
    }

    fn visit(
        &self,
        node: Node<'_>,
        file_path: &str,
        source: &str,
        scopes: &mut Vec<ScopeEntry>,
        symbols: &mut Vec<StoredSymbol>,
    ) -> Result<()> {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            let candidate = classify_candidate(child, source, scopes);
            if let Some(candidate) = candidate {
                let scope_names: Vec<String> =
                    scopes.iter().map(|scope| scope.name.clone()).collect();
                push_symbol(
                    symbols,
                    file_path,
                    self.language_name,
                    &scope_names,
                    &candidate.name,
                    candidate.kind,
                    child,
                    source,
                    None,
                )?;

                if candidate.opens_scope {
                    scopes.push(ScopeEntry {
                        name: candidate.name,
                    });
                    self.visit(child, file_path, source, scopes, symbols)?;
                    scopes.pop();
                    continue;
                }
            }

            self.visit(child, file_path, source, scopes, symbols)?;
        }
        Ok(())
    }
}

impl GenericAstExtractor {
    pub fn language_name(&self) -> &'static str {
        self.language_name
    }

    pub fn supported_extensions(&self) -> &'static [&'static str] {
        self.extensions
    }

    pub fn extraction_tier(&self) -> ExtractionTier {
        ExtractionTier::GenericAstFallback
    }

    pub fn extract(&self, file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
        let tree = parse_tree(self.grammar.clone(), source)?;
        let mut symbols = Vec::new();
        self.visit(
            tree.root_node(),
            file_path,
            source,
            &mut Vec::new(),
            &mut symbols,
        )?;
        for symbol in &mut symbols {
            symbol.extraction_tier = ExtractionTier::GenericAstFallback;
        }
        symbols.sort_by_key(|symbol| (symbol.start_byte, symbol.end_byte, symbol.name.clone()));
        symbols.dedup_by(|right, left| {
            left.start_byte == right.start_byte
                && left.end_byte == right.end_byte
                && left.name == right.name
                && left.kind == right.kind
        });
        Ok(symbols)
    }
}

#[derive(Clone)]
struct ScopeEntry {
    name: String,
}

struct GenericCandidate {
    name: String,
    kind: SymbolKind,
    opens_scope: bool,
}

fn classify_candidate(
    node: Node<'_>,
    source: &str,
    scopes: &[ScopeEntry],
) -> Option<GenericCandidate> {
    let node_kind = node.kind().to_ascii_lowercase();
    let (kind, opens_scope) = classify_symbol_kind(&node_kind, !scopes.is_empty())?;
    let name = extract_name(node, source)?;
    Some(GenericCandidate {
        name,
        kind,
        opens_scope,
    })
}

fn classify_symbol_kind(node_kind: &str, inside_scope: bool) -> Option<(SymbolKind, bool)> {
    if matches!(
        node_kind,
        "class_declaration"
            | "struct_declaration"
            | "record_declaration"
            | "object_declaration"
            | "module_declaration"
            | "namespace_definition"
            | "namespace_declaration"
    ) {
        return Some((SymbolKind::Class, true));
    }

    if node_kind.contains("interface")
        || node_kind.contains("enum")
        || node_kind.contains("trait")
        || node_kind.contains("type_definition")
        || node_kind.contains("type_alias")
        || node_kind == "type_declaration"
    {
        return Some((SymbolKind::Type, true));
    }

    if node_kind.contains("method") {
        return Some((SymbolKind::Method, true));
    }

    if node_kind.contains("function")
        || node_kind.contains("lambda")
        || node_kind.contains("procedure")
        || node_kind.contains("macro_definition")
    {
        return Some((
            if inside_scope {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            },
            true,
        ));
    }

    if node_kind.contains("const")
        || node_kind.contains("property")
        || node_kind.contains("field")
        || node_kind.contains("variable")
        || node_kind.contains("assignment")
        || node_kind.contains("binding")
    {
        return Some((SymbolKind::Constant, false));
    }

    None
}

fn extract_name(node: Node<'_>, source: &str) -> Option<String> {
    for field in ["name", "declarator", "label", "key"] {
        if let Some(candidate) = node
            .child_by_field_name(field)
            .and_then(|child| first_name_text(child, source))
        {
            return Some(candidate);
        }
    }

    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find_map(|child| first_name_text(child, source))
}

fn first_name_text(node: Node<'_>, source: &str) -> Option<String> {
    if let Ok(text) = node_text(node, source) {
        let trimmed = text.trim().trim_matches('"').trim_matches('\'');
        if looks_like_symbol_name(trimmed) {
            return Some(trimmed.to_string());
        }
    }

    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find_map(|child| first_name_text(child, source))
}

fn looks_like_symbol_name(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 96
        && !text.contains('\n')
        && !text.contains(char::is_whitespace)
        && text
            .chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | ':' | '.' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_pack_catalog::GrammarCatalog;

    #[test]
    fn generic_ast_extracts_luau_basics() {
        let grammar = GrammarCatalog::builtin()
            .find_by_grammar("luau")
            .expect("luau grammar");
        let extractor = GenericAstExtractor::from_grammar(grammar);
        let source = r#"
type RaidState = "idle" | "active"

local config = {}

function RaidService.new()
    return {}
end
"#;
        let symbols = extractor.extract("service.luau", source).expect("luau");
        assert!(symbols.iter().any(|symbol| symbol.name == "RaidState"));
        assert!(symbols.iter().any(|symbol| symbol.name == "config"));
        assert!(symbols.iter().any(|symbol| symbol.name.ends_with("new")));
    }
}
