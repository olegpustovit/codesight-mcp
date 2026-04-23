//! tsconfig.json / jsconfig.json path-alias loader.
//!
//! TypeScript's `compilerOptions.paths` map is the canonical alias source
//! for TS + modern JS projects. Lives here (not in the generic orchestrator)
//! because its shape — JSONC comments, `*` glob matching, `./` prefix
//! stripping — is a TypeScript convention.

use std::collections::HashMap;

/// Load path aliases from `<source_root>/tsconfig.json` (supports JSONC).
pub fn load_tsconfig_aliases(source_root: &str) -> HashMap<String, Vec<String>> {
    if source_root.is_empty() {
        return HashMap::new();
    }

    let tsconfig_path = format!("{source_root}/tsconfig.json");
    let content = match std::fs::read_to_string(&tsconfig_path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };

    parse_tsconfig_paths(&content)
}

/// Parse tsconfig JSON(C) content and extract `compilerOptions.paths`.
pub fn parse_tsconfig_paths(content: &str) -> HashMap<String, Vec<String>> {
    let clean = strip_jsonc_comments(content);
    let parsed: serde_json::Value = match serde_json::from_str(&clean) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };

    let paths = match parsed.get("compilerOptions").and_then(|co| co.get("paths")) {
        Some(p) => p,
        None => return HashMap::new(),
    };

    let mut result = HashMap::new();
    if let Some(obj) = paths.as_object() {
        for (key, value) in obj {
            if let Some(arr) = value.as_array() {
                let normalized: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| {
                        if s == "./*" {
                            "/*".to_string()
                        } else {
                            s.strip_prefix("./").unwrap_or(s).to_string()
                        }
                    })
                    .collect();
                result.insert(key.clone(), normalized);
            }
        }
    }
    result
}

fn strip_jsonc_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut in_string = false;

    while i < len {
        if in_string {
            result.push(chars[i]);
            if chars[i] == '\\' && i + 1 < len {
                i += 1;
                result.push(chars[i]);
            } else if chars[i] == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if chars[i] == '"' {
            in_string = true;
            result.push(chars[i]);
            i += 1;
            continue;
        }

        if chars[i] == '/' && i + 1 < len {
            if chars[i + 1] == '/' {
                i += 2;
                while i < len && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            } else if chars[i + 1] == '*' {
                i += 2;
                while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
                continue;
            }
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}
