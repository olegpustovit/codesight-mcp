use std::collections::HashSet;

use codesight_language_support::is_non_code_reference_path;
use codesight_language_support::text_match::ci_word_match;
use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;

struct RefFilter<'a> {
    scope: Option<&'a str>,
    exclude: &'a [String],
    include_non_code: bool,
}

impl<'a> RefFilter<'a> {
    fn from_args(args: &'a InspectGraphArgs) -> Self {
        Self {
            scope: args.scope.as_deref(),
            exclude: args.exclude.as_deref().unwrap_or(&[]),
            include_non_code: args.include_non_code.unwrap_or(false),
        }
    }

    fn new_permissive() -> Self {
        Self {
            scope: None,
            exclude: &[],
            include_non_code: false,
        }
    }

    fn admits(&self, path: &str) -> bool {
        if !self.include_non_code && is_non_code_reference_path(path) {
            return false;
        }
        if let Some(prefix) = self.scope
            && !path.starts_with(prefix)
            && !path.contains(prefix)
        {
            return false;
        }
        for pattern in self.exclude {
            if path.starts_with(pattern) || path.contains(pattern) {
                return false;
            }
        }
        true
    }
}

/// Pure reference match — no MCP envelope, usable from other tools.
#[derive(Debug, Clone)]
pub(crate) struct RefMatch {
    pub file: String,
    pub line: u32,
    pub kind: &'static str,
    pub preview: String,
}

/// Collect whole-word references to `name` in the indexed workspace.
/// `definition_id` (optional) is excluded from symbol-body matches.
pub(crate) fn collect_references(
    store: &codesight_storage::storage::IndexStore,
    name: &str,
) -> anyhow::Result<Vec<RefMatch>> {
    let sym_infos = store.all_symbols_with_source()?;
    let all_contents = store.all_file_contents().unwrap_or_default();
    let filter = RefFilter::new_permissive();

    let mut out: Vec<RefMatch> = Vec::new();
    let mut seen_files: HashSet<String> = HashSet::new();
    for sym in &sym_infos {
        if !filter.admits(&sym.file) {
            continue;
        }
        if sym.name.eq_ignore_ascii_case(name) {
            out.push(RefMatch {
                file: sym.file.clone(),
                line: sym.line,
                kind: "definition",
                preview: sym.source.lines().next().unwrap_or("").to_string(),
            });
            seen_files.insert(sym.file.clone());
            continue;
        }
        if ci_word_match(&sym.source, name) {
            out.push(RefMatch {
                file: sym.file.clone(),
                line: sym.line,
                kind: "symbol_body_match",
                preview: sym.name.clone(),
            });
            seen_files.insert(sym.file.clone());
        }
    }
    for (file, content) in &all_contents {
        if seen_files.contains(file) || !filter.admits(file) {
            continue;
        }
        if ci_word_match(content, name) {
            out.push(RefMatch {
                file: file.clone(),
                line: 0,
                kind: "text_match",
                preview: String::new(),
            });
        }
    }
    Ok(out)
}

