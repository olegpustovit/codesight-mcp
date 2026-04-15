//! OpenAPI/Swagger and generic JSON to Markdown for section extraction.

use std::collections::BTreeMap;

pub fn sniff_openapi(content: &str, file_path: &str) -> bool {
    let lower = file_path.to_ascii_lowercase();
    if !(lower.ends_with(".yaml") || lower.ends_with(".yml") || lower.ends_with(".json")) {
        return false;
    }
    // Quick text scan for openapi/swagger keys
    let content_lower = content.to_ascii_lowercase();
    content_lower.contains("openapi")
        || content_lower.contains("\"swagger\"")
        || content_lower.contains("swagger:")
}

/// Convert an OpenAPI 3.x / Swagger 2.0 spec to Markdown suitable for section extraction.
///
/// Returns an empty string if the content is not a valid OpenAPI/Swagger document.
pub fn convert_openapi(content: &str) -> String {
    // Try parsing as JSON first, then YAML
    let doc: serde_json::Value = if content.trim_start().starts_with('{') {
        match serde_json::from_str(content) {
            Ok(v) => v,
            Err(_) => return String::new(),
        }
    } else {
        match serde_yaml::from_str(content) {
            Ok(v) => v,
            Err(_) => return String::new(),
        }
    };

    // Validate it is actually OpenAPI/Swagger
    let is_openapi = doc.get("openapi").is_some();
    let is_swagger = doc.get("swagger").is_some();
    if !is_openapi && !is_swagger {
        return String::new();
    }

    let mut out = String::new();

    // Title
    let title = doc
        .pointer("/info/title")
        .and_then(|v| v.as_str())
        .unwrap_or("API");
    out.push_str(&format!("# {title}\n\n"));

    // Version
    if let Some(version) = doc.pointer("/info/version").and_then(|v| v.as_str()) {
        out.push_str(&format!("Version: {version}\n\n"));
    }

    // Description
    if let Some(desc) = doc.pointer("/info/description").and_then(|v| v.as_str()) {
        out.push_str(desc);
        out.push_str("\n\n");
    }

    // Group operations by tag
    let paths = doc.get("paths").and_then(|v| v.as_object());

    // Collect all tags used
    let mut tag_ops: BTreeMap<String, Vec<(String, String, &serde_json::Value)>> = BTreeMap::new();
    let mut untagged_ops: Vec<(String, String, &serde_json::Value)> = Vec::new();

    if let Some(paths_map) = paths {
        let methods = [
            "get", "post", "put", "patch", "delete", "head", "options", "trace",
        ];
        for (path, path_item) in paths_map {
            if let Some(obj) = path_item.as_object() {
                for method in &methods {
                    if let Some(op) = obj.get(*method) {
                        let method_upper = method.to_ascii_uppercase();
                        if let Some(tags_arr) = op.get("tags").and_then(|v| v.as_array()) {
                            for tag_val in tags_arr {
                                if let Some(tag) = tag_val.as_str() {
                                    tag_ops.entry(tag.to_string()).or_default().push((
                                        method_upper.clone(),
                                        path.clone(),
                                        op,
                                    ));
                                }
                            }
                        } else {
                            untagged_ops.push((method_upper, path.clone(), op));
                        }
                    }
                }
            }
        }
    }

    // Render tagged operations
    for (tag, ops) in &tag_ops {
        out.push_str(&format!("## {tag}\n\n"));
        for (method, path, op) in ops {
            out.push_str(&format!("### {method} {path}\n\n"));
            render_operation(&mut out, op);
        }
    }

    // Render untagged operations
    if !untagged_ops.is_empty() && tag_ops.is_empty() {
        for (method, path, op) in &untagged_ops {
            out.push_str(&format!("### {method} {path}\n\n"));
            render_operation(&mut out, op);
        }
    }

    // Schemas section
    // OpenAPI 3.x: components.schemas, Swagger 2.0: definitions
    let schemas = doc
        .pointer("/components/schemas")
        .or_else(|| doc.get("definitions"))
        .and_then(|v| v.as_object());

    if let Some(schemas_map) = schemas {
        out.push_str("## Schemas\n\n");
        for (name, schema) in schemas_map {
            out.push_str(&format!("### {name}\n\n"));
            if let Some(desc) = schema.get("description").and_then(|v| v.as_str()) {
                out.push_str(desc);
                out.push_str("\n\n");
            }
            // Render properties
            if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
                let required: std::collections::HashSet<&str> = schema
                    .get("required")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();
                out.push_str("Properties:\n\n");
                for (prop_name, prop_schema) in props {
                    let typ = prop_schema
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("any");
                    let req_marker = if required.contains(prop_name.as_str()) {
                        " *(required)*"
                    } else {
                        ""
                    };
                    out.push_str(&format!("- `{prop_name}`: {typ}{req_marker}\n"));
                }
                out.push('\n');
            }
        }
    }

    out
}

