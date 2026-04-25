//! Impact payload builders.
//!
//! Two user-facing surfaces live here:
//!
//! - [`rename_safety_payload`] — symbol-level rename conflict check. Walks the
//!   defining file's outline plus importers for shape collisions.
//! - [`preview_path_rename`] — module-path prefix rewrite. Per-language logic
//!   is owned by `Language::path_rename_engine` in the language-support crate;
//!   this is a thin dispatch that also surfaces a consistent "unsupported
//!   language" error envelope for callers.
//!
//! The `PathRename*` shared types are re-exported from
//! [`codesight_language_support::path_rename`] so tool handlers can import
//! them from one place.

use rustc_hash::{FxHashMap, FxHashSet};

use serde_json::{Value, json};

pub use codesight_language_support::path_rename::{
    PathRenameChange, PathRenameCollision, PathRenameFilePlan, PathRenamePlan, PathRenameUnhandled,
    affected_paths, summarize as summarize_path_rename,
};
use codesight_language_support::is_vendored_or_fixture_path;
use codesight_language_support::language_id_for_path;
use codesight_language_support::manifest::UNKNOWN_LANGUAGE;
use codesight_language_support::text_match::{classify_body_match, locate_identifier_ci};
use codesight_storage::storage::IndexStore;

// -----------------------------------------------------------------------------
// rename_safety — symbol-level rename conflict check
// -----------------------------------------------------------------------------

