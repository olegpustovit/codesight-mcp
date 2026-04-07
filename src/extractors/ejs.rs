//! **EJS** embedded JavaScript/HTML needs bracket and partial-token handling; shared SCM packs do
//! not encode this template surface well enough for reliable symbol output.
use anyhow::Result;
use regex::Regex;
use std::sync::LazyLock;

use crate::model::{StoredSymbol, SymbolKind};

pub struct EjsExtractor;

impl super::LanguageExtractor for EjsExtractor {
    fn language_name(&self) -> &'static str {
        "ejs"
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["ejs"]
    }

    fn extract(&self, file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
        let mut symbols = Vec::new();

        // Synthetic template symbol: always emitted, named from the file stem.
        let template_name = std::path::Path::new(file_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("template");
        symbols.push(make_symbol(
            file_path,
            template_name,
            SymbolKind::Template,
            1,
            source.lines().next().unwrap_or(""),
            0,
            source.len(),
            source,
        ));

        let lines: Vec<&str> = source.lines().collect();
        let mut byte_offset = 0usize;

        for (i, line) in lines.iter().enumerate() {
            let line_num = (i + 1) as u32;
            let line_start = byte_offset;
            let line_end = byte_offset + line.len();

            // JS function inside scriptlet: <% function name(args) { %>
            if let Some(caps) = re::FUNCTION.captures(line) {
                let name = caps.get(1).unwrap().as_str();
                let sig = caps.get(0).unwrap().as_str().trim();
                symbols.push(make_symbol_with_sig(
                    file_path,
                    name,
                    SymbolKind::Function,
                    line_num,
                    sig,
                    line_start,
                    line_end,
                    source,
                ));
            }

            // Include references: <%- include('path') %> or <%- include('path', data) %>
            if let Some(caps) = re::INCLUDE.captures(line) {
                let path = caps.get(1).unwrap().as_str();
                symbols.push(make_symbol(
                    file_path,
                    path,
                    SymbolKind::Import,
                    line_num,
                    line.trim(),
                    line_start,
                    line_end,
                    source,
                ));
            }

            byte_offset += line.len() + 1;
        }

        symbols.sort_by_key(|s| (s.start_byte, s.end_byte));
        Ok(symbols)
    }
}

mod re {
    use super::*;

    // Match: <% function name(args) ... %>  or  function name(args) { inside scriptlets
    pub static FUNCTION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?:<%[-=]?\s*)?function\s+(\w+)\s*\([^)]*\)").unwrap());

    // Match: <%- include('path') %> or <%- include('path', ...) %>
    // Supports both single and double quotes
    pub static INCLUDE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"<%[-=]?\s*include\s*\(\s*['"]([^'"]+)['"]\s*(?:,[^)]*)?\)"#).unwrap()
    });
}

fn make_symbol(
    file_path: &str,
    name: &str,
    kind: SymbolKind,
    line_num: u32,
    line_text: &str,
    start_byte: usize,
    end_byte: usize,
    source: &str,
) -> StoredSymbol {
    let source_text = source
        .get(start_byte..end_byte)
        .unwrap_or(line_text)
        .to_string();
    let source_hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
    let signature = line_text.trim().to_string();
    StoredSymbol {
        id: format!("{file_path}::{name}#{}", kind.as_str()),
        language: "ejs".to_string(),
        file_path: file_path.to_string(),
        qualified_name: name.to_string(),
        name: name.to_string(),
        kind,
        signature: signature.clone(),
        summary: signature,
        start_line: line_num,
        end_line: line_num,
        start_byte,
        end_byte,
        source_hash,
        source_text,
        file_size_bytes: source.len() as u64,
    }
}

fn make_symbol_with_sig(
    file_path: &str,
    name: &str,
    kind: SymbolKind,
    line_num: u32,
    signature: &str,
    start_byte: usize,
    end_byte: usize,
    source: &str,
) -> StoredSymbol {
    let source_text = source
        .get(start_byte..end_byte)
        .unwrap_or(signature)
        .to_string();
    let source_hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
    StoredSymbol {
        id: format!("{file_path}::{name}#{}", kind.as_str()),
        language: "ejs".to_string(),
        file_path: file_path.to_string(),
        qualified_name: name.to_string(),
        name: name.to_string(),
        kind,
        signature: signature.to_string(),
        summary: signature.to_string(),
        start_line: line_num,
        end_line: line_num,
        start_byte,
        end_byte,
        source_hash,
        source_text,
        file_size_bytes: source.len() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::EjsExtractor;
    use crate::extractors::LanguageExtractor;
    use crate::model::SymbolKind;

    #[test]
    fn ejs_extracts_template_and_functions() {
        let src = r#"<!DOCTYPE html>
<html>
<head><title><%= title %></title></head>
<body>
  <%- include('partials/header', { user: user }) %>

  <% function formatDate(date) { %>
    <span><%= date.toLocaleDateString() %></span>
  <% } %>

  <%- include('partials/footer') %>
</body>
</html>
"#;
        let syms = EjsExtractor.extract("views/index.ejs", src).unwrap();
        assert!(
            syms.iter()
                .any(|s| s.name == "index" && s.kind == SymbolKind::Template)
        );
        assert!(
            syms.iter()
                .any(|s| s.name == "formatDate" && s.kind == SymbolKind::Function)
        );
        assert!(
            syms.iter()
                .any(|s| s.name == "partials/header" && s.kind == SymbolKind::Import)
        );
        assert!(
            syms.iter()
                .any(|s| s.name == "partials/footer" && s.kind == SymbolKind::Import)
        );
    }

    #[test]
    fn ejs_no_scriptlets_produces_template_only() {
        let src = "<html><body><h1>Hello</h1></body></html>";
        let syms = EjsExtractor.extract("static.ejs", src).unwrap();
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].kind, SymbolKind::Template);
        assert_eq!(syms[0].name, "static");
    }
}
