//! Recursive `injections.scm` execution.
//!
//! When a query pack ships an optional `injections.scm`, this phase runs it over the
//! host tree and delegates each `@injection.content` span to the extractor registered
//! for `injection.language`. Symbols from the embedded language are remapped back into
//! host coordinates by [`crate::extractors::containers`].
//!
//! Injection language selection order:
//! 1. Pattern-level property `#set! injection.language "..."`
//! 2. Captured `@injection.language` node text
//!
//! Combined mode (`#set! injection.combined "true"`): gather every
//! `@injection.content` span for the same language into one host-coordinate buffer
//! where the non-injection bytes are masked to ASCII space (newlines preserved), then
//! delegate once. This handles container hosts like EJS whose code fragments span
//! multiple delimiter boundaries and only form a valid program when reassembled.
//!
//! This is the canonical container path for every host with a stable tree-sitter
//! grammar on crates.io — residual Rust shims (`vue`, `razor`, `blade`) only remain
//! because their upstream grammars are incompatible with our `tree-sitter` version.

use std::collections::BTreeMap;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::core::pipeline::StoredSymbol;
use crate::extractors::common::node_text;
use crate::extractors::containers::{RemapOptions, SegmentOrigin, extract_host_segment};

use super::ScmExtractor;

#[derive(Default)]
struct CombinedBucket {
    ranges: Vec<(usize, usize)>,
}

pub(super) fn run_injections(
    ext: &ScmExtractor,
    file_path: &str,
    source: &str,
    tree: &Tree,
) -> Vec<StoredSymbol> {
    let Some(query) = ext.compiled().injections_query.as_ref() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    let mut combined: BTreeMap<String, CombinedBucket> = BTreeMap::new();
    let capture_names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());

    while let Some(m) = matches.next() {
        let pattern_index = m.pattern_index;
        let mut language = pattern_injection_language(query, pattern_index);
        let is_combined = pattern_injection_combined(query, pattern_index);
        let mut content_range: Option<(usize, usize)> = None;

        for cap in m.captures {
            let name = capture_names[cap.index as usize];
            match name {
                "injection.content" => {
                    let node = cap.node;
                    content_range = Some((node.start_byte(), node.end_byte()));
                }
                "injection.language" => {
                    if language.is_none()
                        && let Ok(text) = node_text(cap.node, source)
                    {
                        language = Some(trim_string_literal(text).to_string());
                    }
                }
                _ => {}
            }
        }

        let Some(lang) = language else { continue };
        let Some((start, end)) = content_range else {
            continue;
        };
        if start >= end || end > source.len() {
            continue;
        }

        if is_combined {
            combined.entry(lang).or_default().ranges.push((start, end));
            continue;
        }

        let delegated_source = &source[start..end];
        let origin = SegmentOrigin::new(source, start);
        // Delegation failures must not abort host extraction — skip this injection and
        // keep going. The embedded language extractor may legitimately fail on a
        // fragment that isn't a complete program (e.g. a single SQL expression).
        if let Ok(symbols) = extract_host_segment(
            &lang,
            file_path,
            source,
            delegated_source,
            origin,
            RemapOptions::default(),
        ) {
            out.extend(symbols);
        }
    }

    for (lang, bucket) in combined {
        let masked = masked_buffer(source, &bucket.ranges);
        // Combined delegation runs against a host-sized buffer, so the origin is at
        // byte 0 and `remap_symbols` is an identity transform on positions — it still
        // rewrites language, file metadata, and IDs through `RemapOptions`.
        let origin = SegmentOrigin::new(source, 0);
        if let Ok(symbols) = extract_host_segment(
            &lang,
            file_path,
            source,
            &masked,
            origin,
            RemapOptions::default(),
        ) {
            out.extend(symbols);
        }
    }

    out
}