pub fn rename_safety_payload(
    store: &IndexStore,
    symbol_id: &str,
    new_name: &str,
    include_vendored: bool,
) -> anyhow::Result<Option<Value>> {
    let Some(record) = store.symbol_record(symbol_id)? else {
        return Ok(None);
    };

    let symbol_name = record
        .id
        .rsplit("::")
        .next()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("")
        .to_string();

    let file_path = &record.file_path;
    let outline = store.file_outline(file_path)?;
    let mut conflicts = Vec::new();

    for symbol in &outline {
        if symbol["name"].as_str() == Some(new_name) && symbol["id"].as_str() != Some(symbol_id) {
            conflicts.push(json!({
                "existing_name": new_name,
                "file": file_path,
                "kind": symbol["kind"],
                "id": symbol["id"],
            }));
        }
    }

    let adjacency = store.dependency_graph()?;
    let mut checked_files: FxHashSet<String> = std::iter::once(file_path.clone()).collect();
    let mut reverse_adj = FxHashMap::<String, Vec<String>>::default();
    for (source, targets) in &adjacency {
        for target in targets {
            reverse_adj
                .entry(target.clone())
                .or_default()
                .push(source.clone());
        }
    }

    if let Some(importers) = reverse_adj.get(file_path) {
        // Batch-fetch all importer outlines in one query instead of N.
        let importer_refs: Vec<&str> = importers.iter().map(String::as_str).collect();
        let outlines = store.file_outlines_batch(&importer_refs)?;
        for importer in importers {
            checked_files.insert(importer.clone());
            if let Some(outline) = outlines.get(importer.as_str()) {
                for symbol in outline {
                    if symbol["name"].as_str() == Some(new_name) {
                        conflicts.push(json!({
                            "existing_name": new_name,
                            "file": importer,
                            "kind": symbol["kind"],
                            "id": symbol["id"],
                        }));
                    }
                }
            }
        }
    }

    // P0.3 — trait dispatch awareness. For Rust symbols whose defining file
    // declares the enclosing `trait <Owner> { … }`, a rename is only safe
    // when every `impl <Owner> for X { fn <name> … }` site is rewritten
    // alongside. Enumerate those sites so agents can't silently ship a
    // half-renamed trait.
    //
    // Qualified names like `Struct.method` carry the receiver — strip it so
    // the heuristic searches for the bare Rust identifier `method` in source.
    let symbol_leaf = symbol_name
        .rsplit('.')
        .next()
        .unwrap_or(symbol_name.as_str());
    let (trait_owner, required_edits) =
        detect_trait_impl_sites(store, file_path, symbol_leaf)?;
    let required_edits_rendered: Vec<Value> = required_edits
        .iter()
        .map(|file| {
            json!({
                "file": file,
                "reason": "trait_impl_method_must_be_renamed",
                "detail": "file contains `impl <Trait> for X` that must rename this method in lockstep",
            })
        })
        .collect();
    let trait_method_detected = trait_owner.is_some();
    let impl_count = required_edits.len();

    // FIXUP T1.1: rename safety must also know about references living
    // outside the defining site. A pure collision check can say "safe"
    // while callers across the repo still point at the old name.
    //
    // The incoming `symbol_name` may be `Struct.method` — when present, the
    // receiver prefix tells us the rename is a method-rename so we can
    // require a receiver shape (`ledger.record(` / `Type::record(`) at the
    // call site rather than a bare identifier.
    let is_method_like = matches!(
        record.kind,
        codesight_core::SymbolKind::Method
    ) || symbol_name.contains('.');
    let scan = scan_symbol_references(
        store,
        symbol_id,
        symbol_leaf,
        file_path,
        is_method_like,
        include_vendored,
        REFERENCE_SCAN_LIMIT,
    )?;
    let unrewritten_references: Vec<Value> = scan
        .references
        .iter()
        .map(|row| {
            json!({
                "file": row.file,
                "line": row.line,
                "kind": row.kind,
                "enclosing_symbol": row.enclosing_symbol,
                "body_match_kind": row.body_match_kind,
            })
        })
        .collect();

    let collision_free = conflicts.is_empty();
    let safe = collision_free
        && !trait_method_detected
        && unrewritten_references.is_empty();

    let mut payload = json!({
        "analysis": "rename_safety",
        "safe": safe,
        "collision_free": collision_free,
        "conflicts": conflicts,
        "checked_files": checked_files.len(),
        "symbol": {
            "name": symbol_name,
            "id": symbol_id,
            "file": file_path,
            "is_method_like": is_method_like,
        },
        "new_name": new_name,
        "unrewritten_references": unrewritten_references,
        "reference_scan": {
            "method": "identifier_bounded_with_body_classifier",
            "complete": scan.complete,
            "scanned_files": scan.scanned_files,
            "excluded_vendored_or_fixture": scan.excluded_vendored_or_fixture,
            "rejected_comment_or_string": scan.rejected_comment_or_string,
            "rejected_missing_receiver": scan.rejected_missing_receiver,
            "partial_language_coverage": scan.partial_language_coverage,
            "confidence": scan.confidence,
        },
        "trait_dispatch": {
            "detected_trait_owner": trait_owner,
            "required_edits": required_edits_rendered,
            "impl_site_count": impl_count,
        },
    });

    let mut warnings: Vec<Value> = Vec::new();
    if trait_method_detected {
        warnings.push(json!({
            "code": "TRAIT_METHOD_RENAME",
            "message": format!(
                "'{symbol_name}' appears to be a trait method on '{}'. Every `impl <Trait> for X` site in `required_edits` must rename this method in the same patch.",
                trait_owner.as_deref().unwrap_or("(unknown trait)"),
            ),
        }));
    }
    if scan.partial_language_coverage {
        warnings.push(json!({
            "code": "PARTIAL_LANGUAGE_COVERAGE",
            "message": "reference scan hit symbol bodies in languages without registered identifier heuristics — those hits were downgraded to `partial_language_match` rather than reported as unrewritten references.",
        }));
    }
    if !warnings.is_empty() {
        payload["warnings"] = json!(warnings);
    }

    Ok(Some(payload))
}

// -----------------------------------------------------------------------------
// FIXUP T1.1 — reference-scan helper for rename_safety
// -----------------------------------------------------------------------------

/// Soft cap on how many indexed symbols we scan for references. The scan
/// is a linear substring check, so bound it to keep the handler under the
/// 200ms cold-dispatch budget on large repos.
const REFERENCE_SCAN_LIMIT: usize = 4000;

struct ReferenceScanRow {
    file: String,
    line: u32,
    kind: &'static str,
    enclosing_symbol: Option<String>,
    body_match_kind: &'static str,
}

