//! Raw capture rows before deduplication and scope inference.

use crate::model::SymbolKind;

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

pub(super) fn parse_symbol_kind(kind_str: &str) -> Option<SymbolKind> {
    Some(match kind_str {
        "function" => SymbolKind::Function,
        "class" => SymbolKind::Class,
        "method" => SymbolKind::Method,
        "constant" => SymbolKind::Constant,
        "type" => SymbolKind::Type,
        "module" | "namespace" => SymbolKind::Type,
        _ => return None,
    })
}
