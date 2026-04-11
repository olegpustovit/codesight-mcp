//! Scope inference and symbol finalization after raw capture collection.

use crate::core::pipeline::StoredSymbol;
use crate::core::symbols::ExtractionTier;
use crate::extractors::common::qualify;

use super::ScmExtractor;
use super::capture::{OwnedCapture, StandaloneDocCapture};
use super::docs::clean_docstring;

pub(super) fn dedupe_captures(raws: &mut Vec<OwnedCapture>) {
    raws.sort_by_key(|raw| (raw.name_start_byte, raw.pattern_index));
    raws.dedup_by(|right, left| {
        left.name_start_byte == right.name_start_byte && left.name == right.name
    });
}

pub(super) fn attach_standalone_docs(
    raws: &mut [OwnedCapture],
    standalone_docs: &[StandaloneDocCapture],
) {
    // A standalone doc at byte offset N belongs to the symbol whose span contains N
    // (tightest containing symbol). If no containing symbol is found, associate
    // with the next symbol after the doc (preceding-doc-comment pattern).
    for standalone in standalone_docs {
        let best = raws
            .iter_mut()
            .filter(|raw| {
                raw.def_start <= standalone.byte_offset
                    && standalone.byte_offset < raw.def_end
                    && raw.doc.is_none()
            })
            .min_by_key(|raw| raw.def_end - raw.def_start);
        if let Some(symbol) = best {
            symbol.doc = Some(standalone.raw_text.clone());
            continue;
        }

        let next = raws
            .iter_mut()
            .filter(|raw| raw.def_start >= standalone.byte_offset && raw.doc.is_none())
            .min_by_key(|raw| raw.def_start);
        if let Some(symbol) = next {
            symbol.doc = Some(standalone.raw_text.clone());
        }
    }
}

pub(super) fn infer_scopes_and_finalize_symbols(
    extractor: &ScmExtractor,
    file_path: &str,
    source: &str,
    raws: &mut Vec<OwnedCapture>,
) -> Vec<StoredSymbol> {
    raws.sort_by_key(|raw| (raw.def_start, std::cmp::Reverse(raw.def_end)));

    struct ScopeEntry {
        span_end: usize,
        name: String,
        kind_str: String,
    }

    let mut scope_stack: Vec<ScopeEntry> = Vec::new();
    let mut symbols = Vec::new();

    for raw in raws {
        while let Some(top) = scope_stack.last() {
            if top.span_end <= raw.def_start {
                scope_stack.pop();
            } else {
                break;
            }
        }

        if let Some(exclude) = &raw.props.exclude_inside
            && scope_stack.iter().any(|scope| scope.kind_str == *exclude)
        {
            scope_stack.push(ScopeEntry {
                span_end: raw.def_end,
                name: raw.name.clone(),
                kind_str: raw.kind_str.clone(),
            });
            continue;
        }

        let qualified_name = if let Some(qualified_name) = &raw.qualified_name_override {
            qualified_name.clone()
        } else {
            let scope_names: Vec<String> = scope_stack
                .iter()
                .map(|scope| scope.name.clone())
                .chain(raw.scope_label.iter().cloned())
                .collect();
            qualify(&scope_names, &raw.name)
        };

        scope_stack.push(ScopeEntry {
            span_end: raw.def_end,
            name: raw.name.clone(),
            kind_str: raw.kind_str.clone(),
        });

        if raw.props.scope_only {
            continue;
        }

        let summary = raw
            .doc
            .as_deref()
            .map(clean_docstring)
            .filter(|doc| !doc.is_empty())
            .unwrap_or_else(|| raw.def_signature.clone());
        let source_hash = blake3::hash(raw.def_source_text.as_bytes())
            .to_hex()
            .to_string();

        symbols.push(StoredSymbol {
            id: format!("{}::{}#{}", file_path, qualified_name, raw.kind.as_str()),
            extraction_tier: ExtractionTier::CuratedQueryPack,
            language: extractor.compiled.language_name.to_string(),
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

    symbols
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    #[test]
    fn scope_inference_qualifies_method_and_cleans_doc() -> Result<()> {
        let source = r#"
class UserService:
    def find_user(self):
        """ Find a user. """
        return None
"#;
        let extractor = ScmExtractor::new(
            &super::super::ScmMetadata {
                name: "python".to_string(),
                extensions: vec!["py".to_string()],
                grammar: "python".to_string(),
                override_extractor: None,
            },
            r#"
            (class_definition
              name: (identifier) @name) @definition.class

                (function_definition
                  name: (identifier) @name
                  body: (block
                    (expression_statement (string) @doc))) @definition.method
            "#,
        )?;
        let tree = super::super::capture::parse_host_tree(&extractor, source)?;
        let mut raws = super::super::capture::collect_raw_captures(&extractor, source, &tree)?.raws;

        dedupe_captures(&mut raws);
        let symbols = infer_scopes_and_finalize_symbols(&extractor, "svc.py", source, &mut raws);
        let method = symbols
            .iter()
            .find(|symbol| symbol.name == "find_user")
            .expect("method");
        assert_eq!(method.qualified_name, "UserService.find_user");
        assert_eq!(method.summary, "Find a user.");
        Ok(())
    }
}