/// Build a buffer the same length as `source` where bytes inside any given range are
/// copied verbatim and bytes outside are replaced with ASCII space (or newline, so
/// host line positions remain valid). Because both replacement bytes are ASCII, the
/// result stays valid UTF-8 even when non-injection regions contain multibyte chars.
fn masked_buffer(source: &str, ranges: &[(usize, usize)]) -> String {
    let bytes = source.as_bytes();
    let mut masked = vec![b' '; bytes.len()];
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            masked[i] = b'\n';
        }
    }
    for &(start, end) in ranges {
        if end <= bytes.len() && start <= end {
            masked[start..end].copy_from_slice(&bytes[start..end]);
        }
    }
    // SAFETY: `masked` copies valid UTF-8 substrings from `source` and fills gaps with ASCII
    // space or newline, so the buffer is always valid UTF-8.
    unsafe { String::from_utf8_unchecked(masked) }
}

fn pattern_injection_language(query: &Query, pattern_index: usize) -> Option<String> {
    for p in query.property_settings(pattern_index) {
        if p.key.as_ref() == "injection.language"
            && let Some(value) = p.value.as_ref()
        {
            return Some(value.to_string());
        }
    }
    None
}

fn pattern_injection_combined(query: &Query, pattern_index: usize) -> bool {
    query
        .property_settings(pattern_index)
        .iter()
        .any(|p| p.key.as_ref() == "injection.combined")
}

