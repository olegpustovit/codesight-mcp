//! Query execution phases: capture collection, doc association, scope inference, symbol emission.
//!
//! Ordering: `parse header → compile query` (in [`super::ScmExtractor::new`]) → **here:**
//! `collect captures → dedupe → attach standalone docs → sort for containment → infer scopes /
//! build [`StoredSymbol`](crate::model::StoredSymbol)s → cpp header post-pass`.

use anyhow::{Result, anyhow};
use streaming_iterator::StreamingIterator;
use tree_sitter::QueryCursor;

use crate::extractors::common::{first_line, node_text, qualify, source_slice};
use crate::model::{ExtractionTier, StoredSymbol};

use super::ScmExtractor;
use super::capture::{OwnedCapture, parse_symbol_kind};
use super::docs::clean_docstring;
use super::pattern::{PatternProps, read_pattern_props};

pub(super) fn extract_symbols(
    ext: &ScmExtractor,
    file_path: &str,
    source: &str,
) -> Result<Vec<StoredSymbol>> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&ext.grammar)
        .map_err(|e| anyhow!("set_language: {e}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow!("parse returned no tree"))?;
    let mut cursor = QueryCursor::new();

    let capture_names = ext.query.capture_names();

    // --- Phase 1: Collect raw captures ---
    // QueryMatches is a StreamingIterator — text predicates (#match?, #eq?) are
    // checked automatically. We extract owned data from each match before advancing.
    let mut raws: Vec<OwnedCapture> = Vec::new();
    let mut standalone_docs: Vec<(usize, String)> = Vec::new(); // (byte_offset, cleaned_doc)

    let mut matches = cursor.matches(&ext.query, tree.root_node(), source.as_bytes());
    while let Some(m) = matches.next() {
        let mut props = read_pattern_props(&ext.query, m.pattern_index);

        let mut def_start = 0usize;
        let mut def_end = 0usize;
        let mut def_start_row = 0usize;
        let mut def_end_row = 0usize;
        let mut def_kind_str: Option<String> = None;
        let mut def_signature = String::new();
        let mut def_source_text = String::new();
        let mut name_text: Option<String> = None;
        let mut name_start_byte = 0usize;
        let mut doc_text: Option<String> = None;
        let mut scope_text: Option<String> = None;
        let mut qname_text: Option<String> = None;
        let mut has_def = false;

        let mut capture_props = PatternProps::default();

        for cap in m.captures {
            let cap_name = capture_names[cap.index as usize];
            if let Some(rest) = cap_name.strip_prefix("definition.") {
                has_def = true;
                def_start = cap.node.start_byte();
                def_end = cap.node.end_byte();
                def_start_row = cap.node.start_position().row;
                def_end_row = cap.node.end_position().row;
                // Parse suffixes: "constant.nofunc" → kind="constant", exclude_inside="function"
                // "namespace.scope_only" → kind="namespace", scope_only=true
                let mut kind_part = rest;
                if let Some(base) = rest.strip_suffix(".nofunc") {
                    kind_part = base;
                    capture_props.exclude_inside = Some("function".to_string());
                } else if let Some(base) = rest.strip_suffix(".scope_only") {
                    kind_part = base;
                    capture_props.scope_only = true;
                }
                def_kind_str = Some(kind_part.to_string());
                def_signature = first_line(cap.node, source).unwrap_or_default();
                def_source_text = source_slice(cap.node, source).unwrap_or("").to_string();
            } else {
                match cap_name {
                    "name" => {
                        name_text = node_text(cap.node, source).ok().map(|s| s.to_string());
                        name_start_byte = cap.node.start_byte();
                    }
                    "doc" => {
                        doc_text = node_text(cap.node, source).ok().map(clean_docstring);
                    }
                    "scope" => {
                        scope_text = node_text(cap.node, source)
                            .ok()
                            .map(|t| t.trim_start_matches('*').to_string());
                    }
                    "qualified_name" => {
                        qname_text = node_text(cap.node, source).ok().map(|s| s.to_string());
                    }
                    _ => {} // internal captures (_key, etc.)
                }
            }
        }

        // Standalone @doc captures (no @definition.* in same match):
        // collect with the byte position from the match for containment lookup.
        if !has_def {
            if let Some(doc) = doc_text {
                // Use the first capture's start byte as location hint.
                let byte_pos = m.captures.first().map_or(0, |c| c.node.start_byte());
                standalone_docs.push((byte_pos, doc));
            }
            continue;
        }
        let Some(kind_str) = def_kind_str else {
            continue;
        };

        // Resolve name: fixed_name > @name capture (with optional prefix)
        let name = if let Some(fixed) = &props.fixed_name {
            fixed.clone()
        } else if let Some(n) = name_text {
            if let Some(ref prefix) = props.name_prefix {
                format!("{prefix}{n}")
            } else {
                n
            }
        } else {
            continue;
        };

        let Some(kind) = parse_symbol_kind(&kind_str) else {
            continue;
        };

        // Merge capture-name props with #set! props (capture-name takes precedence).
        if capture_props.exclude_inside.is_some() {
            props.exclude_inside = capture_props.exclude_inside;
        }
        if capture_props.scope_only {
            props.scope_only = true;
        }
        if capture_props.fixed_name.is_some() {
            props.fixed_name = capture_props.fixed_name;
        }

        raws.push(OwnedCapture {
            kind,
            kind_str,
            name,
            name_start_byte,
            def_start,
            def_end,
            def_start_row,
            def_end_row,
            def_signature,
            def_source_text,
            doc: doc_text,
            scope_label: scope_text,
            qualified_name_override: qname_text,
            pattern_index: m.pattern_index,
            props,
        });
    }

    // --- Phase 2: Dedup by name position (lowest pattern_index wins) ---
    raws.sort_by_key(|r| (r.name_start_byte, r.pattern_index));
    raws.dedup_by(|b, a| a.name_start_byte == b.name_start_byte && a.name == b.name);

    // --- Phase 2b: Associate standalone @doc captures with symbols ---
    // A standalone doc at byte offset N belongs to the symbol whose span contains N
    // (tightest containing symbol). If no containing symbol is found, associate
    // with the *next* symbol after the doc (preceding-doc-comment pattern, e.g.
    // Lua `---` comments that sit above a function).
    if !standalone_docs.is_empty() {
        for (doc_byte, doc_text) in &standalone_docs {
            // First try: tightest containing symbol.
            let best = raws
                .iter_mut()
                .filter(|r| r.def_start <= *doc_byte && *doc_byte < r.def_end && r.doc.is_none())
                .min_by_key(|r| r.def_end - r.def_start);
            if let Some(sym) = best {
                sym.doc = Some(doc_text.clone());
                continue;
            }
            // Fallback: next symbol whose def_start is after (or at) doc_byte.
            let next = raws
                .iter_mut()
                .filter(|r| r.def_start >= *doc_byte && r.doc.is_none())
                .min_by_key(|r| r.def_start);
            if let Some(sym) = next {
                sym.doc = Some(doc_text.clone());
            }
        }
    }

    // --- Phase 3: Sort by span for containment scoping ---
    raws.sort_by_key(|r| (r.def_start, std::cmp::Reverse(r.def_end)));

    // --- Phase 4: Containment-based scope inference + build StoredSymbols ---
    struct ScopeEntry {
        span_end: usize,
        name: String,
        kind_str: String,
    }
    let mut scope_stack: Vec<ScopeEntry> = Vec::new();
    let mut symbols = Vec::new();

    for raw in &raws {
        // Pop scopes that don't contain this symbol.
        while let Some(top) = scope_stack.last() {
            if top.span_end <= raw.def_start {
                scope_stack.pop();
            } else {
                break;
            }
        }

        // Check exclude_inside.
        if let Some(exclude) = &raw.props.exclude_inside
            && scope_stack.iter().any(|s| s.kind_str == *exclude)
        {
            scope_stack.push(ScopeEntry {
                span_end: raw.def_end,
                name: raw.name.clone(),
                kind_str: raw.kind_str.clone(),
            });
            continue;
        }

        // Build qualified name.
        let qualified_name = if let Some(qn) = &raw.qualified_name_override {
            qn.clone()
        } else {
            let scope_names: Vec<String> = scope_stack
                .iter()
                .map(|s| s.name.clone())
                .chain(raw.scope_label.iter().cloned())
                .collect();
            qualify(&scope_names, &raw.name)
        };

        // Push as scope for containment.
        scope_stack.push(ScopeEntry {
            span_end: raw.def_end,
            name: raw.name.clone(),
            kind_str: raw.kind_str.clone(),
        });

        if raw.props.scope_only {
            continue;
        }

        let summary = raw.doc.clone().unwrap_or_else(|| raw.def_signature.clone());
        let source_hash = blake3::hash(raw.def_source_text.as_bytes())
            .to_hex()
            .to_string();

        symbols.push(StoredSymbol {
            id: format!("{}::{}#{}", file_path, qualified_name, raw.kind.as_str()),
            extraction_tier: ExtractionTier::CuratedQueryPack,
            language: ext.language_name.to_string(),
            file_path: file_path.to_string(),
            qualified_name,
            name: raw.name.clone(),
            kind: raw.kind,
            signature: raw.def_signature.clone(),
            summary,
            start_line: (raw.def_start_row + 1) as u32,
            end_line: (raw.def_end_row + 1) as u32,
            start_byte: raw.def_start,
            end_byte: raw.def_end,
            source_hash,
            source_text: raw.def_source_text.clone(),
            file_size_bytes: source.len() as u64,
        });
    }

    crate::extractors::engine::cpp_header_language_fallback::adjust_language_for_cpp_header_if_plain_c(
        &mut symbols,
        ext.language_name,
        file_path,
        source,
    );

    symbols.sort_by_key(|s| (s.start_byte, s.end_byte));
    Ok(symbols)
}
