//! Query-pack loading phase: resolve grammar, compile `tags.scm`, and precompile optional
//! `injections.scm` / postprocess configuration.

use anyhow::{Result, anyhow};
use tree_sitter::Query;

use super::metadata::ScmMetadata;
use super::postprocess::{PostprocessStep, postprocess_steps_for};
use crate::extractors::engine::catalogs::QueryPackDefinition;

pub(super) struct CompiledQueryPack {
    pub(super) language_name: &'static str,
    pub(super) extensions: &'static [&'static str],
    pub(super) grammar: tree_sitter::Language,
    pub(super) tags_query: Query,
    pub(super) injections_query: Option<Query>,
    pub(super) postprocess_steps: &'static [PostprocessStep],
}

pub(super) fn compile_pack(pack: &QueryPackDefinition) -> Result<CompiledQueryPack> {
    compile_query_pack_with_injections(
        &pack.metadata,
        &pack.query_source,
        pack.injections_source.as_deref(),
    )
}

pub(super) fn compile_query_pack(
    meta: &ScmMetadata,
    query_source: &str,
) -> Result<CompiledQueryPack> {
    compile_query_pack_with_injections(meta, query_source, None)
}

fn compile_query_pack_with_injections(
    meta: &ScmMetadata,
    query_source: &str,
    injections_source: Option<&str>,
) -> Result<CompiledQueryPack> {
    let grammar = crate::extractors::engine::builtin_grammar(&meta.grammar)
        .ok_or_else(|| anyhow!("unknown grammar: {}", meta.grammar))?;

    let tags_query = Query::new(&grammar, query_source)
        .map_err(|error| anyhow!("query compile error for {}: {error}", meta.name))?;
    let injections_query = injections_source
        .map(|source| {
            Query::new(&grammar, source)
                .map_err(|error| anyhow!("injections compile error for {}: {error}", meta.name))
        })
        .transpose()?;

    let language_name: &'static str = Box::leak(meta.name.clone().into_boxed_str());
    let extensions: &'static [&'static str] = Box::leak(
        meta.extensions
            .iter()
            .map(|ext| &*Box::leak(ext.clone().into_boxed_str()))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );

    Ok(CompiledQueryPack {
        language_name,
        extensions,
        grammar,
        tags_query,
        injections_query,
        postprocess_steps: postprocess_steps_for(meta),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_pack_accepts_optional_injections_query() {
        let metadata = ScmMetadata {
            name: "json".to_string(),
            extensions: vec!["json".to_string()],
            grammar: "json".to_string(),
            override_extractor: None,
        };
        let compiled = compile_query_pack_with_injections(
            &metadata,
            r#"(object) @definition.class"#,
            Some(r#"(string) @injection.content"#),
        )
        .expect("compile json pack");

        assert!(compiled.injections_query.is_some());
    }
}
