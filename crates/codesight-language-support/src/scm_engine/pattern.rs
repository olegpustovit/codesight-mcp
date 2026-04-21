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

#[cfg(test)]
mod tests {
    use tree_sitter::Query;

    use super::*;

    #[test]
    fn reads_supported_set_properties_from_query_pattern() {
        let language =
            crate::builtin_grammar("python").expect("python grammar");
        let query = Query::new(
            &language,
            r#"
            (
              (class_definition name: (identifier) @name) @definition.class
              (#set! fixed_name "Fixed")
              (#set! scope_only)
              (#set! exclude_inside "function")
              (#set! name_prefix "Prefix::")
            )
            "#,
        )
        .expect("compile query");

        let props = read_pattern_props(&query, 0);
        assert_eq!(props.fixed_name.as_deref(), Some("Fixed"));
        assert_eq!(props.exclude_inside.as_deref(), Some("function"));
        assert_eq!(props.name_prefix.as_deref(), Some("Prefix::"));
        assert!(props.scope_only);
    }
}
