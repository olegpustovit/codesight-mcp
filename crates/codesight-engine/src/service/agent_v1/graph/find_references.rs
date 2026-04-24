use std::collections::HashSet;

use codesight_language_support::is_non_code_reference_path;
use codesight_language_support::mentions_identifier_ci_in_file;
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

/// Return true when `c` is part of a Rust-shaped identifier token. Used for
/// word-boundary checks on case-insensitive substring searches.
fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Locate the first case-insensitive, word-boundary-delimited occurrence of
/// `identifier` in `source`. Returns `(line, column)` with 1-based indexing.
fn locate_identifier_ci(source: &str, identifier: &str) -> Option<(u32, u32)> {
    if identifier.is_empty() {
        return None;
    }
    let id_lower = identifier.to_ascii_lowercase();
    let id_len = id_lower.len();
    for (line_idx, line) in source.lines().enumerate() {
        let bytes = line.as_bytes();
        let line_lower = line.to_ascii_lowercase();
        let lower_bytes = line_lower.as_bytes();
        let mut search_from = 0usize;
        while let Some(pos) = line_lower[search_from..].find(&id_lower) {
            let abs = search_from + pos;
            let before_ok = abs == 0 || !is_ident_char(bytes[abs - 1]);
            let end = abs + id_len;
            let after_ok = end >= lower_bytes.len() || !is_ident_char(bytes[end]);
            if before_ok && after_ok {
                return Some((line_idx as u32 + 1, abs as u32 + 1));
            }
            search_from = abs + id_len;
        }
    }
    None
}

fn first_identifier_line(file: &str, content: &str, identifier: &str) -> Option<(u32, String)> {
    content.lines().enumerate().find_map(|(idx, line)| {
        mentions_identifier_ci_in_file(file, line, identifier)
            .then(|| (idx as u32 + 1, line.trim().to_string()))
    })
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
        if mentions_identifier_ci_in_file(&sym.file, &sym.source, name) {
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
        if let Some((line, preview)) = first_identifier_line(file, content, name) {
            out.push(RefMatch {
                file: file.clone(),
                line,
                kind: "file_body_match",
                preview,
            });
        }
    }
    Ok(out)
}

/// Fully-populated reference row. `definition` and `symbol_body_match` are
/// semantic (definition site or a match inside a symbol body). `file_body_match`
/// is a candidate text hit — the identifier appears somewhere in file text that
/// isn't covered by an indexed symbol span.
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

    fn is_semantic(&self) -> bool {
        !matches!(self.kind, "file_body_match")
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
    let mut seen_files: HashSet<String> = HashSet::new();

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
            });
            seen_files.insert(sym.file.clone());
            continue;
        }
        if mentions_identifier_ci_in_file(&sym.file, &sym.source, identifier) {
            let (line, column) = locate_identifier_ci(&sym.source, identifier)
                .map(|(rel_line, col)| (sym.line + rel_line - 1, Some(col)))
                .unwrap_or((sym.line, None));
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
            });
            seen_files.insert(sym.file.clone());
        }
    }

    for (file, content) in all_contents {
        if seen_files.contains(file) || !filter.admits(file) {
            continue;
        }
        let Some((line, col)) = locate_identifier_ci(content, identifier) else {
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
        });
        seen_files.insert(file.clone());
    }

    rows
}

fn split_semantic_and_candidate(rows: Vec<ReferenceRow>) -> (Vec<Value>, Vec<Value>) {
    let mut semantic = Vec::new();
    let mut candidate = Vec::new();
    for row in rows {
        let json = row.to_json();
        if row.is_semantic() {
            semantic.push(json);
        } else {
            candidate.push(json);
        }
    }
    (semantic, candidate)
}

fn partial_references_warning(candidate_count: usize) -> Vec<Value> {
    if candidate_count == 0 {
        return Vec::new();
    }
    vec![json!({
        "code": "PARTIAL_REFERENCES",
        "message": format!(
            "{candidate_count} candidate text hit(s) returned alongside semantic references. \
             Candidate hits live in `candidate_text_hits` — they are substring matches in file text \
             not covered by any indexed symbol span, and are weaker than semantic references."
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
            let (semantic, candidate) = split_semantic_and_candidate(rows);
            total_candidate += candidate.len();
            results.push(json!({
                "identifier": ident,
                "semantic_reference_count": semantic.len(),
                "candidate_text_hit_count": candidate.len(),
                "semantic_references": semantic,
                "candidate_text_hits": candidate,
            }));
        }
        let total_semantic: usize = results
            .iter()
            .filter_map(|r| r.get("semantic_reference_count")?.as_u64())
            .sum::<u64>() as usize;
        self.success_response_with_warnings(
            &MetaBuilder::new(0),
            json!({
                "graph": "find_references",
                "total_semantic_references": total_semantic,
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
        let (semantic, candidate) = split_semantic_and_candidate(rows);
        let semantic_count = semantic.len();
        let candidate_count = candidate.len();

        let _call_edges = if include_call_chain {
            Some(codesight_analysis::graph::build_call_edges(&sym_infos))
        } else {
            None
        };

        let mut data = json!({
            "graph": "find_references",
            "identifier": identifier,
            "semantic_reference_count": semantic_count,
            "candidate_text_hit_count": candidate_count,
            "semantic_references": semantic,
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

        self.success_response_with_warnings(
            &MetaBuilder::new(0),
            data,
            partial_references_warning(candidate_count),
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
