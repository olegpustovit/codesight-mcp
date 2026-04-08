//! Per-query-pattern metadata from tree-sitter `#set!` property directives.

use tree_sitter::Query;

#[derive(Debug, Clone, Default)]
pub(super) struct PatternProps {
    /// Skip this symbol if contained within a definition of this kind.
    pub(super) exclude_inside: Option<String>,
    /// Create a scope entry but don't emit a StoredSymbol.
    pub(super) scope_only: bool,
    /// Use this literal string as the name (e.g. "init" for Swift).
    pub(super) fixed_name: Option<String>,
    /// Prepend this string to the captured @name text (e.g. "@keyframes ").
    pub(super) name_prefix: Option<String>,
}

pub(super) fn read_pattern_props(query: &Query, pattern_index: usize) -> PatternProps {
    let mut props = PatternProps::default();
    for p in query.property_settings(pattern_index) {
        match p.key.as_ref() {
            "exclude_inside" => {
                props.exclude_inside = p.value.as_ref().map(|v| v.to_string());
            }
            "scope_only" => props.scope_only = true,
            "fixed_name" => {
                props.fixed_name = p.value.as_ref().map(|v| v.to_string());
            }
            "name_prefix" => {
                props.name_prefix = p.value.as_ref().map(|v| v.to_string());
            }
            _ => {}
        }
    }
    props
}
