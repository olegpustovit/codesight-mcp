//! Reference existence / usage heuristics for `check_references`.

use serde_json::{Value, json};

use crate::{dep_graph, storage::IndexStore};

pub fn check_identifiers_batch(
    store: &IndexStore,
    identifiers: &[String],
) -> anyhow::Result<Value> {
    let sym_infos = store.all_symbols_with_source()?;
    let all_contents = store.all_file_contents().unwrap_or_default();
    let mut results: Vec<Value> = Vec::new();
    for identifier in identifiers {
        let mut import_count: u64 = 0;
        let mut content_count: u64 = 0;
        for sym in &sym_infos {
            if sym.name == *identifier {
                continue;
            }
            if dep_graph::word_match(&sym.source, identifier) {
                content_count += 1;
            }
        }
        for (_, content) in &all_contents {
            for line in content.lines() {
                let trimmed = line.trim();
                if (trimmed.starts_with("import ") || trimmed.starts_with("from "))
                    && trimmed.contains(identifier.as_str())
                {
                    import_count += 1;
                }
            }
        }
        let is_referenced = import_count > 0 || content_count > 0;
        results.push(json!({
            "identifier": identifier,
            "is_referenced": is_referenced,
            "import_count": import_count,
            "content_count": content_count,
        }));
    }
    Ok(json!({
        "checked_count": results.len(),
        "results": results,
    }))
}

pub fn check_single_identifier(
    store: &IndexStore,
    identifier: &str,
    search_content: bool,
) -> anyhow::Result<Value> {
    let mut import_count: u64 = 0;
    let mut content_count: u64 = 0;
    let mut referencing_files: Vec<String> = Vec::new();

    if let Ok(all_contents) = store.all_file_contents() {
        for (file, content) in &all_contents {
            let mut file_has_import = false;
            let mut file_has_content = false;
            for line in content.lines() {
                let trimmed = line.trim();
                if (trimmed.starts_with("import ") || trimmed.starts_with("from "))
                    && trimmed.contains(identifier)
                {
                    if !file_has_import {
                        import_count += 1;
                        file_has_import = true;
                    }
                } else if search_content
                    && !trimmed.starts_with("def ")
                    && !trimmed.starts_with("class ")
                    && !trimmed.starts_with('#')
                    && dep_graph::word_match(trimmed, identifier)
                {
                    if !file_has_content {
                        content_count += 1;
                        file_has_content = true;
                    }
                }
            }
            if (file_has_import || file_has_content) && !referencing_files.contains(file) {
                referencing_files.push(file.clone());
            }
        }
    }

    let is_referenced = import_count > 0 || content_count > 0;
    Ok(json!({
        "identifier": identifier,
        "is_referenced": is_referenced,
        "import_count": import_count,
        "content_count": content_count,
        "referencing_files": referencing_files,
    }))
}

pub fn check_refs_existence(
    store: &IndexStore,
    refs: &[(String, String)],
) -> anyhow::Result<Value> {
    let mut results: Vec<Value> = Vec::new();
    for (kind, id) in refs {
        let exists = if kind == "symbol" {
            store.symbol_record(id)?.is_some()
        } else if kind == "file" {
            store.file_exists(id)?
        } else {
            false
        };
        results.push(json!({
            "ref": { "stable_id": id, "kind": kind },
            "exists": exists,
            "stale": false,
        }));
    }
    Ok(json!({
        "checked_count": results.len(),
        "results": results,
    }))
}
