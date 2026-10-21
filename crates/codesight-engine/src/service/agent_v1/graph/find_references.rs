use rustc_hash::FxHashSet;

use codesight_language_support::is_non_code_reference_path;
use codesight_language_support::mentions_identifier_ci_in_file;
use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::{Confidence, MetaBuilder};

struct RefFilter<'a> {
    scope: Option<&'a str>,
    exclude: &'a [String],
    include_non_code: bool,
    include_vendored: bool,
}

impl<'a> RefFilter<'a> {
    fn from_args(args: &'a InspectGraphArgs) -> Self {
        let scope = args.scope.as_deref();
        // If the caller scopes into an excluded segment they've clearly
        // opted in; don't re-exclude the same paths from under them.
        let scope_opts_in = scope
            .map(codesight_language_support::scope_opts_in)
            .unwrap_or(false);
        Self {
            scope,
            exclude: args.exclude.as_deref().unwrap_or(&[]),
            include_non_code: args.include_non_code.unwrap_or(false),
            include_vendored: args.include_vendored.unwrap_or(false) || scope_opts_in,
        }
    }

    fn admits(&self, path: &str) -> bool {
        if !self.include_non_code && is_non_code_reference_path(path) {
            return false;
        }
        if !self.include_vendored && codesight_language_support::is_vendored_path(path) {
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

use codesight_language_support::text_match::locate_identifier_ci;

/// Name-based candidates: definitions and grammar identifier tokens form the
/// primary rows; unparsed text mentions remain weak evidence.
#[derive(Debug, Clone)]
struct ReferenceRow {
    file: String,
    line: u32,
    column: Option<u32>,
    name: String,
    kind: &'static str,
    enclosing_symbol: Option<String>,
    enclosing_kind: Option<String>,
    /// ROADMAP P0.4 #4: chain of enclosing scope names walking up from the
    /// enclosing symbol toward the file root. Empty when the reference is
    /// top-level or comes from a `file_body_match` candidate row.
    call_chain: Vec<String>,
    /// Grammar identifier syntax or an unresolved text mention.
    body_match_kind: Option<&'static str>,
}

impl ReferenceRow {
    fn to_json(&self) -> Value {
        let mut m = serde_json::Map::new();
        m.insert("file".into(), json!(self.file));
        m.insert("line".into(), json!(self.line));
        if let Some(col) = self.column {
            m.insert("column".into(), json!(col));
        }
        m.insert("name".into(), json!(self.name));
        m.insert("kind".into(), json!(self.kind));
        if let Some(sub) = self.body_match_kind {
            m.insert("body_match_kind".into(), json!(sub));
        }
        if let Some(sym) = &self.enclosing_symbol {
            m.insert("enclosing_symbol".into(), json!(sym));
        }
        if let Some(k) = &self.enclosing_kind {
            m.insert("enclosing_kind".into(), json!(k));
        }
        if !self.call_chain.is_empty() {
            m.insert("call_chain".into(), json!(self.call_chain));
        }
        Value::Object(m)
    }

    fn is_identifier_candidate(&self) -> bool {
        self.kind == "definition" || self.body_match_kind == Some("identifier_syntax")
    }
}

/// Derive the scope chain (ancestor names) from a symbol's stable id using
/// the shared `SymbolIdRef` parser. The chain is the symbol path minus its
/// leaf; empty for top-level symbols.
fn call_chain_from_stable_id(id: &str) -> Vec<String> {
    codesight_core::SymbolIdRef::new(id)
        .scopes()
        .map(str::to_string)
        .collect()
}

fn collect_rows(
    sym_infos: &[codesight_storage::storage::SymbolSourceRecord],
    all_contents: &[(String, String)],
    identifier: &str,
    filter: &RefFilter<'_>,
    skip_definition: bool,
) -> Vec<ReferenceRow> {
    let mut rows: Vec<ReferenceRow> = Vec::new();
    let mut seen_files: FxHashSet<String> = FxHashSet::default();

    let positions: std::collections::BTreeMap<_, _> = all_contents
        .iter()
        .filter(|(file, source)| filter.admits(file) && source.contains(identifier))
        .map(|(file, source)| {
            (
                file.as_str(),
                codesight_language_support::identifier_syntax::positions(file, source, identifier),
            )
        })
        .collect();

    for sym in sym_infos {
        if !filter.admits(&sym.file) {
            continue;
        }
        if sym.name.eq_ignore_ascii_case(identifier) {
            if skip_definition {
                continue;
            }
            let (line, column) = locate_identifier_ci(&sym.source, identifier)
                .map(|(rel_line, col)| (sym.line + rel_line - 1, Some(col)))
                .unwrap_or((sym.line, None));
            rows.push(ReferenceRow {
                file: sym.file.clone(),
                line,
                column,
                name: sym.name.clone(),
                kind: "definition",
                enclosing_symbol: None,
                enclosing_kind: None,
                call_chain: call_chain_from_stable_id(&sym.id),
                body_match_kind: None,
            });
            seen_files.insert(sym.file.clone());
            continue;
        }
        if mentions_identifier_ci_in_file(&sym.file, &sym.source, identifier) {
            let located = positions
                .get(sym.file.as_str())
                .and_then(Option::as_ref)
                .and_then(|positions| {
                    positions
                        .iter()
                        .copied()
                        .find(|(line, _)| *line >= sym.line && *line <= sym.end_line)
                });
            let (line, column) =
                located
                    .map(|(line, col)| (line, Some(col)))
                    .unwrap_or_else(|| {
                        locate_identifier_ci(&sym.source, identifier)
                            .map(|(line, col)| (sym.line + line - 1, Some(col)))
                            .unwrap_or((sym.line, None))
                    });
            let body_match_kind = Some(if located.is_some() {
                "identifier_syntax"
            } else {
                "text_mention"
            });
            let mut chain = call_chain_from_stable_id(&sym.id);
            chain.push(sym.name.clone());
            rows.push(ReferenceRow {
                file: sym.file.clone(),
                line,
                column,
                name: sym.name.clone(),
                kind: "symbol_body_match",
                enclosing_symbol: Some(sym.name.clone()),
                enclosing_kind: Some(sym.kind.clone()),
                call_chain: chain,
                body_match_kind,
            });
            seen_files.insert(sym.file.clone());
        }
    }

    for (file, content) in all_contents {
        if seen_files.contains(file) || !filter.admits(file) {
            continue;
        }
        let syntax = positions
            .get(file.as_str())
            .and_then(Option::as_ref)
            .and_then(|p| p.first())
            .copied();
        let Some((line, col)) = syntax.or_else(|| locate_identifier_ci(content, identifier)) else {
            continue;
        };
        rows.push(ReferenceRow {
            file: file.clone(),
            line,
            column: Some(col),
            name: identifier.to_string(),
            kind: "file_body_match",
            enclosing_symbol: None,
            enclosing_kind: None,
            call_chain: Vec::new(),
            body_match_kind: Some(if syntax.is_some() {
                "identifier_syntax"
            } else {
                "text_mention"
            }),
        });
        seen_files.insert(file.clone());
    }

    rows
}

fn split_identifier_and_text(rows: Vec<ReferenceRow>) -> (Vec<Value>, Vec<Value>) {
    let mut primary = Vec::new();
    let mut candidate = Vec::new();
    for row in rows {
        let json = row.to_json();
        if row.is_identifier_candidate() {
            primary.push(json);
        } else {
            candidate.push(json);
        }
    }
    (primary, candidate)
}

fn partial_references_warning(candidate_count: usize) -> Vec<Value> {
    if candidate_count == 0 {
        return Vec::new();
    }
    vec![json!({
        "code": "PARTIAL_REFERENCES",
        "message": format!(
            "{candidate_count} text-only candidate hit(s) found; these are weaker than identifier syntax \
             and may be omitted by result limits or request options."
        ),
    })]
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
        let mut total_candidate: usize = 0;
        for ident in identifiers {
            let rows = collect_rows(&sym_infos, &all_contents, ident, &filter, true);
            let (primary, candidate) = split_identifier_and_text(rows);
            total_candidate += candidate.len();
            results.push(json!({
                "identifier": ident,
                "identifier_reference_count": primary.len(),
                "candidate_text_hit_count": candidate.len(),
                "identifier_references": primary,
                "candidate_text_hits": candidate,
            }));
        }
        let total_identifier: usize = results
            .iter()
            .filter_map(|r| r.get("identifier_reference_count")?.as_u64())
            .sum::<u64>() as usize;
        self.success_response_with_warnings(
            &MetaBuilder::new(0).with_confidence(Confidence::Partial),
            json!({
                "graph": "find_references",
                "total_identifier_references": total_identifier,
                "total_candidate_text_hits": total_candidate,
                "results": results,
            }),
            partial_references_warning(total_candidate),
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

        let rows = collect_rows(&sym_infos, &all_contents, identifier, &filter, false);
        let (mut primary, mut candidate) = split_identifier_and_text(rows);
        let total_identifier = primary.len();
        let total_candidate = candidate.len();
        // Tier 2.1: enforce the caller's global `limit` over the merged
        // (primary + candidate) result set. Definitions sort first inside
        // `primary` (see `collect_rows`), so we keep them.
        let limit = args.limit.unwrap_or(50).clamp(1, 500);
        let total_unbounded = total_identifier + total_candidate;
        let primary_truncated = if primary.len() > limit {
            primary.truncate(limit);
            true
        } else {
            false
        };
        let primary_remaining = limit.saturating_sub(primary.len());
        let candidate_truncated = if candidate.len() > primary_remaining {
            candidate.truncate(primary_remaining);
            true
        } else {
            false
        };
        let truncated = primary_truncated || candidate_truncated;
        let primary_count = primary.len();
        let candidate_count = candidate.len();

        // FIXUP T2.5: surface ambiguity at the top level. Callers should
        // not have to iterate the `identifier_references` list hunting for
        // `kind: "definition"` rows.
        let definitions: Vec<Value> = primary
            .iter()
            .filter(|row| row.get("kind").and_then(Value::as_str) == Some("definition"))
            .cloned()
            .collect();
        let definition_count = definitions.len();

        let _call_edges = if include_call_chain {
            Some(codesight_analysis::graph::build_call_edges(&sym_infos))
        } else {
            None
        };

        let mut data = json!({
            "graph": "find_references",
            "identifier": identifier,
            "identifier_reference_count": primary_count,
            "candidate_text_hit_count": candidate_count,
            "definition_count": definition_count,
            "definitions": definitions,
            "identifier_references": primary,
            "candidate_text_hits": candidate,
        });

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
                if mentions_identifier_ci_in_file(&sym.file, &sym.source, identifier) {
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
            data["call_chain"] = Value::Array(chain);
        }

        let mut warnings = partial_references_warning(total_candidate);
        warnings.push(json!({"code": "PARTIAL_REFERENCES",
            "message": "Identifier syntax and definitions are name-based candidates; aliases, shadowing and cross-file bindings are not resolved."}));
        if definition_count > 1 {
            warnings.push(json!({
                "code": "AMBIGUOUS_DEFINITION",
                "message": format!(
                    "{definition_count} definitions of '{identifier}' exist; pick one before editing."
                ),
                "detail": { "definition_count": definition_count },
            }));
        }
        if truncated {
            warnings.push(json!({
                "code": "TRUNCATED",
                "message": format!(
                    "results truncated to limit {limit} (primary + candidate); {total_unbounded} total references exist."
                ),
                "detail": {
                    "limit": limit,
                    "total_identifier_references": total_identifier,
                    "total_candidate_text_hits": total_candidate,
                },
            }));
        }
        self.success_response_with_warnings(
            &MetaBuilder::new(0).with_confidence(Confidence::Partial),
            data,
            warnings,
        )
    }
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
        let args = InspectGraphArgs::default();
        let f = RefFilter::from_args(&args);
        assert!(!f.admits("fixture.json"));
        assert!(f.admits("src/lib.rs"));
    }

    #[test]
    fn filter_scope_and_exclude() {
        let args = InspectGraphArgs {
            scope: Some("src/service/".into()),
            exclude: Some(vec!["tests/".into()]),
            ..InspectGraphArgs::default()
        };
        let f = RefFilter::from_args(&args);
        assert!(f.admits("src/service/foo.rs"));
        assert!(!f.admits("src/cli/foo.rs"));
        assert!(!f.admits("tests/unit.rs"));
    }

    #[test]
    fn filter_include_non_code_opts_in() {
        let args = InspectGraphArgs {
            include_non_code: Some(true),
            ..InspectGraphArgs::default()
        };
        let f = RefFilter::from_args(&args);
        assert!(f.admits("scripts/stress/x.json"));
    }

    #[test]
    fn filter_rejects_vendored_by_default() {
        let args = InspectGraphArgs {
            include_non_code: Some(true),
            ..InspectGraphArgs::default()
        };
        let f = RefFilter::from_args(&args);
        assert!(!f.admits("crates/x/tests/fixtures/sample.cs"));
        assert!(!f.admits("crates/codesight-retrieval/assets/embedder/tokenizer.json"));
        assert!(f.admits("src/lib.rs"));
    }

    #[test]
    fn filter_scope_into_excluded_segment_opts_in() {
        let args = InspectGraphArgs {
            scope: Some("crates/codesight-language-support/tests/fixtures/".into()),
            include_non_code: Some(true),
            ..InspectGraphArgs::default()
        };
        let f = RefFilter::from_args(&args);
        assert!(f.admits("crates/codesight-language-support/tests/fixtures/csharp/sample.cs"));
    }

    #[test]
    fn locate_identifier_ci_finds_word_boundary() {
        // Matches the whole word, not a substring.
        assert_eq!(locate_identifier_ci("let foo = 1;", "foo"), Some((1, 5)));
        assert_eq!(
            locate_identifier_ci("let foobar = 1;\nlet foo = 2;", "foo"),
            Some((2, 5))
        );
        assert!(locate_identifier_ci("let foobar = 1;", "foo").is_none());
    }

    #[test]
    fn locate_identifier_ci_is_case_insensitive() {
        assert_eq!(locate_identifier_ci("let Foo = 1;", "foo"), Some((1, 5)));
    }
}