impl CodeService {
    pub(super) fn graph_find_references(&self, args: &InspectGraphArgs) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &MetaBuilder::new(0));

        if let Some(identifiers) = &args.identifiers {
            return self.find_references_batch(&store, identifiers, args);
        }

        let Some(identifier) = args.identifier.as_deref() else {
            return self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "find_references mode requires identifier.".into(),
                    detail: json!({}),
                },
            );
        };

        self.find_references_single(&store, identifier, args)
    }

    fn find_references_batch(
        &self,
        store: &codesight_storage::storage::IndexStore,
        identifiers: &[String],
        args: &InspectGraphArgs,
    ) -> anyhow::Result<Value> {
        let sym_infos = store.all_symbols_with_source()?;
        let all_contents = store.all_file_contents().unwrap_or_default();
        let filter = RefFilter::from_args(args);
        let mut results: Vec<Value> = Vec::new();
        for ident in identifiers {
            let mut file_refs: Vec<Value> = Vec::new();
            let mut seen_files: HashSet<String> = HashSet::new();
            // Flat row per match: {file, line, name} + kind only when
            // non-default. `symbol_id` is reconstructible from file + name.
            for sym in &sym_infos {
                if !filter.admits(&sym.file) {
                    continue;
                }
                if sym.name.eq_ignore_ascii_case(ident) {
                    continue;
                }
                if ci_word_match(&sym.source, ident) && seen_files.insert(sym.file.clone()) {
                    file_refs.push(compact_match_row(&sym.file, sym.line, &sym.name, &sym.kind));
                }
            }
            for (file, content) in &all_contents {
                if seen_files.contains(file) || !filter.admits(file) {
                    continue;
                }
                if ci_word_match(content, ident) {
                    seen_files.insert(file.clone());
                    file_refs.push(compact_match_row(file, 0, ident, "text_match"));
                }
            }
            results.push(json!({
                "identifier": ident,
                "reference_count": file_refs.len(),
                "references": file_refs,
            }));
        }
        let total_references: usize = results
            .iter()
            .filter_map(|r| r.get("reference_count")?.as_u64())
            .sum::<u64>() as usize;
        self.success_response_with_warnings(
            &MetaBuilder::new(0),
            json!({
                "graph": "find_references",
                "total_references": total_references,
                "results": results,
            }),
            Vec::new(),
        )
    }

    fn find_references_single(
        &self,
        store: &codesight_storage::storage::IndexStore,
        identifier: &str,
        args: &InspectGraphArgs,
    ) -> anyhow::Result<Value> {
        let sym_infos = store.all_symbols_with_source()?;
        let all_contents = store.all_file_contents().unwrap_or_default();
        let include_call_chain = args.include_call_chain.unwrap_or(false);
        let filter = RefFilter::from_args(args);

        // Flat row per match. Replaces the {file, matches:[…]} nesting: when
        // most files have 1–2 matches the nested shape paid a key-per-row
        // tax for no real compression (PLAN2 §3.4b).
        let mut refs: Vec<Value> = Vec::new();
        let mut seen_files: HashSet<String> = HashSet::new();

        for sym in &sym_infos {
            if !filter.admits(&sym.file) {
                continue;
            }
            if sym.name.eq_ignore_ascii_case(identifier) {
                continue;
            }
            if ci_word_match(&sym.source, identifier) {
                seen_files.insert(sym.file.clone());
                refs.push(compact_match_row(&sym.file, sym.line, &sym.name, &sym.kind));
            }
        }

        for (file, content) in &all_contents {
            if seen_files.contains(file) || !filter.admits(file) {
                continue;
            }
            if ci_word_match(content, identifier) {
                seen_files.insert(file.clone());
                refs.push(compact_match_row(file, 0, identifier, "text_match"));
            }
        }

        let _call_edges = if include_call_chain {
            Some(codesight_analysis::graph::build_call_edges(&sym_infos))
        } else {
            None
        };

        if include_call_chain {
            // `calling_symbols` is diagnostic context only — attach per-file
            // via a separate array so the flat row shape stays intact.
            let mut per_file: std::collections::BTreeMap<String, Vec<Value>> =
                std::collections::BTreeMap::new();
            for sym in &sym_infos {
                if sym.name.eq_ignore_ascii_case(identifier) {
                    continue;
                }
                if !filter.admits(&sym.file) {
                    continue;
                }
                if ci_word_match(&sym.source, identifier) {
                    per_file.entry(sym.file.clone()).or_default().push(json!({
                        "id": sym.id,
                        "name": sym.name,
                        "kind": sym.kind,
                        "line": sym.line,
                    }));
                }
            }
            let chain: Vec<Value> = per_file
                .into_iter()
                .map(|(file, callers)| json!({ "file": file, "calling_symbols": callers }))
                .collect();
            let total_count = refs.len();
            return self.success_response_with_warnings(
                &MetaBuilder::new(0),
                json!({
                    "graph": "find_references",
                    "identifier": identifier,
                    "reference_count": total_count,
                    "references": refs,
                    "call_chain": chain,
                }),
                Vec::new(),
            );
        }

        let total_count = refs.len();
        self.success_response_with_warnings(
            &MetaBuilder::new(0),
            json!({
                "graph": "find_references",
                "identifier": identifier,
                "reference_count": total_count,
                "references": refs,
            }),
            Vec::new(),
        )
    }
}

