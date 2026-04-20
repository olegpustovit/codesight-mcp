use std::collections::HashMap;

use crate::frameworks::FileContext;

use super::snapshot::DbtSnapshot;

pub fn render_file_context(snapshot: &DbtSnapshot, rel_path: &str) -> Option<FileContext> {
    if !snapshot.is_in_model_dir(rel_path) {
        return None;
    }
    let stem = DbtSnapshot::file_stem(rel_path)?;
    snapshot
        .models
        .get(&stem)
        .map(|model| model.to_file_context())
}

pub fn metadata(snapshot: &DbtSnapshot) -> HashMap<String, serde_json::Value> {
    let mut meta = HashMap::new();
    meta.insert(
        "dbt_doc_blocks".into(),
        serde_json::Value::Number(serde_json::Number::from(snapshot.doc_blocks.len())),
    );
    meta.insert(
        "dbt_models".into(),
        serde_json::Value::Number(serde_json::Number::from(snapshot.models.len())),
    );
    meta
}
