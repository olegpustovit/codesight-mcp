//! Jupyter notebook JSON to Markdown preprocessing.

pub fn preprocess_notebook(content: &str) -> (String, Option<String>) {
    let parsed: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => return (String::new(), None),
    };

    // Detect language from metadata
    let language = parsed
        .get("metadata")
        .and_then(|m| {
            m.get("kernelspec")
                .and_then(|k| k.get("language"))
                .and_then(|v| v.as_str())
                .or_else(|| {
                    m.get("language_info")
                        .and_then(|li| li.get("name"))
                        .and_then(|v| v.as_str())
                })
        })
        .unwrap_or("python")
        .to_string();

    let cells = match parsed.get("cells").and_then(|c| c.as_array()) {
        Some(arr) => arr,
        None => return (String::new(), Some(language)),
    };

    let mut parts: Vec<String> = Vec::new();
    for cell in cells {
        let cell_type = cell.get("cell_type").and_then(|v| v.as_str()).unwrap_or("");

        let source_text = match cell.get("source") {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(""),
            Some(serde_json::Value::String(s)) => s.clone(),
            _ => continue,
        };

        if source_text.trim().is_empty() {
            continue;
        }

        match cell_type {
            "markdown" => {
                parts.push(source_text);
            }
            "code" => {
                parts.push(format!("```{language}\n{source_text}\n```"));
            }
            _ => {} // skip raw cells, etc.
        }
    }

    (parts.join("\n\n"), Some(language))
}

/// Preprocess document content: converts `.ipynb` to Markdown, passes other
/// formats through unchanged.
pub fn preprocess_content(content: &str, file_path: &str) -> String {
    let lower = file_path.to_ascii_lowercase();
    if lower.ends_with(".ipynb") {
        let (md, _lang) = preprocess_notebook(content);
        md
    } else {
        content.to_string()
    }
}
