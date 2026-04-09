//! **EJS** embedded JavaScript/HTML needs bracket and partial-token handling; shared SCM packs do
//! not encode this template surface well enough for reliable symbol output.
use anyhow::Result;
use regex::Regex;
use std::sync::LazyLock;

use crate::core::pipeline::StoredSymbol;
use crate::core::symbols::{ExtractionTier, SymbolKind};

use crate::extractors::containers::{RemapOptions, SegmentOrigin, extract_host_segment};

pub struct EjsExtractor;

impl crate::extractors::LanguageExtractor for EjsExtractor {
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

        let masked_javascript = mask_ejs_scriptlets(source);
        let mut delegated = extract_host_segment(
            "javascript",
            file_path,
            source,
            &masked_javascript,
            SegmentOrigin::new(source, 0),
            RemapOptions {
                language_override: Some("ejs"),
                ..Default::default()
            },
        )
        .unwrap_or_default();
        delegated.retain(|symbol| symbol.kind == SymbolKind::Function);
        if delegated.is_empty() {
            delegated.extend(regex_function_symbols(file_path, source));
        }
        symbols.extend(delegated);

        for (line_num, line_start, line_end, line) in source_lines(source) {
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

fn source_lines(source: &str) -> Vec<(u32, usize, usize, &str)> {
    let mut lines = Vec::new();
    let mut byte_offset = 0usize;
    for (index, line) in source.lines().enumerate() {
        let start = byte_offset;
        let end = byte_offset + line.len();
        lines.push(((index + 1) as u32, start, end, line));
        byte_offset += line.len() + 1;
    }
    lines
}

fn regex_function_symbols(file_path: &str, source: &str) -> Vec<StoredSymbol> {
    source_lines(source)
        .into_iter()
        .filter_map(|(line_num, line_start, line_end, line)| {
            let caps = re::FUNCTION.captures(line)?;
            let name = caps.get(1)?.as_str();
            let signature = caps.get(0)?.as_str().trim();
            Some(make_symbol_with_sig(
                file_path,
                name,
                SymbolKind::Function,
                line_num,
                signature,
                line_start,
                line_end,
                source,
            ))
        })
        .collect()
}

fn mask_ejs_scriptlets(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut masked = bytes
        .iter()
        .map(|byte| if *byte == b'\n' { b'\n' } else { b' ' })
        .collect::<Vec<_>>();
    let mut index = 0usize;

    while index + 1 < bytes.len() {
        if bytes[index] == b'<' && bytes[index + 1] == b'%' {
            let open_len = if matches!(bytes.get(index + 2), Some(b'=') | Some(b'-')) {
                3
            } else {
                2
            };
            let mut cursor = index + open_len;
            while cursor + 1 < bytes.len() {
                if bytes[cursor] == b'%' && bytes[cursor + 1] == b'>' {
                    masked[(index + open_len)..cursor]
                        .copy_from_slice(&bytes[(index + open_len)..cursor]);
                    index = cursor + 2;
                    break;
                }
                cursor += 1;
            }
        } else {
            index += 1;
        }
    }

    String::from_utf8(masked).expect("masked EJS buffer should stay utf-8")
}

#[allow(clippy::too_many_arguments)]
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
        extraction_tier: ExtractionTier::ManualOverride,
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

#[allow(clippy::too_many_arguments)]
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
        extraction_tier: ExtractionTier::ManualOverride,
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
    use crate::core::symbols::SymbolKind;
    use crate::extractors::LanguageExtractor;

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
