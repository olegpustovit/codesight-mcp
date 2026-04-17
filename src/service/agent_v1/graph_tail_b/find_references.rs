use std::collections::HashSet;

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;

fn ci_word_match(body: &str, name: &str) -> bool {
    let body_lc = body.to_lowercase();
    let name_lc = name.to_lowercase();
    codesight_analysis::graph::word_match(&body_lc, &name_lc)
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

    let mut out: Vec<RefMatch> = Vec::new();
    let mut seen_files: HashSet<String> = HashSet::new();
    for sym in &sym_infos {
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
        if seen_files.contains(file) {
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
            return self.find_references_batch(&store, identifiers, ci_word_match);
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

        self.find_references_single(&store, identifier, args, ci_word_match)
    }

    fn find_references_batch(
        &self,
        store: &codesight_storage::storage::IndexStore,
        identifiers: &[String],
        ci_word_match: fn(&str, &str) -> bool,
    ) -> anyhow::Result<Value> {
        let sym_infos = store.all_symbols_with_source()?;
        let all_contents = store.all_file_contents().unwrap_or_default();
        let mut results: Vec<Value> = Vec::new();
        for ident in identifiers {
            let mut file_refs: Vec<Value> = Vec::new();
            let mut seen_files: HashSet<String> = HashSet::new();
            for sym in &sym_infos {
                if sym.name.eq_ignore_ascii_case(ident) {
                    continue;
                }
                if ci_word_match(&sym.source, ident) && seen_files.insert(sym.file.clone()) {
                    file_refs.push(json!({
                        "symbol_id": sym.id,
                        "name": sym.name,
                        "file": sym.file,
                        "line": sym.line,
                    }));
                }
            }
            for (file, content) in &all_contents {
                if seen_files.contains(file) {
                    continue;
                }
                if ci_word_match(content, ident) {
                    seen_files.insert(file.clone());
                    file_refs.push(json!({
                        "file": file,
                        "name": ident,
                        "line": 0,
                        "kind": "text_match",
                    }));
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
        self.success_response_with_meta_extra(
            &MetaBuilder::new(0),
            json!({
                "graph": "find_references",
                "total_references": total_references,
                "results": results,
            }),
            Vec::new(),
            [(
                "tip".to_string(),
                json!("Use check_references for import-only checks."),
            )],
        )
    }

    fn find_references_single(
        &self,
        store: &codesight_storage::storage::IndexStore,
        identifier: &str,
        args: &InspectGraphArgs,
        ci_word_match: fn(&str, &str) -> bool,
    ) -> anyhow::Result<Value> {
        let sym_infos = store.all_symbols_with_source()?;
        let all_contents = store.all_file_contents().unwrap_or_default();
        let include_call_chain = args.include_call_chain.unwrap_or(false);

        let mut file_map: std::collections::BTreeMap<String, Vec<Value>> =
            std::collections::BTreeMap::new();
        let mut seen_files: HashSet<String> = HashSet::new();
        for sym in &sym_infos {
            if sym.name.eq_ignore_ascii_case(identifier) {
                continue;
            }
            if ci_word_match(&sym.source, identifier) {
                seen_files.insert(sym.file.clone());
                file_map.entry(sym.file.clone()).or_default().push(json!({
                    "symbol_id": sym.id,
                    "name": sym.name,
                    "line": sym.line,
                    "kind": sym.kind,
                }));
            }
        }

        for (file, content) in &all_contents {
            if seen_files.contains(file) {
                continue;
            }
            if ci_word_match(content, identifier) {
                seen_files.insert(file.clone());
                file_map.entry(file.clone()).or_default().push(json!({
                    "name": identifier,
                    "line": 0,
                    "kind": "text_match",
                }));
            }
        }

        let _call_edges = if include_call_chain {
            Some(codesight_analysis::graph::build_call_edges(&sym_infos))
        } else {
            None
        };

        let mut refs: Vec<Value> = Vec::new();
        for (file, matches) in &file_map {
            let mut file_ref = json!({
                "file": file,
                "matches": matches,
            });
            if include_call_chain {
                let file_syms: Vec<&codesight_analysis::graph::SymbolInfo> =
                    sym_infos.iter().filter(|s| &s.file == file).collect();
                let mut calling_symbols: Vec<Value> = Vec::new();
                for fs in &file_syms {
                    if fs.name.eq_ignore_ascii_case(identifier) {
                        continue;
                    }
                    if ci_word_match(&fs.source, identifier) {
                        calling_symbols.push(json!({
                            "id": fs.id,
                            "name": fs.name,
                            "kind": fs.kind,
                            "line": fs.line,
                        }));
                    }
                }
                if !calling_symbols.is_empty() {
                    file_ref["calling_symbols"] = json!(calling_symbols);
                }
            }
            refs.push(file_ref);
        }

        let total_count: usize = file_map.values().map(|m| m.len()).sum();
        self.success_response_with_meta_extra(
            &MetaBuilder::new(0),
            json!({
                "graph": "find_references",
                "identifier": identifier,
                "reference_count": total_count,
                "references": refs,
            }),
            Vec::new(),
            [(
                "tip".to_string(),
                json!("Use check_references for import-only checks."),
            )],
        )
    }
}
