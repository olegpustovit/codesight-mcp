//! **Razor** (`.cshtml` / `.razor`): markup mixes HTML and embedded languages; there is no single
//! grammar for the whole file.
//!
//! ## Pipeline
//!
//! - Markup and Razor directives (`@page`, `@inject`) stay line/regex-based (no stable mixed
//!   tree-sitter grammar).
//! - **`@code` / `@functions` blocks** — parse a synthetic C# wrapper with
//!   [`tree_sitter_c_sharp`] first; fall back to regex when the parse yields no declarations.
//! - **Inline `<script>`** (no `src`) — [`tree_sitter_javascript`] first; regex fallback.

use regex::Regex;
use std::sync::LazyLock;
use tree_sitter::Node;

use crate::model::{ExtractionTier, StoredSymbol, SymbolKind};

use super::common::{node_text, parse_tree};
use super::fallback_pipeline::sort_symbols_by_span;

pub struct RazorExtractor;

impl super::LanguageExtractor for RazorExtractor {
    fn language_name(&self) -> &'static str {
        "razor"
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["cshtml", "razor"]
    }

    fn extract(&self, file_path: &str, source: &str) -> anyhow::Result<Vec<StoredSymbol>> {
        let mut symbols = Vec::new();

        // Derive the component/view name from the file stem
        let view_name = std::path::Path::new(file_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
            .to_string();

        // The view/component itself is a class symbol at line 1
        let first_line_end = source.lines().next().map_or(0, |l| l.len());
        symbols.push(make_sym(
            file_path,
            &view_name,
            SymbolKind::Class,
            1,
            &view_name,
            0,
            first_line_end,
            source,
            "razor",
            &[],
        ));

        let lines: Vec<&str> = source.lines().collect();
        let mut byte_offset = 0usize;
        let mut style_count = 0u32;

        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            let line_num = (i + 1) as u32;
            let line_start = byte_offset;
            let line_end = byte_offset + line.len();

            // @page directives (Blazor)
            if let Some(caps) = RE_PAGE.captures(trimmed) {
                let route = caps[1].to_string();
                symbols.push(make_sym(
                    file_path,
                    &route,
                    SymbolKind::Constant,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                    "razor",
                    &[view_name.clone()],
                ));
            }
            // @inject directives (Blazor)
            else if let Some(caps) = RE_INJECT.captures(trimmed) {
                let name = caps[2].to_string();
                symbols.push(make_sym(
                    file_path,
                    &name,
                    SymbolKind::Constant,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                    "razor",
                    &[view_name.clone()],
                ));
            }

            // HTML elements with id attributes
            for caps in RE_HTML_ID.captures_iter(line) {
                let id_val = caps[1].to_string();
                symbols.push(make_sym(
                    file_path,
                    &id_val,
                    SymbolKind::Constant,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                    "razor",
                    &[view_name.clone()],
                ));
            }

            // External script src
            if let Some(caps) = RE_SCRIPT_SRC.captures(trimmed) {
                let src = caps[1].to_string();
                let script_name = src.rsplit('/').next().unwrap_or(&src);
                let script_name = script_name.trim_start_matches('~');
                symbols.push(make_sym(
                    file_path,
                    script_name,
                    SymbolKind::Function,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                    "razor",
                    &[view_name.clone()],
                ));
            }

            // Inline <style> blocks
            if trimmed == "<style>" || trimmed.starts_with("<style ") {
                style_count += 1;
                let style_name = format!("style_{style_count}");
                symbols.push(make_sym(
                    file_path,
                    &style_name,
                    SymbolKind::Constant,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                    "razor",
                    &[view_name.clone()],
                ));
            }

            byte_offset += line.len() + 1;
        }

        // Extract C# code blocks: @functions { ... } and @code { ... }
        extract_csharp_blocks(file_path, source, &view_name, &mut symbols);

        // Extract inline JavaScript from <script> blocks (without src)
        extract_inline_js(file_path, source, &view_name, &mut symbols);

        sort_symbols_by_span(&mut symbols);
        // Intentionally no `dedup_by_start_byte`: the file-level view symbol and `@page` on line 1
        // share the same line start byte; both must remain.
        Ok(symbols)
    }
}

