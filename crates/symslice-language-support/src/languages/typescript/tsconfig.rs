//! tsconfig.json / jsconfig.json path-alias loader.
//!
//! TypeScript's `compilerOptions.paths` map is the canonical alias source
//! for TS + modern JS projects. Lives here (not in the generic orchestrator)
//! because its shape — JSONC comments, `*` glob matching, `./` prefix
//! stripping — is a TypeScript convention.

use rustc_hash::FxHashMap;

/// Load path aliases from `<source_root>/tsconfig.json` (supports JSONC).
pub fn load_tsconfig_aliases(source_root: &str) -> FxHashMap<String, Vec<String>> {
    if source_root.is_empty() {
        return FxHashMap::default();
    }

    let tsconfig_path = format!("{source_root}/tsconfig.json");
    let content = match std::fs::read_to_string(&tsconfig_path) {
        Ok(c) => c,
        Err(_) => return FxHashMap::default(),
    };

    parse_tsconfig_paths(&content)
}

/// Parse tsconfig JSON(C) content and extract `compilerOptions.paths`.
pub fn parse_tsconfig_paths(content: &str) -> FxHashMap<String, Vec<String>> {
    let parsed = match jsonc_parser::parse_to_serde_value(content, &Default::default()) {
        Ok(Some(v)) => v,
        _ => return FxHashMap::default(),
    };

    let paths = match parsed.get("compilerOptions").and_then(|co| co.get("paths")) {
        Some(p) => p,
        None => return FxHashMap::default(),
    };

    let mut result = FxHashMap::default();
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
