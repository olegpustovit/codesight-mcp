//! CUTPLAN adapter handlers — the 6-tool MCP surface.
//!
//! Each adapter parses the new arg shape, resolves an implicit source_id from
//! the cwd when omitted, translates to the underlying handler's argument
//! shape, calls the existing handler, then post-processes the response to
//! enforce the frozen envelope and the 8 reliability invariants.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::repo_id::normalize_repo_id;
use crate::service::CodeService;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::savings::MetaBuilder;

// ── Arg structs ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SearchArgs {
    #[serde(default)]
    pub per_file_limit: Option<usize>,
    pub query: String,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub fuzzy: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct OutlineArgs {
    pub view: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ReadArgs {
    pub refs: Vec<String>,
    #[serde(default)]
    pub context_lines: Option<i64>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub with_neighbors: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ReferencesArgs {
    pub kind: String,
    #[serde(default)]
    pub r#ref: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub include_weak: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ImpactArgs {
    pub analysis: String,
    pub r#ref: String,
    #[serde(default)]
    pub new_name: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    #[allow(dead_code)]
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct HealthArgs {
    pub analysis: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub top_n: Option<usize>,
    #[serde(default)]
    pub min_confidence: Option<f64>,
    #[serde(default)]
    pub include_tests: Option<bool>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

// ── Source resolution (P1.3) ────────────────────────────────────────────────

/// Resolve an implicit source_id from the current working directory.
/// Auto-indexes the cwd if it isn't already known. Returns the canonical
/// `source_id` ready to feed to underlying handlers.
fn resolve_implicit_source_id(svc: &CodeService) -> Result<(String, String), ToolError> {
    let cwd = std::env::current_dir().map_err(|e| {
        ToolError::new(
            ToolErrorCategory::Internal,
            "CWD_UNREADABLE",
            format!("could not read current working directory: {e}"),
            json!({}),
        )
    })?;
    let canonical = cwd.canonicalize().map_err(|e| {
        ToolError::new(
            ToolErrorCategory::Internal,
            "CWD_UNREADABLE",
            format!("could not canonicalize cwd: {e}"),
            json!({ "cwd": cwd.display().to_string() }),
        )
    })?;
    let source_id = normalize_repo_id(&canonical);
    let path_str = canonical.to_string_lossy().to_string();
    // Touch ensure_repo_store to lazily auto-index when missing. We discard
    // the store handle — the downstream handler will reopen via its own
    // `ensure_repo_store` call but find the index already populated.
    let _ = svc.ensure_repo_store(&source_id, Some(&path_str));
    Ok((source_id, path_str))
}

// ── Envelope normalization (P3) ─────────────────────────────────────────────

/// Strip non-frozen `_meta` keys. Keeps only `confidence`, `timing_ms`,
/// `tokens_saved`. Drops verbose echo fields, `actionable_next`, and any
/// `*_effective` re-narration.
fn freeze_envelope(value: &mut Value) {
    let Value::Object(body) = value else {
        return;
    };
    if let Some(Value::Object(meta)) = body.get_mut("_meta") {
        meta.retain(|k, _| {
            matches!(
                k.as_str(),
                "confidence" | "timing_ms" | "tokens_saved" | "error_category"
            )
        });
    }
    body.remove("session");
    body.remove("actionable_next");
}

// ── search adapter ──────────────────────────────────────────────────────────

impl CodeService {
    pub(in crate::service) fn cutplan_search(&self, v: Value) -> anyhow::Result<Value> {
        let args: SearchArgs = match serde_json::from_value(v) {
            Ok(a) => a,
            Err(e) => {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError::new(
                        ToolErrorCategory::InvalidArgument,
                        "INVALID_ARGUMENT",
                        format!("invalid search args: {e}"),
                        json!({}),
                    ),
                );
            }
        };

        let (sid, _) = match resolve_implicit_source_id(self) {
            Ok(p) => p,
            Err(e) => return self.error_response(&MetaBuilder::new(0), e),
        };

        let mode = args.mode.as_deref().unwrap_or("keyword");
        // P2.5: nl mode defaults to code-only. Markdown only enters via scope.
        let scope_is_markdown = args
            .scope
            .as_deref()
            .map(|s| s.contains(".md") || s.starts_with("docs/"))
            .unwrap_or(false);
        let (query_mode, domain, unit_kinds): (&str, &str, Option<Vec<&str>>) = match mode {
            "regex" => ("regex", "code", None),
            "nl" => (
                "natural_language",
                if scope_is_markdown { "docs" } else { "code" },
                None,
            ),
            _ => (
                "keyword",
                if scope_is_markdown { "docs" } else { "code" },
                if scope_is_markdown {
                    Some(vec!["section"])
                } else {
                    None
                },
            ),
        };

        // Translate scope glob → file_pattern when it contains '/' or '*',
        // else use as plain prefix scope.
        let mut translated = json!({
            "source_id": sid,
            "query": args.query,
            "query_mode": query_mode,
            "domain": domain,
            "limit": args.limit.unwrap_or(25),
            "fuzzy": args.fuzzy.unwrap_or(false),
        });
        if let Some(uk) = unit_kinds {
            translated["unit_kinds"] = json!(uk);
        }
        if let Some(scope) = args.scope.as_deref() {
            if scope.contains('*') || scope.contains('?') {
                translated["file_pattern"] = json!(scope);
            } else {
                translated["scope"] = json!(scope);
            }
        }
        if let Some(t) = args.max_tokens {
            translated["max_tokens"] = json!(t);
        }

        if let Some(cap) = args.per_file_limit {
            translated["per_file_limit"] = json!(cap);
        }
        let mut result = self.search_catalog(translated)?;
        freeze_envelope(&mut result);
        // P2.6: fuzzy hits already merge into items[] in the underlying
        // handler when `fuzzy=true`, but defensive: if a side-channel
        // `data.fuzzy` lane is present, fold it in and drop the lane.
        if let Value::Object(ref mut body) = result
            && let Some(Value::Object(data)) = body.get_mut("data")
            && let Some(fuzzy_lane) = data.remove("fuzzy")
            && let Some(Value::Array(items)) = data.get_mut("items")
            && let Some(arr) = fuzzy_lane.as_array()
        {
            for hit in arr {
                let mut tagged = hit.clone();
                if let Some(o) = tagged.as_object_mut() {
                    o.insert("match_type".to_string(), json!("fuzzy"));
                }
                items.push(tagged);
            }
        }
        Ok(result)
    }

    // ── outline adapter ────────────────────────────────────────────────────

    pub(in crate::service) fn cutplan_outline(&self, v: Value) -> anyhow::Result<Value> {
        let args: OutlineArgs = match serde_json::from_value(v) {
            Ok(a) => a,
            Err(e) => {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError::new(
                        ToolErrorCategory::InvalidArgument,
                        "INVALID_ARGUMENT",
                        format!("invalid outline args: {e}"),
                        json!({}),
                    ),
                );
            }
        };

        let (sid, _) = match resolve_implicit_source_id(self) {
            Ok(p) => p,
            Err(e) => return self.error_response(&MetaBuilder::new(0), e),
        };

        let view = match args.view.as_str() {
            "file" => "file_outline",
            "repo" => "repo_outline",
            other => {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError::new(
                        ToolErrorCategory::InvalidArgument,
                        "INVALID_ARGUMENT",
                        format!("outline view must be 'file' or 'repo', got '{other}'"),
                        json!({ "view": other }),
                    ),
                );
            }
        };

        let mut translated = json!({ "source_id": sid, "view": view });
        if let Some(p) = args.path {
            translated["file_path"] = json!(p);
        }
        if let Some(ps) = args.paths {
            translated["file_paths"] = json!(ps);
        }
        if let Some(t) = args.max_tokens {
            translated["max_tokens"] = json!(t);
        }

        let mut result = self.inspect_structure(translated)?;
        if args.view == "repo" {
            self.annotate_indexed_scope(&sid, &mut result);
        }
        freeze_envelope(&mut result);
        Ok(result)
    }

    // ── read adapter ───────────────────────────────────────────────────────

    pub(in crate::service) fn cutplan_read(&self, v: Value) -> anyhow::Result<Value> {
        let args: ReadArgs = match serde_json::from_value(v) {
            Ok(a) => a,
            Err(e) => {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError::new(
                        ToolErrorCategory::InvalidArgument,
                        "INVALID_ARGUMENT",
                        format!("invalid read args: {e}"),
                        json!({}),
                    ),
                );
            }
        };

        let (sid, _) = match resolve_implicit_source_id(self) {
            Ok(p) => p,
            Err(e) => return self.error_response(&MetaBuilder::new(0), e),
        };

        // Translate string refs into the legacy form. Support these patterns:
        //   "path:N-M", "path:N", "path::Name#kind", "id:<stable_id>",
        //   "section:path::heading"
        let mut object_refs: Vec<Value> = Vec::with_capacity(args.refs.len());
        for ref_str in &args.refs {
            object_refs.push(parse_ref_string(ref_str));
        }

        let mut translated = json!({
            "source_id": sid,
            "refs": object_refs,
            "verify": true,
        });
        if let Some(c) = args.context_lines {
            translated["context_lines"] = json!(c);
        }
        if let Some(t) = args.max_tokens {
            translated["max_tokens"] = json!(t);
        }

        let mut result = self.read_content(translated)?;

        // P2.7: with_neighbors=true — append structural neighbors via the
        // import + reference graph, no string-match. Implemented as a thin
        // sweep that calls inspect_graph for each symbol ref's host file.
        if args.with_neighbors.unwrap_or(false) {
            self.attach_structural_neighbors(&mut result, &args.refs);
        }

        freeze_envelope(&mut result);
        // Ensure whole-file read confidence semantics (P3 + plan §3 read
        // contract): downgrade overall to "partial" when a ref is a bare path
        // with no slice or symbol, so callers know to outline first.
        let any_full_file = args.refs.iter().any(|r| !r.contains(':'));
        if any_full_file && let Value::Object(ref mut body) = result {
            if let Some(Value::Object(meta)) = body.get_mut("_meta") {
                meta.insert("confidence".to_string(), json!("partial"));
            }
            if let Some(Value::Array(warnings)) = body.get_mut("warnings") {
                warnings.push(json!({
                    "code": "OUTLINE_FIRST",
                    "message": "whole-file read — call outline view=file path=… first then read specific symbols.",
                }));
            }
        }
        Ok(result)
    }

    /// Tier 3.2: append structural neighbors (import + reference graph) for
    /// `read` requests with `with_neighbors=true`. Dispatches by ref form so
    /// symbol and range refs no longer silently no-op:
    /// - `path::Name#kind` / `id:<sid>` → file imports + symbol references
    /// - `path:N-M` / `path:N` / `path` → file imports
    /// - `section:…` → no neighbors (documented in `tools.json`)
    fn attach_structural_neighbors(&self, response: &mut Value, refs: &[String]) {
        let Value::Object(body) = response else {
            return;
        };
        let Some(Value::Object(data)) = body.get_mut("data") else {
            return;
        };

        let mut neighbors: Vec<Value> = Vec::new();
        let mut seen_paths: rustc_hash::FxHashSet<String> = rustc_hash::FxHashSet::default();
        let mut seen_symbols: rustc_hash::FxHashSet<String> = rustc_hash::FxHashSet::default();

        let (sid, _) = match resolve_implicit_source_id(self) {
            Ok(p) => p,
            Err(_) => return,
        };

        for ref_str in refs {
            let parsed = parse_ref_string(ref_str);
            let kind = parsed.get("kind").and_then(Value::as_str).unwrap_or("");

            // Determine host file path for imports lookup.
            let host_path = parsed
                .get("path")
                .and_then(Value::as_str)
                .map(|s| s.to_string())
                .or_else(|| {
                    parsed
                        .get("stable_id")
                        .and_then(Value::as_str)
                        .and_then(|s| s.split("::").next())
                        .map(|s| s.to_string())
                })
                .filter(|p| !p.is_empty());

            // For symbol refs, attach reference-graph neighbors keyed by the
            // bare identifier (last segment of the qualified name).
            if kind == "symbol"
                && let Some(ident) = extract_identifier_from_ref(ref_str)
                && seen_symbols.insert(ident.clone())
                && let Ok(refs_resp) = self.inspect_graph(json!({
                    "source_id": sid,
                    "graph": "find_references",
                    "identifier": ident,
                    "limit": 5,
                }))
                && let Some(items) = refs_resp
                    .pointer("/data/identifier_references")
                    .and_then(Value::as_array)
            {
                for item in items.iter().take(5) {
                    neighbors.push(json!({
                        "kind": "reference",
                        "to_symbol": ident,
                        "from": item,
                    }));
                }
            }

            // Outgoing imports for the host file (skip section refs — there
            // is no host file at the symbol layer for doc sections).
            if kind == "section" {
                continue;
            }
            if let Some(path) = host_path
                && seen_paths.insert(path.clone())
                && let Ok(imp_resp) = self.inspect_graph(json!({
                    "source_id": sid,
                    "graph": "imports",
                    "file_path": path,
                }))
                && let Some(items) = imp_resp.pointer("/data/edges").and_then(Value::as_array)
            {
                for item in items.iter().take(10) {
                    neighbors.push(json!({
                        "kind": "import",
                        "from": path,
                        "to": item,
                    }));
                }
            }
        }

        if !neighbors.is_empty() {
            data.insert("structural_neighbors".to_string(), Value::Array(neighbors));
        }
    }

    // ── references adapter ─────────────────────────────────────────────────

    pub(in crate::service) fn cutplan_references(&self, v: Value) -> anyhow::Result<Value> {
        let args: ReferencesArgs = match serde_json::from_value(v) {
            Ok(a) => a,
            Err(e) => {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError::new(
                        ToolErrorCategory::InvalidArgument,
                        "INVALID_ARGUMENT",
                        format!("invalid references args: {e}"),
                        json!({}),
                    ),
                );
            }
        };

        let (sid, _) = match resolve_implicit_source_id(self) {
            Ok(p) => p,
            Err(e) => return self.error_response(&MetaBuilder::new(0), e),
        };

        let translated = match args.kind.as_str() {
            "symbol" => {
                let Some(ref_str) = args.r#ref.as_deref() else {
                    return self.error_response(
                        &MetaBuilder::new(0),
                        ToolError::new(
                            ToolErrorCategory::InvalidArgument,
                            "INVALID_ARGUMENT",
                            "kind=symbol requires `ref` (string)",
                            json!({}),
                        ),
                    );
                };
                // Extract identifier name from ref string (path::Name#kind or id:<stable>).
                let identifier =
                    extract_identifier_from_ref(ref_str).unwrap_or_else(|| ref_str.to_string());
                let mut t = json!({
                    "source_id": sid,
                    "graph": "find_references",
                    "identifier": identifier,
                });
                if let Some(s) = args.scope.as_deref() {
                    t["scope"] = json!(s);
                }
                if let Some(l) = args.limit {
                    t["limit"] = json!(l);
                }
                t
            }
            "imports" | "importers" => {
                let Some(ref path) = args.path else {
                    return self.error_response(
                        &MetaBuilder::new(0),
                        ToolError::new(
                            ToolErrorCategory::InvalidArgument,
                            "INVALID_ARGUMENT",
                            format!("kind={} requires `path`", args.kind),
                            json!({}),
                        ),
                    );
                };
                json!({
                    "source_id": sid,
                    "graph": args.kind,
                    "file_path": path,
                    "limit": args.limit.unwrap_or(50),
                    "scope": args.scope,
                })
            }
            other => {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError::new(
                        ToolErrorCategory::InvalidArgument,
                        "INVALID_ARGUMENT",
                        format!("kind must be 'symbol', 'imports', or 'importers', got '{other}'"),
                        json!({ "kind": other }),
                    ),
                );
            }
        };

        let mut result = self.inspect_graph(translated)?;

        // P2.1: rewrite `name` → `identifier` on symbol-reference rows. Move
        // candidate text hits into `weak_hits[]` (opt-in via include_weak).
        if args.kind == "symbol" {
            reshape_references_symbol(
                &mut result,
                args.r#ref.as_deref(),
                args.include_weak.unwrap_or(false),
            );
        }

        freeze_envelope(&mut result);
        Ok(result)
    }

    // ── impact adapter ─────────────────────────────────────────────────────

    pub(in crate::service) fn cutplan_impact(&self, v: Value) -> anyhow::Result<Value> {
        let args: ImpactArgs = match serde_json::from_value(v) {
            Ok(a) => a,
            Err(e) => {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError::new(
                        ToolErrorCategory::InvalidArgument,
                        "INVALID_ARGUMENT",
                        format!("invalid impact args: {e}"),
                        json!({}),
                    ),
                );
            }
        };

        let (sid, _) = match resolve_implicit_source_id(self) {
            Ok(p) => p,
            Err(e) => return self.error_response(&MetaBuilder::new(0), e),
        };

        match args.analysis.as_str() {
            // P2.2: blast_radius is symbol-scoped — bypass the legacy
            // file-importer path entirely. Walk symbol references, dedupe
            // by file. Refuse with `unreliable` when the references graph
            // can't produce identifier-syntax candidates.
            "blast_radius" => self.cutplan_impact_blast_radius(&sid, &args),
            "rename_safety" => self.cutplan_impact_rename_safety(&sid, &args),
            other => self.error_response(
                &MetaBuilder::new(0),
                ToolError::new(
                    ToolErrorCategory::InvalidArgument,
                    "INVALID_ARGUMENT",
                    format!("analysis must be 'blast_radius' or 'rename_safety', got '{other}'"),
                    json!({ "analysis": other }),
                ),
            ),
        }
    }

    /// Strict-ref check for `impact`. A structured ref (`path::Name#kind` or
    /// `id:<stable_id>`) names one specific symbol, so it must resolve to an
    /// indexed symbol before we walk references by bare name. Otherwise a
    /// typo'd or deleted path would silently fall back to a name-only search
    /// and could return `confidence: safe` for a symbol that does not exist.
    /// Bare identifiers (no `::`) are unaffected.
    ///
    /// Returns `Some(NOT_FOUND)` when the ref cannot be resolved.
    fn unresolved_structured_ref(
        &self,
        sid: &str,
        ref_str: &str,
        identifier: &str,
    ) -> Option<ToolError> {
        let trimmed = ref_str.trim();
        let stable_id = trimmed.strip_prefix("id:").unwrap_or(trimmed);
        let (path, _) = stable_id.split_once("::")?;
        let not_found = |reason: &str| {
            ToolError::new(
                ToolErrorCategory::NotFound,
                "NOT_FOUND",
                format!("ref '{ref_str}' does not resolve to an indexed symbol: {reason}"),
                json!({ "ref": ref_str, "path": path, "identifier": identifier }),
            )
        };
        let store = match self.open_repo_store(sid) {
            Ok(store) => store,
            Err(e) => return Some(e),
        };
        if matches!(store.symbol_record(stable_id), Ok(Some(_))) {
            return None;
        }
        match store.file_exists(path) {
            Ok(true) => {}
            Ok(false) => return Some(not_found("file is not in the index")),
            Err(e) => return Some(not_found(&format!("index lookup failed: {e}"))),
        }
        match store.symbol_id_at_name(path, identifier) {
            Ok(Some(_)) => None,
            Ok(None) => Some(not_found("no symbol with that name in the file")),
            Err(e) => Some(not_found(&format!("index lookup failed: {e}"))),
        }
    }

    fn cutplan_impact_blast_radius(&self, sid: &str, args: &ImpactArgs) -> anyhow::Result<Value> {
        let identifier = match extract_identifier_from_ref(&args.r#ref) {
            Some(id) => id,
            None => {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError::new(
                        ToolErrorCategory::InvalidArgument,
                        "INVALID_ARGUMENT",
                        format!(
                            "blast_radius: could not parse identifier from ref '{}'",
                            args.r#ref
                        ),
                        json!({ "ref": args.r#ref }),
                    ),
                );
            }
        };

        if let Some(err) = self.unresolved_structured_ref(sid, &args.r#ref, &identifier) {
            return self.error_response(&MetaBuilder::new(0), err);
        }

        // Collect name-based identifier candidates and deduplicate by file.
        let refs_response = self.inspect_graph(json!({
            "source_id": sid,
            "graph": "find_references",
            "identifier": identifier,
        }))?;

        let mut files: rustc_hash::FxHashSet<String> = rustc_hash::FxHashSet::default();
        let mut weak_files: rustc_hash::FxHashSet<String> = rustc_hash::FxHashSet::default();
        if let Some(arr) = refs_response
            .pointer("/data/identifier_references")
            .and_then(Value::as_array)
        {
            for row in arr {
                if let Some(file) = row.get("file").and_then(Value::as_str) {
                    files.insert(file.to_string());
                }
            }
        }
        if let Some(arr) = refs_response
            .pointer("/data/candidate_text_hits")
            .and_then(Value::as_array)
        {
            for row in arr {
                if let Some(file) = row.get("file").and_then(Value::as_str) {
                    weak_files.insert(file.to_string());
                }
            }
        }

        // Classify the evidence before limiting it; an empty name search
        // cannot establish that a change has no impact.
        let confidence = if files.is_empty() && !weak_files.is_empty() {
            "unreliable"
        } else {
            "partial"
        };
        let mut sorted: Vec<String> = files.into_iter().collect();
        sorted.sort();
        if let Some(limit) = args.limit {
            sorted.truncate(limit);
        }
        let count = sorted.len();
        let mut weak_sorted: Vec<String> = weak_files.into_iter().collect();
        weak_sorted.sort();

        let mut body = json!({
            "analysis": "blast_radius",
            "status": "incomplete",
            "complete": false,
            "identifier": identifier,
            "symbol_scoped_files": sorted,
            "symbol_scoped_file_count": count,
        });
        if !weak_sorted.is_empty() {
            body["weak_files_text_mentions"] = json!(weak_sorted);
        }
        let mut response = self.success_response_with_meta_extra(
            &MetaBuilder::new(0),
            body,
            refs_response["warnings"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
            [("confidence".to_string(), json!(confidence))],
        )?;
        freeze_envelope(&mut response);
        Ok(response)
    }

    fn cutplan_impact_rename_safety(&self, sid: &str, args: &ImpactArgs) -> anyhow::Result<Value> {
        let new_name = match args.new_name.as_deref() {
            Some(n) if !n.is_empty() => n,
            _ => {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError::new(
                        ToolErrorCategory::InvalidArgument,
                        "INVALID_ARGUMENT",
                        "rename_safety requires `new_name`",
                        json!({}),
                    ),
                );
            }
        };
        let identifier = match extract_identifier_from_ref(&args.r#ref) {
            Some(id) => id,
            None => {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError::new(
                        ToolErrorCategory::InvalidArgument,
                        "INVALID_ARGUMENT",
                        format!(
                            "rename_safety: could not parse identifier from ref '{}'",
                            args.r#ref
                        ),
                        json!({ "ref": args.r#ref }),
                    ),
                );
            }
        };

        if let Some(err) = self.unresolved_structured_ref(sid, &args.r#ref, &identifier) {
            return self.error_response(&MetaBuilder::new(0), err);
        }

        // Walk the references graph for both the old and new names.
        let old_refs = self.inspect_graph(json!({
            "source_id": sid,
            "graph": "find_references",
            "identifier": identifier.clone(),
        }))?;
        let new_refs = self.inspect_graph(json!({
            "source_id": sid,
            "graph": "find_references",
            "identifier": new_name,
        }))?;

        let identifier_count = old_refs
            .pointer("/data/identifier_reference_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let candidate_count = old_refs
            .pointer("/data/candidate_text_hit_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let collisions = new_refs
            .pointer("/data/identifier_reference_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);

        // Syntax coverage is not binding completeness or rename safety.
        let total = identifier_count + candidate_count;
        let identifier_match_pct = if total == 0 {
            0.0
        } else {
            (identifier_count as f64 / total as f64 * 10000.0).round() / 100.0
        };
        let body = json!({
            "analysis": "rename_safety",
            "identifier": identifier,
            "new_name": new_name,
            "safe": null,
            "status": "undecided",
            "complete": false,
            "identifier_match_pct": identifier_match_pct,
            "scanned_identifier_references": identifier_count,
            "candidate_text_hits": candidate_count,
            "collision_count_with_new_name": collisions,
        });
        let mut response = self.success_response_with_meta_extra(
            &MetaBuilder::new(0),
            body,
            vec![json!({"code": "PARTIAL_REFERENCES", "message": "Name-based syntax candidates cannot establish complete bindings or rename safety; collision counts are also name-based."})],
            [("confidence".to_string(), json!("partial"))],
        )?;
        freeze_envelope(&mut response);
        Ok(response)
    }

    // ── health adapter ─────────────────────────────────────────────────────

    pub(in crate::service) fn cutplan_health(&self, v: Value) -> anyhow::Result<Value> {
        let args: HealthArgs = match serde_json::from_value(v) {
            Ok(a) => a,
            Err(e) => {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError::new(
                        ToolErrorCategory::InvalidArgument,
                        "INVALID_ARGUMENT",
                        format!("invalid health args: {e}"),
                        json!({}),
                    ),
                );
            }
        };

        let (sid, _) = match resolve_implicit_source_id(self) {
            Ok(p) => p,
            Err(e) => return self.error_response(&MetaBuilder::new(0), e),
        };

        let analysis = match args.analysis.as_str() {
            "quality_signal" | "hotspots" | "dead_code" | "repo_health" | "test_gaps" => {
                &args.analysis
            }
            other => {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError::new(
                        ToolErrorCategory::InvalidArgument,
                        "INVALID_ARGUMENT",
                        format!("analysis must be 'quality_signal', 'hotspots', 'dead_code', 'repo_health', or 'test_gaps', got '{other}'"),
                        json!({ "analysis": other }),
                    ),
                );
            }
        };

        let mut translated = json!({ "source_id": sid, "analysis": analysis });
        if let Some(s) = args.scope {
            translated["scope"] = json!(s);
        }
        if let Some(t) = args.top_n {
            translated["top_n"] = json!(t);
        }
        if let Some(c) = args.min_confidence {
            translated["min_confidence"] = json!(c);
        }
        if let Some(t) = args.include_tests {
            translated["include_tests"] = json!(t);
        }
        if let Some(t) = args.max_tokens {
            translated["max_tokens"] = json!(t);
        }

        let mut result = self.evaluate_quality(translated)?;
        self.annotate_indexed_scope(&sid, &mut result);
        freeze_envelope(&mut result);
        Ok(result)
    }
}

// ── Ref string parsing ──────────────────────────────────────────────────────

/// Parse a string ref into a content-ref object. Recognized forms:
///   "path:N-M"            → { kind: "file", path, start_line: N, end_line: M }
///   "path:N"              → { kind: "file", path, start_line: N, end_line: N }
///   "path::Name#kind"     → { kind: "symbol", path, stable_id: "path::Name#kind" }
///   "id:<stable_id>"      → { kind: "symbol", stable_id }
///   "section:path::head"  → { kind: "section", stable_id: "section:path::head" }
fn parse_ref_string(s: &str) -> Value {
    let trimmed = s.trim();
    if let Some(stable_id) = trimmed.strip_prefix("id:") {
        return json!({ "kind": "symbol", "stable_id": stable_id });
    }
    if let Some(rest) = trimmed.strip_prefix("section:") {
        return json!({ "kind": "section", "stable_id": rest });
    }
    if let Some((path, _name_kind)) = trimmed.split_once("::")
        && !path.is_empty()
    {
        return json!({ "kind": "symbol", "path": path, "stable_id": trimmed });
    }
    if let Some((path, range)) = trimmed.rsplit_once(':')
        && !path.is_empty()
    {
        let (start, end) = if let Some((a, b)) = range.split_once('-') {
            (a.parse::<u32>().ok(), b.parse::<u32>().ok())
        } else {
            let n = range.parse::<u32>().ok();
            (n, n)
        };
        if let (Some(s_line), Some(e_line)) = (start, end) {
            return json!({
                "kind": "file",
                "path": path,
                "start_line": s_line,
                "end_line": e_line,
            });
        }
    }
    // Bare path — whole file read. The adapter will downgrade confidence.
    json!({ "kind": "file", "path": trimmed })
}

/// Pull the bare identifier (symbol name) out of a ref string for the
/// `find_references` legacy handler. `path::Name#kind` → "Name".
fn extract_identifier_from_ref(s: &str) -> Option<String> {
    let trimmed = s.trim();
    let after = if let Some(rest) = trimmed.strip_prefix("id:") {
        rest
    } else {
        trimmed
    };
    let after_path = match after.split_once("::") {
        Some((_, rest)) => rest,
        None => after,
    };
    let bare = match after_path.split_once('#') {
        Some((name, _kind)) => name,
        None => after_path,
    };
    if bare.is_empty() {
        None
    } else {
        // Last segment of the qualified name (e.g. `Foo.bar` → "bar").
        Some(bare.rsplit(['.', ':']).next().unwrap_or(bare).to_string())
    }
}

// ── Reliability fix post-processors ─────────────────────────────────────────

/// P2.1: rewrite the find_references payload into the new
/// `references kind=symbol` shape:
///   - `name` field becomes `identifier` (= searched name)
///   - `enclosing_symbol`/`enclosing_kind` already populated by the legacy
///     handler — kept as-is.
///   - `body_match_kind` already populated for symbol_body_match rows.
///   - candidate_text_hits move to `weak_hits[]`. They appear only when
///     `include_weak=true`; otherwise dropped.
fn reshape_references_symbol(value: &mut Value, ref_str: Option<&str>, include_weak: bool) {
    let identifier = ref_str
        .and_then(extract_identifier_from_ref)
        .unwrap_or_default();

    let Value::Object(body) = value else {
        return;
    };
    let Some(Value::Object(data)) = body.get_mut("data") else {
        return;
    };

    // Rewrite identifier_references rows: name → identifier (= searched name).
    if let Some(Value::Array(items)) = data.get_mut("identifier_references") {
        for item in items.iter_mut() {
            if let Some(o) = item.as_object_mut() {
                o.insert("identifier".to_string(), json!(identifier));
                o.remove("name");
            }
        }
        let items = data.remove("identifier_references").unwrap();
        data.insert("items".to_string(), items);
    }

    // Move candidate_text_hits to weak_hits[] (opt-in only).
    if let Some(weak) = data.remove("candidate_text_hits")
        && include_weak
    {
        // Tag each weak hit with identifier too.
        let mut tagged = weak;
        if let Value::Array(arr) = &mut tagged {
            for item in arr.iter_mut() {
                if let Some(o) = item.as_object_mut() {
                    o.insert("identifier".to_string(), json!(identifier));
                    o.remove("name");
                }
            }
        }
        data.insert("weak_hits".to_string(), tagged);
    }
    data.remove("definitions");
    data.remove("definition_count");
    data.remove("graph");
    data.remove("identifier_reference_count");
    data.remove("candidate_text_hit_count");
}

// P2.2/P2.3 implementations live inline in `cutplan_impact_blast_radius` and
// `cutplan_impact_rename_safety` above — the legacy `evaluate_impact` handler
// is bypassed for those analyses.