static RE_PAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^@page\s+"([^"]+)""#).unwrap());
static RE_INJECT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^@inject\s+(\S+)\s+(\w+)"#).unwrap());
static RE_HTML_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"id="([^"]+)""#).unwrap());
static RE_SCRIPT_SRC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<script\s+[^>]*src="([^"]+)""#).unwrap());
static RE_CSHARP_METHOD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:public|private|protected|internal)?\s*(?:override\s+)?(?:static\s+)?(?:async\s+)?(?:void|string|int|bool|decimal|float|double|long|char|byte|Task(?:<[^>]+>)?|\w+)\s+(\w+)\s*\(").unwrap()
});
static RE_CSHARP_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:public|private|protected|internal)\s+(?:static\s+)?(?:readonly\s+)?(?:int|string|bool|decimal|float|double|long|char|byte|\w+)\s+(\w+)\s*[=;]").unwrap()
});
static RE_JS_FUNCTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"function\s+(\w+)\s*\(").unwrap());

fn extract_csharp_blocks(
    file_path: &str,
    source: &str,
    view_name: &str,
    symbols: &mut Vec<StoredSymbol>,
) {
    let block_re: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?m)@(?:functions|code)\s*\{").unwrap());

    for m in block_re.find_iter(source) {
        let block_start = m.start();
        let brace_start = source[block_start..].find('{').unwrap() + block_start;
        if let Some(block_end) = find_matching_brace(source, brace_start) {
            let block_content = &source[brace_start + 1..block_end];
            let body_start_file = brace_start + 1;
            if !extract_csharp_block_via_ts(
                file_path,
                source,
                view_name,
                block_content,
                body_start_file,
                symbols,
            ) {
                extract_csharp_block_via_regex(
                    file_path,
                    source,
                    view_name,
                    block_content,
                    brace_start,
                    symbols,
                );
            }
        }
    }
}

/// Returns `true` when tree-sitter emitted at least one symbol for this block.
fn extract_csharp_block_via_ts(
    file_path: &str,
    source: &str,
    view_name: &str,
    block_content: &str,
    body_start_file: usize,
    symbols: &mut Vec<StoredSymbol>,
) -> bool {
    const PREFIX: &str = "class __CodesightRazorHost {\n";
    const SUFFIX: &str = "\n}";
    let wrapped = format!("{PREFIX}{block_content}{SUFFIX}");
    let Ok(tree) = parse_tree(tree_sitter_c_sharp::LANGUAGE, &wrapped) else {
        return false;
    };
    let inner_base = PREFIX.len();
    let inner_end = inner_base + block_content.len();
    let mut count = 0usize;
    walk_csharp_declarations(
        tree.root_node(),
        &wrapped,
        file_path,
        source,
        view_name,
        body_start_file,
        inner_base,
        inner_end,
        symbols,
        &mut count,
    );
    count > 0
}

