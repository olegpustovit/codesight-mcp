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

use std::collections::HashSet;

use serde_json::{Value, json};

pub use codesight_language_support::path_rename::{
    PathRenameChange, PathRenameCollision, PathRenameFilePlan, PathRenamePlan, PathRenameUnhandled,
    affected_paths, summarize as summarize_path_rename,
};
use codesight_storage::storage::IndexStore;

// -----------------------------------------------------------------------------
// rename_safety — symbol-level rename conflict check
// -----------------------------------------------------------------------------

pub fn rename_safety_payload(
    store: &IndexStore,
    symbol_id: &str,
    new_name: &str,
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
    let mut checked_files = HashSet::from([file_path.clone()]);
    let mut reverse_adj = std::collections::HashMap::<String, Vec<String>>::new();
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

    Ok(Some(json!({
        "analysis": "rename_safety",
        "safe": conflicts.is_empty(),
        "conflicts": conflicts,
        "checked_files": checked_files.len(),
        "symbol": {
            "name": symbol_name,
            "id": symbol_id,
            "file": file_path,
        },
        "new_name": new_name,
    })))
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
    engine.preview(&root, from, to, scope, partial_mode).map(Some)
}
