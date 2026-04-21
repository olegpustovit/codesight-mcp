//! **EJS**: embedded JavaScript template. The grammar-backed pipeline now owns
//! scriptlet detection and JavaScript delegation through `runtime/queries/ejs/`
//! (`tags.scm` + `injections.scm`, backed by `tree-sitter-embedded-template`).
//!
//! This override stays only for two tiny host-semantic post-passes that the grammar
//! does not encode structurally:
//!
//! 1. A synthetic **template** symbol derived from the file stem (downstream outline
//!    consumers expect every template file to surface at least one symbol).
//! 2. An `include('path')` directive scan emitted as **Import** symbols so the graph
//!    builder sees template-to-partial edges.
//!
//! Everything else (JavaScript function/class/const detection, offset remapping) is
//! grammar-backed via the shared injections runner and `containers::extract_host_segment`.
//!
//! Exit criteria: if `tree-sitter-embedded-template` ever exposes `include(...)` as
//! a distinct node kind and structural raw-output context, both post-passes can move
//! into `runtime/queries/ejs/tags.scm` and this override can be deleted.

use std::sync::{LazyLock, OnceLock};

use anyhow::Result;
use regex::Regex;

use codesight_core::StoredSymbol;
use codesight_core::symbols::{ExtractionTier, SymbolKind};
use crate::query_pack_catalog::QueryPackCatalog;
use crate::scm_engine::ScmExtractor;

pub struct EjsExtractor;

impl EjsExtractor {
    pub fn language_name(&self) -> &'static str {
        "ejs"
    }

    pub fn supported_extensions(&self) -> &'static [&'static str] {
        &["ejs"]
    }

    pub fn extraction_tier(&self) -> ExtractionTier {
        ExtractionTier::CuratedQueryPack
    }

    pub fn extract(&self, file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
        // 1. Grammar-backed extraction via the ejs query pack. Its `injections.scm`
        //    delegates every `(code)` span to the JavaScript extractor and remaps
        //    resulting symbols back into host coordinates — no regex masking.
        let mut symbols = ejs_pack()?
            .extract(file_path, source)
            .unwrap_or_else(|_| Vec::new());

        // Delegated JS symbols must surface as language "ejs" so downstream outline
        // and graph queries see a consistent host language for the template file.
        for symbol in &mut symbols {
            symbol.language = codesight_core::EJS;
        }

        // 2. Synthetic template symbol — host semantics, not a grammar construct.
        let template_name = std::path::Path::new(file_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("template");
        symbols.insert(
            0,
            make_symbol(
                file_path,
                template_name,
                SymbolKind::Template,
                1,
                source.lines().next().unwrap_or(""),
                0,
                source.len(),
                source,
            ),
        );

        // 3. `include('path')` directive scan → Import symbols. The grammar produces
        //    these as regular JavaScript call expressions inside `(code)` nodes; a
        //    tiny regex post-pass picks them up by path without re-parsing.
        for (line_num, line_start, line_end, line) in source_lines(source) {
            if let Some(caps) = INCLUDE_RE.captures(line) {
                let Some(path) = caps.get(1).map(|g| g.as_str()) else {
                    continue;
                };
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

fn ejs_pack() -> Result<&'static ScmExtractor> {
    // Built-in pack is compiled in; a missing entry is a logic bug, not a runtime error.
    static EJS_PACK: OnceLock<ScmExtractor> = OnceLock::new();
    Ok(EJS_PACK.get_or_init(|| {
        let catalog = QueryPackCatalog::builtin();
        let pack = catalog
            .find_by_pack_id("ejs")
            .expect("infallible: built-in catalog missing ejs query pack");
        ScmExtractor::from_pack(pack).expect("infallible: ejs pack failed to build")
    }))
}

static INCLUDE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<%[-=]?\s*include\s*\(\s*['"]([^'"]+)['"]\s*(?:,[^)]*)?\)"#)
        .expect("infallible: ejs include regex")
});

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
        extraction_tier: ExtractionTier::CuratedQueryPack,
        language: codesight_core::EJS,
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

#[cfg(test)]
mod tests {
    use super::EjsExtractor;
    use codesight_core::symbols::SymbolKind;
    use crate::scm_engine::ScmExtractor;

    const TEMPLATE: &str = r#"<!DOCTYPE html>
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

    #[test]
    fn ejs_template_and_include_symbols_from_post_pass() {
        let syms = EjsExtractor.extract("views/index.ejs", TEMPLATE).unwrap();
        assert!(
            syms.iter()
                .any(|s| s.name == "index" && s.kind == SymbolKind::Template),
            "missing synthetic template symbol: {syms:?}"
        );
        assert!(
            syms.iter()
                .any(|s| s.name == "partials/header" && s.kind == SymbolKind::Import),
            "missing header include: {syms:?}"
        );
        assert!(
            syms.iter()
                .any(|s| s.name == "partials/footer" && s.kind == SymbolKind::Import),
            "missing footer include: {syms:?}"
        );
    }

    #[test]
    fn ejs_javascript_functions_come_from_grammar_backed_injection() {
        let syms = EjsExtractor.extract("views/index.ejs", TEMPLATE).unwrap();
        let formatted = syms
            .iter()
            .find(|s| s.name == "formatDate")
            .unwrap_or_else(|| panic!("no formatDate symbol: {syms:?}"));
        assert_eq!(formatted.kind, SymbolKind::Function);
        // Grammar-backed injections must emit the symbol under the EJS host language,
        // and its byte span must fall inside the original .ejs source.
        assert_eq!(formatted.language, codesight_core::EJS);
        assert!(formatted.end_byte <= TEMPLATE.len());
        assert!(
            TEMPLATE[formatted.start_byte..formatted.end_byte].contains("formatDate"),
            "injected symbol span mis-remapped: {}..{}",
            formatted.start_byte,
            formatted.end_byte
        );
    }

    #[test]
    fn ejs_static_template_produces_template_symbol_only() {
        let src = "<html><body><h1>Hello</h1></body></html>";
        let syms = EjsExtractor.extract("static.ejs", src).unwrap();
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].kind, SymbolKind::Template);
        assert_eq!(syms[0].name, "static");
    }
}
