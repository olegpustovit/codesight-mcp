//! Shared tree-sitter helpers for language extractors (Strategy implementations).

use anyhow::{Context, Result, anyhow};
use tree_sitter::{Node, Tree};

use crate::model::{StoredSymbol, SymbolKind};

pub fn parse_tree(lang: impl Into<tree_sitter::Language>, source: &str) -> Result<Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&lang.into())
        .context("failed to load tree-sitter grammar")?;
    parser
        .parse(source, None)
        .ok_or_else(|| anyhow!("tree-sitter returned no parse tree"))
}

pub fn node_text<'a>(node: Node<'_>, source: &'a str) -> Result<&'a str> {
    node.utf8_text(source.as_bytes())
        .context("tree-sitter node was not valid utf-8")
}

pub fn source_slice<'a>(node: Node<'_>, source: &'a str) -> Result<&'a str> {
    source
        .get(node.start_byte()..node.end_byte())
        .context("tree-sitter byte range was out of bounds")
}

pub fn first_line(node: Node<'_>, source: &str) -> Result<String> {
    Ok(source_slice(node, source)?
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string())
}

/// Stable IDs use dot-qualified names; all extractors use `.` here.
pub fn qualify(scopes: &[String], name: &str) -> String {
    let mut parts: Vec<&str> = scopes.iter().map(String::as_str).collect();
    parts.push(name);
    parts.join(".")
}

pub fn push_symbol(
    symbols: &mut Vec<StoredSymbol>,
    file_path: &str,
    language: &str,
    scopes: &[String],
    name: &str,
    kind: SymbolKind,
    span: Node<'_>,
    source: &str,
    summary: Option<String>,
) -> Result<()> {
    push_symbol_ex(
        symbols, file_path, language, scopes, name, kind, span, source, summary, None,
    )
}

/// Like [`push_symbol`] but accepts a pre-built `qualified_name` instead of
/// computing it from scopes.  Use this when the language's native separator
/// (e.g. Lua/Luau `:` for colon-methods) must be preserved verbatim.
pub fn push_symbol_prequalified(
    symbols: &mut Vec<StoredSymbol>,
    file_path: &str,
    language: &str,
    qualified_name: String,
    name: &str,
    kind: SymbolKind,
    span: Node<'_>,
    source: &str,
    summary: Option<String>,
) -> Result<()> {
    let signature = first_line(span, source)?;
    let summary = summary.unwrap_or_else(|| signature.clone());
    let source_text = source_slice(span, source)?.to_string();
    let source_hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
    symbols.push(StoredSymbol {
        id: format!("{file_path}::{qualified_name}#{}", kind.as_str()),
        language: language.to_string(),
        file_path: file_path.to_string(),
        qualified_name,
        name: name.to_string(),
        kind,
        signature,
        summary,
        start_line: (span.start_position().row + 1) as u32,
        end_line: (span.end_position().row + 1) as u32,
        start_byte: span.start_byte(),
        end_byte: span.end_byte(),
        source_hash,
        source_text,
        file_size_bytes: source.len() as u64,
    });
    Ok(())
}

pub fn push_symbol_ex(
    symbols: &mut Vec<StoredSymbol>,
    file_path: &str,
    language: &str,
    scopes: &[String],
    name: &str,
    kind: SymbolKind,
    span: Node<'_>,
    source: &str,
    summary: Option<String>,
    signature_prefix: Option<&str>,
) -> Result<()> {
    let qualified_name = qualify(scopes, name);
    let mut signature = first_line(span, source)?;
    if let Some(prefix) = signature_prefix {
        signature = format!("{prefix}{signature}");
    }
    let summary = summary.unwrap_or_else(|| signature.clone());
    let source_text = source_slice(span, source)?.to_string();
    let source_hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
    symbols.push(StoredSymbol {
        id: format!("{file_path}::{qualified_name}#{}", kind.as_str()),
        language: language.to_string(),
        file_path: file_path.to_string(),
        qualified_name,
        name: name.to_string(),
        kind,
        signature,
        summary,
        start_line: (span.start_position().row + 1) as u32,
        end_line: (span.end_position().row + 1) as u32,
        start_byte: span.start_byte(),
        end_byte: span.end_byte(),
        source_hash,
        source_text,
        file_size_bytes: source.len() as u64,
    });
    Ok(())
}
