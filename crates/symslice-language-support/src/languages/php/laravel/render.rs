use rustc_hash::FxHashMap;

use crate::frameworks::FileContext;

use super::snapshot::LaravelSnapshot;

pub fn render_file_context(snapshot: &LaravelSnapshot, rel_path: &str) -> Option<FileContext> {
    let normalized = rel_path.replace('\\', "/");
    snapshot.file_contexts.get(&normalized).cloned()
}

pub fn metadata(snapshot: &LaravelSnapshot) -> FxHashMap<String, serde_json::Value> {
    let mut meta = FxHashMap::default();
    let mut columns_map = serde_json::Map::new();

    for (table, migration) in &snapshot.migrations {
        let columns: Vec<serde_json::Value> = migration
            .columns
            .keys()
            .map(|column| serde_json::Value::String(column.clone()))
            .collect();
        columns_map.insert(table.clone(), serde_json::Value::Array(columns));
    }

    meta.insert(
        "laravel_columns".into(),
        serde_json::Value::Object(columns_map),
    );
    meta
}