/// Compact per-match row: `{file, line, name}` + `kind` only when
/// non-default. "function" is the dominant kind and is assumed when absent.
fn compact_match_row(file: &str, line: u32, name: &str, kind: &str) -> Value {
    let mut m = serde_json::Map::new();
    m.insert("file".to_string(), json!(file));
    m.insert("line".to_string(), json!(line));
    m.insert("name".to_string(), json!(name));
    if kind != "function" {
        m.insert("kind".to_string(), json!(kind));
    }
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_code_path_classifier() {
        assert!(is_non_code_reference_path("scripts/stress/x.json"));
        assert!(is_non_code_reference_path("docs/README.md"));
        assert!(is_non_code_reference_path("Cargo.lock"));
        assert!(!is_non_code_reference_path("src/main.rs"));
        assert!(!is_non_code_reference_path("foo.py"));
        assert!(!is_non_code_reference_path("no-extension"));
    }

    #[test]
    fn filter_rejects_non_code_by_default() {
        let args = InspectGraphArgs {
            source_id: "x".into(),
            graph: "find_references".into(),
            r#ref: None,
            limit: 0,
            repo_path: None,
            module_path: None,
            rules: None,
            symbol_id: None,
            direction: None,
            depth: None,
            algorithm: None,
            top_n: None,
            file_path: None,
            identifier: None,
            identifiers: None,
            include_call_chain: None,
            cross_repo: None,
            fqn: None,
            max_tokens: None,
            scope: None,
            exclude: None,
            include_non_code: None,
            file_pattern: None,
        };
        let f = RefFilter::from_args(&args);
        assert!(!f.admits("fixture.json"));
        assert!(f.admits("src/lib.rs"));
    }

    #[test]
    fn filter_scope_and_exclude() {
        let args = InspectGraphArgs {
            source_id: "x".into(),
            graph: "find_references".into(),
            r#ref: None,
            limit: 0,
            repo_path: None,
            module_path: None,
            rules: None,
            symbol_id: None,
            direction: None,
            depth: None,
            algorithm: None,
            top_n: None,
            file_path: None,
            identifier: None,
            identifiers: None,
            include_call_chain: None,
            cross_repo: None,
            fqn: None,
            max_tokens: None,
            scope: Some("src/service/".into()),
            exclude: Some(vec!["tests/".into()]),
            include_non_code: None,
            file_pattern: None,
        };
        let f = RefFilter::from_args(&args);
        assert!(f.admits("src/service/foo.rs"));
        assert!(!f.admits("src/cli/foo.rs"));
        assert!(!f.admits("tests/unit.rs"));
    }

    #[test]
    fn filter_include_non_code_opts_in() {
        let args = InspectGraphArgs {
            source_id: "x".into(),
            graph: "find_references".into(),
            r#ref: None,
            limit: 0,
            repo_path: None,
            module_path: None,
            rules: None,
            symbol_id: None,
            direction: None,
            depth: None,
            algorithm: None,
            top_n: None,
            file_path: None,
            identifier: None,
            identifiers: None,
            include_call_chain: None,
            cross_repo: None,
            fqn: None,
            max_tokens: None,
            scope: None,
            exclude: None,
            include_non_code: Some(true),
            file_pattern: None,
        };
        let f = RefFilter::from_args(&args);
        assert!(f.admits("scripts/stress/x.json"));
    }
}
