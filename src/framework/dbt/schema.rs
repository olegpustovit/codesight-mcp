use std::collections::HashMap;
use std::path::Path;

use crate::framework::FileContext;

use super::docs::resolve_description;

/// Metadata extracted from a dbt YAML schema file for a single model.
#[derive(Debug, Clone, Default)]
pub struct DbtModelMetadata {
    pub description: String,
    pub tags: Vec<String>,
    pub columns: HashMap<String, String>,
}

impl DbtModelMetadata {
    /// Convert to a `FileContext`.
    pub fn to_file_context(&self) -> FileContext {
        FileContext {
            description: self.description.clone(),
            tags: self.tags.clone(),
            properties: self
                .columns
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }
}

/// Parse models from YAML content. Returns model_name -> DbtModelMetadata.
pub fn parse_yml_content(
    content: &str,
    doc_blocks: &HashMap<String, String>,
) -> HashMap<String, DbtModelMetadata> {
    let mut result = HashMap::new();

    let yaml: serde_yaml::Value = match serde_yaml::from_str(content) {
        Ok(v) => v,
        Err(_) => return result,
    };

    let models = match yaml.get("models") {
        Some(serde_yaml::Value::Sequence(seq)) => seq,
        _ => return result,
    };

    for model in models {
        let name = match model.get("name") {
            Some(serde_yaml::Value::String(s)) => s.clone(),
            _ => continue,
        };

        let raw_desc = model
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let description = resolve_description(&raw_desc, doc_blocks);

        // Tags: merge top-level tags + config.tags, deduplicated
        let mut tags = Vec::new();
        if let Some(serde_yaml::Value::Sequence(top_tags)) = model.get("tags") {
            for t in top_tags {
                if let Some(s) = t.as_str() {
                    tags.push(s.to_string());
                }
            }
        }
        if let Some(config) = model.get("config")
            && let Some(serde_yaml::Value::Sequence(cfg_tags)) = config.get("tags")
        {
            for t in cfg_tags {
                if let Some(s) = t.as_str()
                    && !tags.contains(&s.to_string())
                {
                    tags.push(s.to_string());
                }
            }
        }

        let mut columns = HashMap::new();
        if let Some(serde_yaml::Value::Sequence(cols)) = model.get("columns") {
            for col in cols {
                if let Some(col_name) = col.get("name").and_then(|v| v.as_str()) {
                    let col_desc_raw = col
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let col_desc = resolve_description(&col_desc_raw, doc_blocks);
                    columns.insert(col_name.to_string(), col_desc);
                }
            }
        }

        result.insert(
            name,
            DbtModelMetadata {
                description,
                tags,
                columns,
            },
        );
    }

    result
}

/// Parse YAML files in a directory.
pub fn parse_yml_files(
    dir: &Path,
    doc_blocks: &HashMap<String, String>,
) -> HashMap<String, DbtModelMetadata> {
    let mut result = HashMap::new();
    if !dir.exists() {
        return result;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if (ext == "yml" || ext == "yaml")
                && let Ok(content) = std::fs::read_to_string(&path)
            {
                result.extend(parse_yml_content(&content, doc_blocks));
            }
        }
    }
    result
}