struct ReferenceScan {
    references: Vec<ReferenceScanRow>,
    complete: bool,
    scanned_files: usize,
    /// Hits silently dropped because the enclosing symbol's file lived in
    /// `/fixtures/`, `/node_modules/`, `/target/`, etc.
    excluded_vendored_or_fixture: usize,
    /// Hits that landed inside a line `classify_body_match` tagged as
    /// `comment_or_string`.
    rejected_comment_or_string: usize,
    /// Hits for method-like symbols that lacked a receiver prefix
    /// (`obj.name(` or `Type::name(`) — treated as unrelated identifiers.
    rejected_missing_receiver: usize,
    /// True when at least one hit came from a file whose language isn't
    /// registered. Those hits are downgraded (kind=`partial_language_match`)
    /// and surfaced via `PARTIAL_LANGUAGE_COVERAGE`.
    partial_language_coverage: bool,
    /// Overall confidence tier: `high` | `partial` | `none_after_filters`.
    confidence: &'static str,
}

/// Scan indexed symbols for references to `symbol_name` outside the
/// defining file. Returns typed rows — the MCP handler shapes the JSON.
/// Uses `store.all_symbols_with_source()` so we don't re-read files from
/// disk. Filters:
///
/// 1. Vendored / fixture / generated paths are excluded up-front via the
///    shared [`is_vendored_or_fixture_path`] predicate. These paths produce
///    no rename obligations — renaming source never requires editing them.
/// 2. Each word-bounded identifier hit is re-classified with
///    [`classify_body_match`]. Hits in comments / string literals are
///    dropped — they don't reflect a real reference to the symbol.
/// 3. When `require_receiver` is true (method rename / `Struct.method`
///    shape), the hit must be preceded by `.` or `::` so
///    `ledger.record(...)` or `Ledger::record(...)` qualifies but a bare
///    `record` keyword in some other language (Erlang!) does not.
/// 4. Hits from files whose language isn't in the registry are downgraded
///    to `partial_language_match` rather than dropped — the caller is
///    warned via `partial_language_coverage`.
#[allow(clippy::too_many_arguments)]
fn scan_symbol_references(
    store: &IndexStore,
    symbol_id: &str,
    symbol_name: &str,
    defining_file: &str,
    require_receiver: bool,
    include_vendored: bool,
    scan_limit: usize,
) -> anyhow::Result<ReferenceScan> {
    if symbol_name.is_empty() {
        return Ok(ReferenceScan {
            references: Vec::new(),
            complete: true,
            scanned_files: 0,
            excluded_vendored_or_fixture: 0,
            rejected_comment_or_string: 0,
            rejected_missing_receiver: 0,
            partial_language_coverage: false,
            confidence: "high",
        });
    }
    let sym_infos = store.all_symbols_with_source()?;
    let mut seen_files: FxHashSet<String> = FxHashSet::default();
    let mut references = Vec::new();
    let mut complete = true;
    let mut excluded_vendored = 0usize;
    let mut rejected_comment = 0usize;
    let mut rejected_no_receiver = 0usize;
    let mut partial_language = false;
    for (idx, sym) in sym_infos.iter().enumerate() {
        if idx >= scan_limit {
            complete = false;
            break;
        }
        if sym.id == symbol_id {
            continue;
        }
        if sym.file == defining_file {
            continue;
        }
        if !include_vendored && is_vendored_or_fixture_path(&sym.file) {
            excluded_vendored += 1;
            continue;
        }
        let Some((rel_line, col)) = locate_identifier_ci(&sym.source, symbol_name) else {
            continue;
        };
        let line_text = sym
            .source
            .lines()
            .nth(rel_line.saturating_sub(1) as usize)
            .unwrap_or("");
        // `col` is 1-based; `classify_body_match` takes a 0-based column.
        let body_match = classify_body_match(
            line_text,
            col.saturating_sub(1) as usize,
            symbol_name.len(),
        );
        if body_match == "comment_or_string" {
            rejected_comment += 1;
            continue;
        }
        if require_receiver
            && !has_receiver_prefix(line_text, col.saturating_sub(1) as usize)
        {
            rejected_no_receiver += 1;
            continue;
        }
        let lang_id = language_id_for_path(&sym.file);
        let kind = if lang_id == UNKNOWN_LANGUAGE {
            partial_language = true;
            "partial_language_match"
        } else {
            "symbol_body_reference"
        };
        let absolute_line = sym.line + rel_line.saturating_sub(1);
        references.push(ReferenceScanRow {
            file: sym.file.clone(),
            line: absolute_line,
            kind,
            enclosing_symbol: Some(sym.name.clone()),
            body_match_kind: body_match,
        });
        seen_files.insert(sym.file.clone());
    }
    let confidence = if !partial_language && references.iter().all(|r| r.kind == "symbol_body_reference") {
        "high"
    } else if references.is_empty() {
        "none_after_filters"
    } else {
        "partial"
    };
    Ok(ReferenceScan {
        references,
        complete,
        scanned_files: seen_files.len(),
        excluded_vendored_or_fixture: excluded_vendored,
        rejected_comment_or_string: rejected_comment,
        rejected_missing_receiver: rejected_no_receiver,
        partial_language_coverage: partial_language,
        confidence,
    })
}