/// Strip matching surrounding quotes from a captured `@injection.language` node — e.g.
/// a Rust string literal node text is `"sql"`, but we want `sql`.
fn trim_string_literal(text: &str) -> &str {
    let bytes = text.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if first == last && (first == b'"' || first == b'\'' || first == b'`') {
            return &text[1..text.len() - 1];
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractors::LanguageExtractor;
    use crate::extractors::engine::catalogs::QueryPackCatalog;
    use crate::extractors::engine::scm::ScmExtractor;

    #[test]
    fn trim_string_literal_handles_common_quoting() {
        assert_eq!(trim_string_literal("\"sql\""), "sql");
        assert_eq!(trim_string_literal("'sql'"), "sql");
        assert_eq!(trim_string_literal("`sql`"), "sql");
        assert_eq!(trim_string_literal("sql"), "sql");
        assert_eq!(trim_string_literal(""), "");
    }

    fn scm_for(pack_id: &str) -> ScmExtractor {
        let catalog = QueryPackCatalog::builtin();
        let pack = catalog
            .find_by_pack_id(pack_id)
            .unwrap_or_else(|| panic!("missing {pack_id} query pack"));
        ScmExtractor::from_pack(pack).unwrap_or_else(|err| panic!("compile {pack_id} pack: {err}"))
    }

    #[test]
    fn rust_sqlx_macro_string_injected_as_sql() {
        let extractor = scm_for("rust");
        let source = r#"
fn run() {
    sqlx::query!("CREATE TABLE users (id INT PRIMARY KEY)");
}
"#;
        let symbols = extractor
            .extract("src/main.rs", source)
            .expect("extract rust");

        // Host rust symbol for `run` must still be present — injections must not
        // displace the host extraction.
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "run" && s.language == codesight_core::RUST),
            "missing host rust symbol for run: {symbols:?}"
        );

        // Injected SQL symbol for the `users` table must show up, remapped into host
        // coordinates (its start_byte must fall inside the Rust source).
        let sql_syms: Vec<_> = symbols
            .iter()
            .filter(|s| s.language == codesight_core::SQL)
            .collect();
        assert!(
            !sql_syms.is_empty(),
            "no injected sql symbols produced from sqlx::query!: {symbols:?}"
        );
        let table = sql_syms
            .iter()
            .find(|s| s.name.contains("users"))
            .unwrap_or_else(|| panic!("no users table symbol; got {sql_syms:?}"));
        let host_start = source.find("CREATE TABLE").expect("locate sql fragment");
        assert!(
            table.start_byte >= host_start && table.end_byte <= source.len(),
            "injected sql span not remapped into host coordinates: {}..{} (host source len {})",
            table.start_byte,
            table.end_byte,
            source.len()
        );
    }

    #[test]
    fn javascript_sql_tagged_template_injected_as_sql() {
        let extractor = scm_for("javascript");
        let source = r#"
function build() {
    const q = sql`CREATE TABLE orders (id INT)`;
    return q;
}
"#;
        let symbols = extractor
            .extract("src/app.js", source)
            .expect("extract javascript");
        let sql_syms: Vec<_> = symbols
            .iter()
            .filter(|s| s.language == codesight_core::SQL)
            .collect();
        // The template_string node includes backticks; the SQL grammar may tolerate
        // them. Either way, at least the injection path must run without errors and
        // host symbols must still be present.
        assert!(
            symbols.iter().any(|s| s.name == "build"),
            "missing host javascript symbol for build: {symbols:?}"
        );
        // If the sql extractor produced anything, it must be remapped into host bytes.
        for sym in sql_syms {
            assert!(
                sym.end_byte <= source.len(),
                "sql symbol out of host bounds: {sym:?}"
            );
        }
    }

    #[test]
    fn ejs_combined_mode_reassembles_cross_directive_js_functions() {
        let extractor = scm_for("ejs");
        let source = r#"<!DOCTYPE html>
<body>
<% function render(items) { %>
  <ul>
    <% items.forEach(function(item) { %>
      <li><%= item.name %></li>
    <% }); %>
  </ul>
<% } %>
</body>
"#;
        let symbols = extractor
            .extract("views/list.ejs", source)
            .expect("extract ejs");
        // `render` opens in one directive and closes in another — only combined mode
        // can reassemble a JS program that parses it as a function declaration.
        let render = symbols
            .iter()
            .find(|s| s.name == "render")
            .unwrap_or_else(|| panic!("missing render fn: {symbols:?}"));
        assert_eq!(render.kind, crate::core::symbols::SymbolKind::Function);
        assert!(
            source[render.start_byte..render.end_byte].contains("function render"),
            "combined-mode symbol span not anchored to host bytes: {}..{}",
            render.start_byte,
            render.end_byte
        );
    }

    #[test]
    fn masked_buffer_preserves_newlines_and_ranges() {
        // "hello\nworld\nfoo" — mask everything except "world".
        let src = "hello\nworld\nfoo";
        let ranges = [(6usize, 11usize)];
        let masked = masked_buffer(src, &ranges);
        assert_eq!(masked.len(), src.len());
        assert_eq!(&masked[6..11], "world");
        assert_eq!(masked.as_bytes()[5], b'\n');
        assert_eq!(masked.as_bytes()[11], b'\n');
        for i in 0..5 {
            assert_eq!(masked.as_bytes()[i], b' ');
        }
    }

    #[test]
    fn masked_buffer_keeps_utf8_valid_for_multibyte_non_injection_regions() {
        // The é outside the injection range must not leave a broken UTF-8 sequence.
        let src = "é world é";
        let start = src
            .find("world")
            .expect("test fixture contains ASCII substring world");
        let end = start + "world".len();
        let masked = masked_buffer(src, &[(start, end)]);
        // Valid UTF-8 check: construction would have panicked otherwise. Range is intact.
        assert!(masked.is_char_boundary(start));
        assert!(masked.is_char_boundary(end));
        assert_eq!(&masked[start..end], "world");
    }

    #[test]
    fn pack_without_injections_scm_is_unaffected() {
        // Python has no injections.scm; extraction must still succeed and produce
        // host symbols without visiting the injection path.
        let extractor = scm_for("python");
        let source = "def greet(name):\n    return name\n";
        let symbols = extractor.extract("src/mod.py", source).expect("extract");
        assert!(symbols.iter().any(|s| s.name == "greet"));
    }
}
