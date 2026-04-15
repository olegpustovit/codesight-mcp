//! Domain-level impact payload builders used by handlers and slice tests.

use std::collections::HashSet;

use serde_json::{Value, json};

use codesight_storage::storage::IndexStore;

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
        for importer in importers {
            checked_files.insert(importer.clone());
            let importer_outline = store.file_outline(importer)?;
            for symbol in &importer_outline {
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
