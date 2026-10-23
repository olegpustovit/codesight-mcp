//! Raw capture collection before scope inference/finalization.

use anyhow::{Result, anyhow};
use streaming_iterator::StreamingIterator;
use tree_sitter::{QueryCursor, Tree};

use crate::extract_tree_sitter::{node_text, parser_for_language};

use symslice_core::symbols::SymbolKind;

use super::ScmExtractor;
use super::pattern::PatternProps;

pub(super) struct OwnedCapture {
    pub(super) kind: SymbolKind,
    pub(super) kind_str: String,
    pub(super) name: String,
    pub(super) name_start_byte: usize,
    pub(super) def_start: usize,
    pub(super) def_end: usize,
    pub(super) def_start_row: usize,
    pub(super) def_end_row: usize,
    pub(super) def_signature: String,
    pub(super) def_source_text: String,
    pub(super) doc: Option<String>,
    pub(super) scope_label: Option<String>,
    pub(super) qualified_name_override: Option<String>,
    pub(super) pattern_index: usize,
    pub(super) props: PatternProps,
}

pub(super) struct StandaloneDocCapture {
    pub(super) byte_offset: usize,
    pub(super) raw_text: String,
}

pub(super) struct CaptureBatch {
    pub(super) raws: Vec<OwnedCapture>,
    pub(super) standalone_docs: Vec<StandaloneDocCapture>,
}

pub(super) fn parse_symbol_kind(kind_str: &str) -> Option<SymbolKind> {
    Some(match kind_str {
        "function" => SymbolKind::Function,
        "class" => SymbolKind::Class,
        "method" => SymbolKind::Method,
        "constant" => SymbolKind::Constant,
        "type" => SymbolKind::Type,
        "module" | "namespace" => SymbolKind::Type,
        "reexport" => SymbolKind::Reexport,
        _ => return None,
    })
}

pub(super) fn parse_host_tree(ext: &ScmExtractor, source: &str) -> Result<Tree> {
    parser_for_language(&ext.compiled().grammar, |parser| {
        parser
            .parse(source, None)
            .ok_or_else(|| anyhow!("parse returned no tree"))
    })
}

pub(super) fn collect_raw_captures(
    ext: &ScmExtractor,
    source: &str,
    tree: &Tree,
) -> Result<CaptureBatch> {
    let compiled = ext.compiled();
    let mut cursor = QueryCursor::new();
    let capture_names = compiled.tags_query.capture_names();

    // QueryMatches is a StreamingIterator — text predicates (#match?, #eq?) are
    // checked automatically. We extract owned data from each match before advancing.
    let mut raws = Vec::new();
    let mut standalone_docs = Vec::new();
    let mut matches = cursor.matches(&compiled.tags_query, tree.root_node(), source.as_bytes());
    while let Some(m) = matches.next() {
        let mut props = super::pattern::read_pattern_props(&compiled.tags_query, m.pattern_index);

        let mut def_start = usize::MAX;
        let mut def_end = 0usize;
        let mut def_start_row = usize::MAX;
        let mut def_end_row = 0usize;
        let mut def_kind_str: Option<String> = None;
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
                // Join same-kind sibling spans; mixed-kind workspace packs
                // retain their previous last-definition semantics.
                if def_kind_str.as_deref() != Some(kind_part) {
                    def_start = cap.node.start_byte();
                    def_end = cap.node.end_byte();
                    def_start_row = cap.node.start_position().row;
                    def_end_row = cap.node.end_position().row;
                } else {
                    def_start = def_start.min(cap.node.start_byte());
                    def_end = def_end.max(cap.node.end_byte());
                    def_start_row = def_start_row.min(cap.node.start_position().row);
                    def_end_row = def_end_row.max(cap.node.end_position().row);
                }
                def_kind_str = Some(kind_part.to_string());
            } else {
                match cap_name {
                    "name" => {
                        name_text = node_text(cap.node, source).ok().map(|s| s.to_string());
                        name_start_byte = cap.node.start_byte();
                    }
                    "doc" => {
                        doc_text = node_text(cap.node, source).ok().map(|s| s.to_string());
                    }
                    "scope" => {
                        scope_text = node_text(cap.node, source)
                            .ok()
                            .map(|t| t.trim_start_matches('*').to_string());
                    }
                    "qualified_name" => {
                        qname_text = node_text(cap.node, source).ok().map(|s| s.to_string());
                    }
                    _ => {}
                }
            }
        }

        if !has_def {
            if let Some(doc) = doc_text {
                standalone_docs.push(StandaloneDocCapture {
                    byte_offset: m
                        .captures
                        .first()
                        .map_or(0, |capture| capture.node.start_byte()),
                    raw_text: doc,
                });
            }
            continue;
        }
        let Some(kind_str) = def_kind_str else {
            continue;
        };

        let name = if let Some(fixed) = &props.fixed_name {
            fixed.clone()
        } else if let Some(name) = name_text {
            if let Some(prefix) = &props.name_prefix {
                format!("{prefix}{name}")
            } else {
                name
            }
        } else {
            continue;
        };

        let Some(kind) = parse_symbol_kind(&kind_str) else {
            continue;
        };

        if capture_props.exclude_inside.is_some() {
            props.exclude_inside = capture_props.exclude_inside;
        }
        props.scope_only |= capture_props.scope_only;

        let def_source_text = source
            .get(def_start..def_end)
            .unwrap_or_default()
            .to_string();
        let def_signature = def_source_text
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
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

    Ok(CaptureBatch {
        raws,
        standalone_docs,
    })
}
