//! Shared tree-sitter helpers for language extractors (Strategy implementations).

use std::cell::RefCell;
use std::collections::HashMap;

use tree_sitter::{Node, Parser, Tree};

use codesight_core::StoredSymbol;
use codesight_core::symbols::{ExtractionTier, SymbolKind};

use crate::extract_error::{ExtractError, ExtractResult};

thread_local! {
    /// Per-thread cache of `tree_sitter::Parser`s keyed by a cheap identity
    /// tuple over the compiled grammar. Building a `Parser` and calling
    /// `set_language` costs several microseconds per file, which adds up
    /// across thousands of files during cold ingest. Reusing one parser per
    /// `(thread, grammar)` pair is safe: `Parser` is `Send` but not `Sync`,
    /// so the thread-local scope is exactly right. N is small (one entry per
    /// grammar the worker has seen), so a hash map keeps lookups flat.
    static PARSERS: RefCell<HashMap<GrammarKey, Parser>> = RefCell::new(HashMap::new());
}

/// Fingerprint used to key the thread-local parser pool. `tree_sitter::Language`
/// doesn't implement `Hash`, so we fold a couple of cheap accessors into a
/// tuple that's unique across every grammar we bundle. Collisions would only
/// matter if two grammars shared *every* accessor, which tree-sitter's own
/// ABI guarantees to avoid in practice.
#[derive(Clone, Copy, Eq, PartialEq, Hash)]
struct GrammarKey {
    abi_version: usize,
    node_kind_count: u32,
    field_count: u32,
    parse_state_count: u32,
}

fn grammar_key(lang: &tree_sitter::Language) -> GrammarKey {
    GrammarKey {
        abi_version: lang.abi_version(),
        node_kind_count: lang.node_kind_count() as u32,
        field_count: lang.field_count() as u32,
        parse_state_count: lang.parse_state_count() as u32,
    }
}

/// Lease the thread-local parser for `lang` and run `f` against it. The
/// parser is returned to the cache when `f` finishes.
pub fn parser_for_language<T, F>(lang: &tree_sitter::Language, f: F) -> T
where
    F: FnOnce(&mut Parser) -> T,
{
    let key = grammar_key(lang);
    PARSERS.with(|cell| {
        let mut map = cell.borrow_mut();
        let parser = map.entry(key).or_insert_with(|| {
            let mut p = Parser::new();
            // Missing grammar here means the caller handed in a broken
            // language handle — a programmer error, not a runtime condition.
            p.set_language(lang)
                .expect("tree-sitter set_language failed on cached parser");
            p
        });
        f(parser)
    })
}

pub fn parse_tree(lang: impl Into<tree_sitter::Language>, source: &str) -> ExtractResult<Tree> {
    let lang = lang.into();
    parser_for_language(&lang, |parser| {
        parser.parse(source, None).ok_or(ExtractError::MissingParseTree)
    })
}

pub fn node_text<'a>(node: Node<'_>, source: &'a str) -> ExtractResult<&'a str> {
    node.utf8_text(source.as_bytes())
        .map_err(|_| ExtractError::InvalidUtf8)
}

pub fn source_slice<'a>(node: Node<'_>, source: &'a str) -> ExtractResult<&'a str> {
    source
        .get(node.start_byte()..node.end_byte())
        .ok_or(ExtractError::ByteRangeOutOfBounds)
}

pub fn first_line(node: Node<'_>, source: &str) -> ExtractResult<String> {
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

#[allow(clippy::too_many_arguments)]
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
) -> ExtractResult<()> {
    push_symbol_ex(
        symbols, file_path, language, scopes, name, kind, span, source, summary, None,
    )
}

/// Like [`push_symbol`] but accepts a pre-built `qualified_name` instead of
/// computing it from scopes.  Use this when the language's native separator
/// (e.g. Lua/Luau `:` for colon-methods) must be preserved verbatim.
#[allow(clippy::too_many_arguments)]
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
) -> ExtractResult<()> {
    let signature = first_line(span, source)?;
    let summary = summary.unwrap_or_else(|| signature.clone());
    let source_text = source_slice(span, source)?.to_string();
    let source_hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
    symbols.push(StoredSymbol {
        id: format!("{file_path}::{qualified_name}#{}", kind.as_str()),
        extraction_tier: ExtractionTier::ManualOverride,
        language: codesight_core::LanguageId::intern(language),
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

#[allow(clippy::too_many_arguments)]
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
) -> ExtractResult<()> {
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
        extraction_tier: ExtractionTier::ManualOverride,
        language: codesight_core::LanguageId::intern(language),
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