/// True when the identifier at 0-based byte column `col` is immediately
/// preceded (ignoring turbofish) by `.` or `::` — a receiver / path
/// qualifier. Used to distinguish `ledger.record(...)` and
/// `SavingsLedger::record(...)` from a bare `record` token that happens
/// to live in unrelated code (Erlang keyword, field name, etc.).
fn has_receiver_prefix(line_text: &str, col: usize) -> bool {
    if col == 0 {
        return false;
    }
    let before = &line_text.as_bytes()[..col.min(line_text.len())];
    // Tolerate whitespace / `<T>` between the qualifier and the identifier
    // (`obj . record(`, `Ledger:: <u32>record(`) — bounded scan only.
    let mut end = before.len();
    while end > 0 && (before[end - 1] == b' ' || before[end - 1] == b'\t') {
        end -= 1;
    }
    if end == 0 {
        return false;
    }
    let trimmed = &before[..end];
    if trimmed.ends_with(b".") {
        return true;
    }
    if trimmed.ends_with(b"::") {
        return true;
    }
    // `foo::<T>::record(` — after trimming angle-bracketed generics the
    // tail should still end in `::`.
    if let Some(lt) = trimmed.iter().rposition(|b| *b == b'<')
        && let Some(gt) = trimmed.iter().rposition(|b| *b == b'>')
        && lt < gt
    {
        let head = &trimmed[..lt];
        if head.ends_with(b"::") {
            return true;
        }
    }
    false
}

/// FIXUP T1.1 + crate_graph: dispatch trait/interface detection to the
/// owning Language. Per-language code (`.ends_with(".rs")` etc.) used to
/// live here; it now sits beside the rest of each language's
/// implementation in `codesight-language-support`.
fn detect_trait_impl_sites(
    store: &IndexStore,
    file_path: &str,
    symbol_name: &str,
) -> anyhow::Result<(Option<String>, Vec<String>)> {
    if symbol_name.is_empty() {
        return Ok((None, Vec::new()));
    }
    let registry = codesight_language_support::builtin_registry();
    let Some(lang) = registry.for_path(std::path::Path::new(file_path)) else {
        return Ok((None, Vec::new()));
    };
    let contents = store.all_file_contents().unwrap_or_default();
    let defining = contents
        .iter()
        .find(|(p, _)| p == file_path)
        .map(|(_, c)| c.as_str())
        .unwrap_or("");
    Ok(
        match lang.detect_trait_impl_sites(defining, &contents, file_path, symbol_name) {
            Some((name, hits)) => (Some(name), hits),
            None => (None, Vec::new()),
        },
    )
}

// -----------------------------------------------------------------------------
// path_rename — dispatch to the language-owned engine
// -----------------------------------------------------------------------------

/// Preview a module-path rewrite using the language associated with `lang_id`
/// (e.g. `"rust"`). Returns `Ok(None)` when the language doesn't expose a
/// path-rename engine so callers can render a consistent error envelope.
pub fn preview_path_rename(
    store: &IndexStore,
    lang_id: &str,
    from: &str,
    to: &str,
    scope: Option<&str>,
    partial_mode: &str,
) -> anyhow::Result<Option<PathRenamePlan>> {
    let Some(lang) = codesight_language_support::builtin_registry().for_language_name(lang_id)
    else {
        return Ok(None);
    };
    let Some(engine) = lang.path_rename_engine() else {
        return Ok(None);
    };
    let root = store.root_path()?;
    engine
        .preview(&root, from, to, scope, partial_mode)
        .map(Some)
}