#[allow(clippy::too_many_arguments)]
fn walk_csharp_declarations(
    node: Node<'_>,
    wrapped: &str,
    file_path: &str,
    full_source: &str,
    view_name: &str,
    body_start_file: usize,
    inner_base: usize,
    inner_end: usize,
    symbols: &mut Vec<StoredSymbol>,
    count: &mut usize,
) {
    let map_byte = |b: usize| body_start_file + b.saturating_sub(inner_base);
    let in_block = |n: Node<'_>| n.start_byte() >= inner_base && n.end_byte() <= inner_end;

    match node.kind() {
        "method_declaration" if in_block(node) => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = node_text(name_node, wrapped)
                && name != "get"
                && name != "set"
                && name != "value"
            {
                push_csharp_method_symbol(
                    file_path,
                    full_source,
                    view_name,
                    name,
                    node,
                    map_byte,
                    symbols,
                );
                *count += 1;
            }
        }
        "property_declaration" if in_block(node) => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = node_text(name_node, wrapped)
            {
                push_csharp_fieldlike_symbol(
                    file_path,
                    full_source,
                    view_name,
                    name,
                    node,
                    map_byte,
                    symbols,
                );
                *count += 1;
            }
        }
        "field_declaration" if in_block(node) => {
            let mut c = node.walk();
            for child in node.named_children(&mut c) {
                if child.kind() != "variable_declaration" {
                    continue;
                }
                let mut c2 = child.walk();
                for vd in child.named_children(&mut c2) {
                    if vd.kind() != "variable_declarator" {
                        continue;
                    }
                    if let Some(id_node) = vd.child_by_field_name("name")
                        && let Ok(name) = node_text(id_node, wrapped)
                    {
                        push_csharp_fieldlike_symbol(
                            file_path,
                            full_source,
                            view_name,
                            name,
                            vd,
                            map_byte,
                            symbols,
                        );
                        *count += 1;
                    }
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_csharp_declarations(
            child,
            wrapped,
            file_path,
            full_source,
            view_name,
            body_start_file,
            inner_base,
            inner_end,
            symbols,
            count,
        );
    }
}

fn push_csharp_method_symbol(
    file_path: &str,
    full_source: &str,
    view_name: &str,
    name: &str,
    span: Node<'_>,
    map_byte: impl Fn(usize) -> usize,
    symbols: &mut Vec<StoredSymbol>,
) {
    let abs_start = map_byte(span.start_byte());
    let abs_end = map_byte(span.end_byte()).min(full_source.len());
    let line_num = full_source[..abs_start].lines().count() as u32;
    let end_line = full_source[..abs_end].lines().count() as u32;
    let line_text = full_source
        .lines()
        .nth(line_num.saturating_sub(1) as usize)
        .unwrap_or("");
    let src_text = full_source.get(abs_start..abs_end).unwrap_or("");
    let name = name.to_string();
    symbols.push(StoredSymbol {
        id: format!("{file_path}::{view_name}.{name}#method"),
        extraction_tier: ExtractionTier::ManualOverride,
        language: "csharp".to_string(),
        file_path: file_path.to_string(),
        qualified_name: format!("{view_name}.{name}"),
        name,
        kind: SymbolKind::Method,
        signature: line_text.trim().to_string(),
        summary: line_text.trim().to_string(),
        start_line: line_num,
        end_line,
        start_byte: abs_start,
        end_byte: abs_end,
        source_hash: blake3::hash(src_text.as_bytes()).to_hex().to_string(),
        source_text: src_text.to_string(),
        file_size_bytes: full_source.len() as u64,
    });
}

fn push_csharp_fieldlike_symbol(
    file_path: &str,
    full_source: &str,
    view_name: &str,
    name: &str,
    span: Node<'_>,
    map_byte: impl Fn(usize) -> usize,
    symbols: &mut Vec<StoredSymbol>,
) {
    if symbols
        .iter()
        .any(|s| s.name == name && s.kind == SymbolKind::Method)
    {
        return;
    }
    let abs_start = map_byte(span.start_byte());
    let abs_end = map_byte(span.end_byte()).min(full_source.len());
    let line_num = full_source[..abs_start].lines().count() as u32;
    let line_text = full_source
        .lines()
        .nth(line_num.saturating_sub(1) as usize)
        .unwrap_or("");
    let name = name.to_string();
    let src_text = full_source.get(abs_start..abs_end).unwrap_or("");
    symbols.push(StoredSymbol {
        id: format!("{file_path}::{view_name}.{name}#constant"),
        extraction_tier: ExtractionTier::ManualOverride,
        language: "csharp".to_string(),
        file_path: file_path.to_string(),
        qualified_name: format!("{view_name}.{name}"),
        name,
        kind: SymbolKind::Constant,
        signature: line_text.trim().to_string(),
        summary: line_text.trim().to_string(),
        start_line: line_num,
        end_line: line_num,
        start_byte: abs_start,
        end_byte: abs_end,
        source_hash: blake3::hash(src_text.as_bytes()).to_hex().to_string(),
        source_text: src_text.to_string(),
        file_size_bytes: full_source.len() as u64,
    });
}

/// Regex fallback for Razor C# fragments tree-sitter rejects or parses with zero declarations.
fn extract_csharp_block_via_regex(
    file_path: &str,
    source: &str,
    view_name: &str,
    block_content: &str,
    brace_start: usize,
    symbols: &mut Vec<StoredSymbol>,
) {
    for caps in RE_CSHARP_METHOD.captures_iter(block_content) {
        let name = caps[1].to_string();
        if name == "get" || name == "set" || name == "value" {
            continue;
        }
        let match_start = caps.get(0).unwrap().start();
        let abs_byte = brace_start + 1 + match_start;
        let line_num = source[..abs_byte].lines().count() as u32;
        let line_text = source
            .lines()
            .nth(line_num.saturating_sub(1) as usize)
            .unwrap_or("");

        let method_end_byte = block_content[match_start..]
            .find('{')
            .and_then(|body_offset| {
                find_matching_brace(source, brace_start + 1 + match_start + body_offset)
            })
            .unwrap_or(abs_byte + line_text.len());

        let end_line = source[..method_end_byte].lines().count() as u32;
        let src_text = source.get(abs_byte..method_end_byte).unwrap_or("");

        symbols.push(StoredSymbol {
            id: format!("{file_path}::{view_name}.{name}#method"),
            extraction_tier: ExtractionTier::ManualOverride,
            language: "csharp".to_string(),
            file_path: file_path.to_string(),
            qualified_name: format!("{view_name}.{name}"),
            name,
            kind: SymbolKind::Method,
            signature: line_text.trim().to_string(),
            summary: line_text.trim().to_string(),
            start_line: line_num,
            end_line,
            start_byte: abs_byte,
            end_byte: method_end_byte,
            source_hash: blake3::hash(src_text.as_bytes()).to_hex().to_string(),
            source_text: src_text.to_string(),
            file_size_bytes: source.len() as u64,
        });
    }

    for caps in RE_CSHARP_FIELD.captures_iter(block_content) {
        let name = caps[1].to_string();
        if symbols
            .iter()
            .any(|s| s.name == name && s.kind == SymbolKind::Method)
        {
            continue;
        }
        let match_start = caps.get(0).unwrap().start();
        let abs_byte = brace_start + 1 + match_start;
        let line_num = source[..abs_byte].lines().count() as u32;
        let line_text = source
            .lines()
            .nth(line_num.saturating_sub(1) as usize)
            .unwrap_or("");

        symbols.push(StoredSymbol {
            id: format!("{file_path}::{view_name}.{name}#constant"),
            extraction_tier: ExtractionTier::ManualOverride,
            language: "csharp".to_string(),
            file_path: file_path.to_string(),
            qualified_name: format!("{view_name}.{name}"),
            name,
            kind: SymbolKind::Constant,
            signature: line_text.trim().to_string(),
            summary: line_text.trim().to_string(),
            start_line: line_num,
            end_line: line_num,
            start_byte: abs_byte,
            end_byte: abs_byte + line_text.len(),
            source_hash: blake3::hash(line_text.as_bytes()).to_hex().to_string(),
            source_text: line_text.to_string(),
            file_size_bytes: source.len() as u64,
        });
    }
}

fn extract_inline_js(
    file_path: &str,
    source: &str,
    view_name: &str,
    symbols: &mut Vec<StoredSymbol>,
) {
    let script_open = Regex::new(r"(?i)<script(?:\s[^>]*)?>").unwrap();
    let script_close = Regex::new(r"(?i)</script>").unwrap();

    let opens: Vec<_> = script_open.find_iter(source).collect();
    let closes: Vec<_> = script_close.find_iter(source).collect();

    for (open_match, close_match) in opens.iter().zip(closes.iter()) {
        let tag = open_match.as_str();
        if tag.contains("src=") {
            continue;
        }
        let content = &source[open_match.end()..close_match.start()];
        let content_start = open_match.end();

        if !extract_js_functions_via_ts(
            file_path,
            source,
            view_name,
            content,
            content_start,
            symbols,
        ) {
            for caps in RE_JS_FUNCTION.captures_iter(content) {
                let name = caps[1].to_string();
                let match_start = caps.get(0).unwrap().start();
                let abs_byte = content_start + match_start;
                let line_num = source[..abs_byte].lines().count() as u32;
                let line_text = source
                    .lines()
                    .nth(line_num.saturating_sub(1) as usize)
                    .unwrap_or("");

                symbols.push(StoredSymbol {
                    id: format!("{file_path}::{view_name}.{name}#function"),
                    extraction_tier: ExtractionTier::ManualOverride,
                    language: "javascript".to_string(),
                    file_path: file_path.to_string(),
                    qualified_name: format!("{view_name}.{name}"),
                    name,
                    kind: SymbolKind::Function,
                    signature: line_text.trim().to_string(),
                    summary: line_text.trim().to_string(),
                    start_line: line_num,
                    end_line: line_num,
                    start_byte: abs_byte,
                    end_byte: abs_byte + line_text.trim().len(),
                    source_hash: blake3::hash(line_text.as_bytes()).to_hex().to_string(),
                    source_text: line_text.trim().to_string(),
                    file_size_bytes: source.len() as u64,
                });
            }
        }
    }
}

/// Returns `true` if at least one `function` declaration was recorded from tree-sitter.
fn extract_js_functions_via_ts(
    file_path: &str,
    full_source: &str,
    view_name: &str,
    script_inner: &str,
    script_inner_start_file: usize,
    symbols: &mut Vec<StoredSymbol>,
) -> bool {
    let Ok(tree) = parse_tree(tree_sitter_javascript::LANGUAGE, script_inner) else {
        return false;
    };
    let mut count = 0usize;
    walk_js_function_decls(
        tree.root_node(),
        script_inner,
        file_path,
        full_source,
        view_name,
        script_inner_start_file,
        symbols,
        &mut count,
    );
    count > 0
}

fn walk_js_function_decls(
    node: Node<'_>,
    script_inner: &str,
    file_path: &str,
    full_source: &str,
    view_name: &str,
    script_inner_start_file: usize,
    symbols: &mut Vec<StoredSymbol>,
    count: &mut usize,
) {
    if node.kind() == "function_declaration"
        && let Some(name_node) = node.child_by_field_name("name")
        && let Ok(name) = node_text(name_node, script_inner)
    {
        let abs_start = script_inner_start_file + node.start_byte();
        let abs_end = script_inner_start_file + node.end_byte();
        let line_num = full_source[..abs_start].lines().count() as u32;
        let end_line = full_source[..abs_end.min(full_source.len())]
            .lines()
            .count() as u32;
        let line_text = full_source
            .lines()
            .nth(line_num.saturating_sub(1) as usize)
            .unwrap_or("");
        let src_text = full_source
            .get(abs_start..abs_end.min(full_source.len()))
            .unwrap_or("");
        let name = name.to_string();
        symbols.push(StoredSymbol {
            id: format!("{file_path}::{view_name}.{name}#function"),
            extraction_tier: ExtractionTier::ManualOverride,
            language: "javascript".to_string(),
            file_path: file_path.to_string(),
            qualified_name: format!("{view_name}.{name}"),
            name,
            kind: SymbolKind::Function,
            signature: line_text.trim().to_string(),
            summary: line_text.trim().to_string(),
            start_line: line_num,
            end_line,
            start_byte: abs_start,
            end_byte: abs_end.min(full_source.len()),
            source_hash: blake3::hash(src_text.as_bytes()).to_hex().to_string(),
            source_text: src_text.to_string(),
            file_size_bytes: full_source.len() as u64,
        });
        *count += 1;
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_js_function_decls(
            child,
            script_inner,
            file_path,
            full_source,
            view_name,
            script_inner_start_file,
            symbols,
            count,
        );
    }
}

fn find_matching_brace(source: &str, open_pos: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.get(open_pos) != Some(&b'{') {
        return None;
    }
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape_next = false;

    for (i, &b) in bytes[open_pos..].iter().enumerate() {
        if escape_next {
            escape_next = false;
            continue;
        }
        if b == b'\\' && in_string {
            escape_next = true;
            continue;
        }
        if b == b'"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(open_pos + i);
            }
        }
    }
    None
}

fn make_sym(
    file_path: &str,
    name: &str,
    kind: SymbolKind,
    line_num: u32,
    signature: &str,
    start_byte: usize,
    end_byte: usize,
    source: &str,
    language: &str,
    scopes: &[String],
) -> StoredSymbol {
    let qualified_name = if scopes.is_empty() {
        name.to_string()
    } else {
        format!("{}.{}", scopes.last().unwrap(), name)
    };
    let source_text = source
        .get(start_byte..end_byte)
        .unwrap_or(signature)
        .to_string();
    let source_hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
    StoredSymbol {
        id: format!("{file_path}::{qualified_name}#{}", kind.as_str()),
        extraction_tier: ExtractionTier::ManualOverride,
        language: language.to_string(),
        file_path: file_path.to_string(),
        qualified_name,
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