fn render_operation(out: &mut String, op: &serde_json::Value) {
    // Summary
    if let Some(summary) = op.get("summary").and_then(|v| v.as_str()) {
        out.push_str(summary);
        out.push_str("\n\n");
    }
    // Description
    if let Some(desc) = op.get("description").and_then(|v| v.as_str()) {
        out.push_str(desc);
        out.push_str("\n\n");
    }
    // Parameters
    if let Some(params) = op.get("parameters").and_then(|v| v.as_array())
        && !params.is_empty()
    {
        out.push_str("Parameters:\n\n");
        for param in params {
            let name = param.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let location = param.get("in").and_then(|v| v.as_str()).unwrap_or("?");
            let desc = param
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            out.push_str(&format!("- `{name}` ({location}): {desc}\n"));
        }
        out.push('\n');
    }
    // Request body
    if let Some(rb) = op.get("requestBody") {
        out.push_str("Request Body");
        // Check for schema ref
        if let Some(schema_ref) = rb
            .pointer("/content/application/json/schema/$ref")
            .and_then(|v| v.as_str())
        {
            let ref_name = schema_ref.rsplit('/').next().unwrap_or(schema_ref);
            out.push_str(&format!(": {ref_name}"));
        }
        out.push_str("\n\n");
    }
    // Responses
    if let Some(responses) = op.get("responses").and_then(|v| v.as_object()) {
        out.push_str("Responses:\n\n");
        for (code, resp) in responses {
            let desc = resp
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            out.push_str(&format!("- `{code}`: {desc}\n"));
        }
        out.push('\n');
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// JSON → Markdown converter
// ─────────────────────────────────────────────────────────────────────────────

/// Convert a JSON document to Markdown suitable for section extraction.
///
/// Top-level object keys become `##` sections; nested objects become `###`.
/// Arrays are rendered as bullet lists.  Scalar roots return empty.
/// Supports JSONC (strips `//` and `/* */` comments before parsing).
pub fn convert_json(content: &str, file_path: &str) -> String {
    let stripped = strip_jsonc_comments(content);
    let doc: serde_json::Value = match serde_json::from_str(&stripped) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };

    let title = if file_path.is_empty() {
        "JSON Document".to_string()
    } else {
        std::path::Path::new(file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("JSON Document")
            .to_string()
    };

    match &doc {
        serde_json::Value::Object(map) => {
            let mut out = format!("# {title}\n\n");
            for (key, value) in map {
                render_json_section(&mut out, key, value, 2);
            }
            out
        }
        serde_json::Value::Array(arr) => {
            let mut out = format!("# {title}\n\n");
            for item in arr {
                match item {
                    serde_json::Value::Object(map) => {
                        for (k, v) in map {
                            let s = render_json_value(v);
                            if !s.is_empty() {
                                out.push_str(&format!("- {k}: {s}\n"));
                            }
                        }
                    }
                    other => {
                        let s = render_json_value(other);
                        if !s.is_empty() {
                            out.push_str(&format!("- {s}\n"));
                        }
                    }
                }
            }
            out
        }
        _ => String::new(), // scalar root
    }
}

fn render_json_section(out: &mut String, key: &str, value: &serde_json::Value, depth: usize) {
    let hashes = "#".repeat(depth.min(6));
    match value {
        serde_json::Value::Object(map) => {
            out.push_str(&format!("{hashes} {key}\n\n"));
            for (k, v) in map {
                render_json_section(out, k, v, depth + 1);
            }
        }
        serde_json::Value::Array(arr) => {
            out.push_str(&format!("{hashes} {key}\n\n"));
            for item in arr {
                let s = render_json_value(item);
                if !s.is_empty() {
                    out.push_str(&format!("- {s}\n"));
                }
            }
            out.push('\n');
        }
        other => {
            let s = render_json_value(other);
            out.push_str(&format!("{hashes} {key}\n\n{s}\n\n"));
        }
    }
}

fn render_json_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .map(render_json_value)
            .collect::<Vec<_>>()
            .join(", "),
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, v)| format!("{k}: {}", render_json_value(v)))
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// Strip JSONC-style comments (`//` line comments, `/* */` block comments).
fn strip_jsonc_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;

    while let Some(ch) = chars.next() {
        if in_string {
            result.push(ch);
            if ch == '\\' {
                if let Some(&next) = chars.peek() {
                    result.push(next);
                    chars.next();
                }
            } else if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            in_string = true;
            result.push(ch);
        } else if ch == '/' {
            match chars.peek() {
                Some(&'/') => {
                    // Line comment — skip to end of line
                    chars.next();
                    for c in chars.by_ref() {
                        if c == '\n' {
                            result.push('\n');
                            break;
                        }
                    }
                }
                Some(&'*') => {
                    // Block comment — skip to */
                    chars.next();
                    let mut prev = ' ';
                    for c in chars.by_ref() {
                        if prev == '*' && c == '/' {
                            break;
                        }
                        prev = c;
                    }
                }
                _ => result.push(ch),
            }
        } else {
            result.push(ch);
        }
    }
    result
}
