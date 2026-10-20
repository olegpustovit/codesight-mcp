use regex::Regex;
use std::sync::LazyLock;

use codesight_core::StoredSymbol;
use codesight_core::symbols::{ExtractionTier, SymbolKind};

pub(super) fn strip_xml_tags(s: &str) -> String {
    static XML_TAG: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<[^>]+>").expect("infallible: al xml tag strip regex"));
    XML_TAG.replace_all(s, "").trim().to_string()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn make_symbol(
    file_path: &str,
    name: &str,
    kind: SymbolKind,
    line_num: u32,
    line_text: &str,
    start_byte: usize,
    end_byte: usize,
    source: &str,
    scopes: &[String],
    docstring: Option<String>,
    _decorators: &[String],
) -> StoredSymbol {
    let qualified_name = if scopes.is_empty() {
        name.to_string()
    } else {
        format!("{}.{}", scopes[scopes.len() - 1], name)
    };
    let source_text = source
        .get(start_byte..end_byte)
        .unwrap_or(line_text)
        .to_string();
    let source_hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
    let signature = line_text.trim().to_string();
    let summary = docstring.unwrap_or_else(|| signature.clone());
    StoredSymbol {
        id: format!("{file_path}::{qualified_name}#{}", kind.as_str()),
        extraction_tier: ExtractionTier::ManualOverride,
        language: codesight_core::AL,
        file_path: file_path.to_string(),
        qualified_name,
        name: name.to_string(),
        kind,
        signature,
        summary,
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
pub(super) fn make_symbol_with_sig(
    file_path: &str,
    name: &str,
    kind: SymbolKind,
    line_num: u32,
    signature: &str,
    start_byte: usize,
    end_byte: usize,
    source: &str,
    scopes: &[String],
    docstring: Option<String>,
) -> StoredSymbol {
    let qualified_name = if scopes.is_empty() {
        name.to_string()
    } else {
        format!("{}.{}", scopes[scopes.len() - 1], name)
    };
    let source_text = source
        .get(start_byte..end_byte)
        .unwrap_or(signature)
        .to_string();
    let source_hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
    let summary = docstring.unwrap_or_else(|| signature.to_string());
    StoredSymbol {
        id: format!("{file_path}::{qualified_name}#{}", kind.as_str()),
        extraction_tier: ExtractionTier::ManualOverride,
        language: codesight_core::AL,
        file_path: file_path.to_string(),
        qualified_name,
        name: name.to_string(),
        kind,
        signature: signature.to_string(),
        summary,
        start_line: line_num,
        end_line: line_num,
        start_byte,
        end_byte,
        source_hash,
        source_text,
        file_size_bytes: source.len() as u64,
    }
}
