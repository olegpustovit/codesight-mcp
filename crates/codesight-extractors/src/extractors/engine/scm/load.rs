//! Query-pack loading phase: resolve grammar and compile `tags.scm` / optional `injections.scm`.
//!
//! Called lazily by [`super::ScmExtractor::compiled`] on first `extract()`, so each pack's
//! `Query::new` cost (10–50 ms) is only paid when that language actually appears in the
//! workspace — not up-front for all ~30 packs.

use anyhow::{Result, anyhow};
use tree_sitter::Query;

pub(super) struct CompiledQueryPack {
    pub(super) grammar: tree_sitter::Language,
    pub(super) tags_query: Query,
    pub(super) injections_query: Option<Query>,
}

pub(super) fn compile_queries(
    grammar_name: &str,
    query_source: &str,
    injections_source: Option<&str>,
    name_for_err: &str,
) -> Result<CompiledQueryPack> {
    let grammar = crate::extractors::engine::builtin_grammar(grammar_name)
        .ok_or_else(|| anyhow!("unknown grammar: {grammar_name}"))?;

    let tags_query = Query::new(&grammar, query_source)
        .map_err(|error| anyhow!("query compile error for {name_for_err}: {error}"))?;
    let injections_query = injections_source
        .map(|source| {
            Query::new(&grammar, source)
                .map_err(|error| anyhow!("injections compile error for {name_for_err}: {error}"))
        })
        .transpose()?;

    Ok(CompiledQueryPack {
        grammar,
        tags_query,
        injections_query,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_queries_accepts_optional_injections_query() {
        let compiled = compile_queries(
            "json",
            r#"(object) @definition.class"#,
            Some(r#"(string) @injection.content"#),
            "json",
        )
        .expect("compile json pack");

        assert!(compiled.injections_query.is_some());
    }
}
