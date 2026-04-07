//! Late `inspect_graph` modes (find_references, FQN, cross_source).

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::fuzzy;
use crate::model::SearchSymbol;
use crate::savings::MetaBuilder;
use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::agent_v1::util_graph::{
    build_call_chain, extract_symbol_names_from_content, graph_ref_symbol_id,
};
use crate::service::helpers::ToolError;
use crate::storage::IndexStore;

impl CodeService {
    pub(in crate::service) fn inspect_graph_tail_b(
        &self,
        args: &InspectGraphArgs,
        mode: &str,
        limit: usize,
        warnings: &mut Vec<Value>,
    ) -> anyhow::Result<Value> {
        match mode {
            "find_references" => {
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&MetaBuilder::new(0), e),
                };

                // Helper: case-insensitive word match
                let ci_word_match = |body: &str, name: &str| -> bool {
                    let body_lc = body.to_lowercase();
                    let name_lc = name.to_lowercase();
                    crate::dep_graph::word_match(&body_lc, &name_lc)
                };

                // Batch mode: identifiers array
                if let Some(identifiers) = &args.identifiers {
                    let sym_infos = store.all_symbols_with_source()?;
                    let all_contents = store.all_file_contents().unwrap_or_default();
                    let mut results: Vec<Value> = Vec::new();
                    for ident in identifiers {
                        let mut file_refs: Vec<Value> = Vec::new();
                        let mut seen_files: HashSet<String> = HashSet::new();
                        // Search symbol source texts
                        for sym in &sym_infos {
                            if sym.name.eq_ignore_ascii_case(ident) {
                                continue;
                            }
                            if ci_word_match(&sym.source, ident) {
                                if seen_files.insert(sym.file.clone()) {
                                    file_refs.push(json!({
                                        "symbol_id": sym.id,
                                        "name": sym.name,
                                        "file": sym.file,
                                        "line": sym.line,
                                    }));
                                }
                            }
                        }
                        // Also search file contents for import-level references
                        for (file, content) in &all_contents {
                            if seen_files.contains(file) {
                                continue;
                            }
                            if ci_word_match(content, ident) {
                                seen_files.insert(file.clone());
                                file_refs.push(json!({
                                    "file": file,
                                    "line": 0,
                                }));
                            }
                        }
                        results.push(json!({
                            "identifier": ident,
                            "reference_count": file_refs.len(),
                            "references": file_refs,
                        }));
                    }
                    return self.success_response_with_meta_extra(
                        &MetaBuilder::new(0),
                        json!({
                            "graph": "find_references",
                            "results": results,
                        }),
                        Vec::new(),
                        [(
                            "tip".to_string(),
                            json!("Use check_references for import-only checks."),
                        )],
                    );
                }

                let Some(identifier) = args.identifier.as_deref() else {
                    return self.error_response(
                        &MetaBuilder::new(0),
                        ToolError {
                            code: "INVALID_REQUEST",
                            message: "find_references mode requires identifier.".into(),
                            detail: json!({}),
                        },
                    );
                };
                let sym_infos = store.all_symbols_with_source()?;
                let all_contents = store.all_file_contents().unwrap_or_default();
                let include_call_chain = args.include_call_chain.unwrap_or(false);

                // Group references by file, with matches array
                let mut file_map: std::collections::BTreeMap<String, Vec<Value>> =
                    std::collections::BTreeMap::new();
                let mut seen_files: HashSet<String> = HashSet::new();
                for sym in &sym_infos {
                    if sym.name.eq_ignore_ascii_case(identifier) {
                        continue;
                    } // skip self
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

                // Also search full file contents for references not captured by symbol source
                for (file, content) in &all_contents {
                    if seen_files.contains(file) {
                        continue;
                    }
                    if ci_word_match(content, identifier) {
                        seen_files.insert(file.clone());
                        file_map.entry(file.clone()).or_default().push(json!({
                            "file": file,
                            "line": 0,
                        }));
                    }
                }

                let _call_edges = if include_call_chain {
                    Some(crate::dep_graph::build_call_edges(&sym_infos))
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
                        // For each file, find the symbols in that file that call the identifier
                        let file_syms: Vec<&crate::dep_graph::SymbolInfo> =
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
            "resolve_fqn" => {
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&MetaBuilder::new(0), e),
                };
                let Some(fqn) = args.fqn.as_deref() else {
                    return self.error_response(
                        &MetaBuilder::new(0),
                        ToolError {
                            code: "INVALID_REQUEST",
                            message: "resolve_fqn mode requires fqn parameter.".into(),
                            detail: json!({}),
                        },
                    );
                };
                // Load composer.json PSR-4 map from the indexed repo root
                let root = store.root_path().ok();
                let psr4_map = root.as_ref().and_then(|r| {
                    let composer = r.join("composer.json");
                    let content = std::fs::read_to_string(composer).ok()?;
                    let parsed: Value = serde_json::from_str(&content).ok()?;
                    let autoload = parsed.get("autoload")?.get("psr-4")?.as_object()?;
                    let mut map = std::collections::HashMap::new();
                    for (ns, dir) in autoload {
                        if let Some(d) = dir.as_str() {
                            map.insert(ns.clone(), d.to_string());
                        }
                    }
                    Some(map)
                });
                let Some(psr4_map) = psr4_map else {
                    return self.error_response(
                        &MetaBuilder::new(0),
                        ToolError {
                            code: "NOT_FOUND",
                            message:
                                "No PSR-4 autoload map found (composer.json missing or invalid)."
                                    .into(),
                            detail: json!({}),
                        },
                    );
                };
                let source_files: HashSet<String> = store.all_file_paths()?.into_iter().collect();
                match crate::dep_graph::fqn_to_symbol(fqn, &psr4_map, &source_files) {
                    Some(symbol_id) => self.success_response(
                        &MetaBuilder::new(0),
                        json!({
                            "graph": "resolve_fqn",
                            "fqn": fqn,
                            "resolved": symbol_id,
                        }),
                    ),
                    None => self.error_response(
                        &MetaBuilder::new(0),
                        ToolError {
                            code: "NOT_FOUND",
                            message: format!(
                                "FQN '{}' could not be resolved against the indexed source files.",
                                fqn
                            ),
                            detail: json!({ "fqn": fqn }),
                        },
                    ),
                }
            }
            "cross_source" => {
                warnings.push(json!({
                    "code": "NOT_IMPLEMENTED",
                    "message": "graph mode 'cross_source' needs edge indexes not yet available.",
                }));
                self.success_response_with_warnings(
                    &MetaBuilder::new(0),
                    json!({ "graph": mode, "edges": [], "nodes": [] }),
                    warnings.clone(),
                )
            }
            _ => self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    code: "INVALID_REQUEST",
                    message: format!("Unknown inspect_graph graph mode '{mode}'."),
                    detail: json!({ "graph": args.graph }),
                },
            ),
        }
    }
}
