//! **SQL** extractor: **tree-sitter first** ([`tree_sitter_sequel`]), then named fallbacks.
//!
//! ## Pipeline ([`crate::extractors::fallback_pipeline`])
//!
//! 1. Parse with `tree-sitter-sequel` and walk DDL / CTE nodes ([`visit`]).
//! 2. Append dbt/Jinja symbols and regex CTE gap-fill ([`fallback`]).
//! 3. [`sort_symbols_by_span`] + [`dedup_symbols_by_start_byte`] so structural
//!    hits win when spans collide.

mod fallback;

use anyhow::Result;
use tree_sitter::Node;

use crate::extractors::fallback_pipeline::{dedup_symbols_by_start_byte, sort_symbols_by_span};
use crate::model::SymbolKind;

use super::common::{node_text, parse_tree, push_symbol};

pub struct SqlExtractor;

impl super::LanguageExtractor for SqlExtractor {
    fn language_name(&self) -> &'static str {
        "sql"
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["sql"]
    }

    fn extract(
        &self,
        file_path: &str,
        source: &str,
    ) -> anyhow::Result<Vec<crate::model::StoredSymbol>> {
        let tree = parse_tree(tree_sitter_sequel::LANGUAGE, source)?;
        let mut symbols = Vec::new();
        visit(tree.root_node(), file_path, source, &mut symbols)?;
        fallback::extract_dbt_macros(file_path, source, &mut symbols)?;
        fallback::extract_ctes(file_path, source, &mut symbols)?;
        sort_symbols_by_span(&mut symbols);
        dedup_symbols_by_start_byte(&mut symbols);
        Ok(symbols)
    }
}

fn visit(
    node: Node<'_>,
    file_path: &str,
    source: &str,
    symbols: &mut Vec<crate::model::StoredSymbol>,
) -> Result<()> {
    match node.kind() {
        "create_table_statement" | "create_table" => {
            if let Some(name) = find_object_name(node, source) {
                let docstring = extract_preceding_comment(node, source);
                push_symbol(
                    symbols,
                    file_path,
                    "sql",
                    &[],
                    &name,
                    SymbolKind::Type,
                    node,
                    source,
                    docstring,
                )?;
            }
            return Ok(());
        }
        "create_view_statement" | "create_view" => {
            if let Some(name) = find_object_name(node, source) {
                push_symbol(
                    symbols,
                    file_path,
                    "sql",
                    &[],
                    &name,
                    SymbolKind::Type,
                    node,
                    source,
                    None,
                )?;
            }
            return Ok(());
        }
        "create_function_statement" | "create_function" => {
            if let Some(name) = find_object_name(node, source) {
                push_symbol(
                    symbols,
                    file_path,
                    "sql",
                    &[],
                    &name,
                    SymbolKind::Function,
                    node,
                    source,
                    None,
                )?;
                if let Some(sym) = symbols.last_mut() {
                    let full_text = node_text(node, source).unwrap_or("");
                    let mut sig_lines = Vec::new();
                    for line in full_text.lines() {
                        let trimmed = line.trim();
                        let upper = trimmed.to_ascii_uppercase();
                        if upper.starts_with("BEGIN")
                            || upper.starts_with("AS $$")
                            || upper.starts_with("LANGUAGE")
                        {
                            break;
                        }
                        sig_lines.push(trimmed);
                    }
                    if sig_lines.len() > 1 {
                        sym.signature = sig_lines.join(" ");
                    }
                }
            }
            return Ok(());
        }
        "create_index_statement" | "create_index" => {
            if let Some(name) = find_object_name(node, source) {
                push_symbol(
                    symbols,
                    file_path,
                    "sql",
                    &[],
                    &name,
                    SymbolKind::Type,
                    node,
                    source,
                    None,
                )?;
            }
            return Ok(());
        }
        "create_schema_statement" | "create_schema" => {
            if let Some(name) = find_object_name(node, source) {
                push_symbol(
                    symbols,
                    file_path,
                    "sql",
                    &[],
                    &name,
                    SymbolKind::Type,
                    node,
                    source,
                    None,
                )?;
            }
            return Ok(());
        }
        "cte" | "common_table_expression" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source)?.to_string();
                push_symbol(
                    symbols,
                    file_path,
                    "sql",
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

fn find_object_name(node: Node<'_>, source: &str) -> Option<String> {
    if let Some(name_node) = node.child_by_field_name("name") {
        return node_text(name_node, source).ok().map(|s| s.to_string());
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "identifier" | "object_reference" | "table_reference" | "schema_name"
            | "relation_expr" => {
                if let Ok(t) = node_text(child, source) {
                    let name = t.trim().to_string();
                    if !name.is_empty()
                        && !name.eq_ignore_ascii_case("TABLE")
                        && !name.eq_ignore_ascii_case("VIEW")
                        && !name.eq_ignore_ascii_case("FUNCTION")
                        && !name.eq_ignore_ascii_case("INDEX")
                        && !name.eq_ignore_ascii_case("SCHEMA")
                        && !name.eq_ignore_ascii_case("CREATE")
                        && !name.eq_ignore_ascii_case("OR")
                        && !name.eq_ignore_ascii_case("REPLACE")
                        && !name.eq_ignore_ascii_case("IF")
                        && !name.eq_ignore_ascii_case("NOT")
                        && !name.eq_ignore_ascii_case("EXISTS")
                    {
                        return Some(name);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn extract_preceding_comment(node: Node<'_>, source: &str) -> Option<String> {
    let start = node.start_byte();
    if start == 0 {
        return None;
    }
    let before = &source[..start];
    let lines: Vec<&str> = before.lines().rev().collect();
    let mut comments = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with("--") {
            comments.push(trimmed.trim_start_matches('-').trim());
        } else if trimmed.is_empty() {
            continue;
        } else {
            break;
        }
    }
    if comments.is_empty() {
        return None;
    }
    comments.reverse();
    Some(comments.join(" "))
}

#[cfg(test)]
mod tests {
    use super::SqlExtractor;
    use crate::extractors::LanguageExtractor;

    #[test]
    fn sql_extracts_ddl() {
        let src = r#"
CREATE TABLE orders (
    id INT PRIMARY KEY,
    status VARCHAR(50)
);

CREATE VIEW active_orders AS
SELECT * FROM orders WHERE status = 'active';

CREATE FUNCTION get_order_total(order_id INT)
RETURNS DECIMAL AS $$
BEGIN
    RETURN 0;
END;
$$ LANGUAGE plpgsql;
"#;
        let s = SqlExtractor.extract("schema.sql", src).expect("sql");
        assert!(
            s.iter().any(|x| x.name == "orders"),
            "missing orders table: {:#?}",
            s.iter().map(|x| &x.name).collect::<Vec<_>>()
        );
    }
}
