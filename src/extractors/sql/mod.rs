//! **SQL** extractor: structural symbols come from the shared query-pack engine; dbt/Jinja
//! directives stay in named fallbacks because they are not visible to the sequel grammar.

mod fallback;

use std::sync::LazyLock;

use crate::extractors::engine::catalogs::QueryPackCatalog;
use crate::extractors::engine::scm::ScmExtractor;
use crate::extractors::fallback_pipeline::{dedup_symbols_by_start_byte, sort_symbols_by_span};
use crate::model::ExtractionTier;

pub struct SqlExtractor;

impl super::LanguageExtractor for SqlExtractor {
    fn language_name(&self) -> &'static str {
        "sql"
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["sql"]
    }

    fn extraction_tier(&self) -> ExtractionTier {
        ExtractionTier::CuratedQueryPack
    }

    fn extract(
        &self,
        file_path: &str,
        source: &str,
    ) -> anyhow::Result<Vec<crate::model::StoredSymbol>> {
        let mut symbols = sql_pack()
            .extract(file_path, source)
            .unwrap_or_else(|_| Vec::new());
        enrich_structural_symbols(source, &mut symbols);
        fallback::extract_dbt_macros(file_path, source, &mut symbols)?;
        fallback::extract_ctes(file_path, source, &mut symbols)?;
        sort_symbols_by_span(&mut symbols);
        dedup_symbols_by_start_byte(&mut symbols);
        Ok(symbols)
    }
}

fn sql_pack() -> &'static ScmExtractor {
    static SQL_PACK: LazyLock<ScmExtractor> = LazyLock::new(|| {
        let catalog = QueryPackCatalog::builtin();
        let pack = catalog
            .find_by_pack_id("sql")
            .expect("missing sql query pack");
        ScmExtractor::from_pack(pack).expect("failed to compile sql query pack")
    });
    &SQL_PACK
}

fn enrich_structural_symbols(source: &str, symbols: &mut [crate::model::StoredSymbol]) {
    for symbol in symbols {
        let original_signature = symbol.signature.clone();
        if symbol.kind == crate::model::SymbolKind::Function
            && original_signature
                .to_ascii_uppercase()
                .starts_with("CREATE FUNCTION")
        {
            symbol.signature = function_signature(&symbol.source_text);
        }
        if let Some(docstring) = extract_preceding_comment(source, symbol.start_byte) {
            symbol.summary = docstring;
        } else if symbol.summary == original_signature {
            symbol.summary = symbol.signature.clone();
        }
    }
}

fn function_signature(source_text: &str) -> String {
    let mut signature_lines = Vec::new();
    for line in source_text.lines() {
        let trimmed = line.trim();
        let upper = trimmed.to_ascii_uppercase();
        if upper.starts_with("BEGIN") || upper.starts_with("AS $$") || upper.starts_with("LANGUAGE")
        {
            break;
        }
        signature_lines.push(trimmed);
    }
    if signature_lines.len() > 1 {
        signature_lines.join(" ")
    } else {
        signature_lines
            .into_iter()
            .next()
            .unwrap_or_default()
            .to_string()
    }
}

fn extract_preceding_comment(source: &str, start_byte: usize) -> Option<String> {
    let before = &source[..start_byte.min(source.len())];
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
