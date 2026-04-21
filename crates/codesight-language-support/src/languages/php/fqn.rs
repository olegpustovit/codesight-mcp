//! PHP PSR-4 autoload map + FQN ↔ symbol_id conversion.
//!
//! Kept next to the PHP language module because the semantics (namespace
//! separator `\`, `.php` extension, `composer.json` layout) are PHP-specific.

use std::collections::{HashMap, HashSet};

use codesight_core::SymbolIdRef;

/// PSR-4 autoloading map: namespace prefix → directory prefix.
/// E.g. `{"App\\": "app/", "Database\\": "database/"}`
pub type Psr4Map = HashMap<String, String>;

/// Load PSR-4 autoloading map from a `composer.json` file.
pub fn load_psr4_map(source_root: &str) -> Psr4Map {
    let composer_path = format!("{source_root}/composer.json");
    let content = match std::fs::read_to_string(&composer_path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };

    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };

    let mut map = HashMap::new();

    if let Some(autoload) = parsed.get("autoload")
        && let Some(psr4) = autoload.get("psr-4").and_then(|v| v.as_object())
    {
        for (ns, dir) in psr4 {
            if let Some(dir_str) = dir.as_str() {
                let dir_norm = if dir_str.ends_with('/') {
                    dir_str.to_string()
                } else {
                    format!("{dir_str}/")
                };
                map.insert(ns.clone(), dir_norm);
            }
        }
    }

    if let Some(autoload_dev) = parsed.get("autoload-dev")
        && let Some(psr4) = autoload_dev.get("psr-4").and_then(|v| v.as_object())
    {
        for (ns, dir) in psr4 {
            if let Some(dir_str) = dir.as_str() {
                let dir_norm = if dir_str.ends_with('/') {
                    dir_str.to_string()
                } else {
                    format!("{dir_str}/")
                };
                map.entry(ns.clone()).or_insert(dir_norm);
            }
        }
    }

    map
}

/// Convert a PSR-4 symbol_id to a fully qualified name.
pub fn symbol_to_fqn(symbol_id: &str, psr4_map: &Psr4Map) -> Option<String> {
    let id = SymbolIdRef::new(symbol_id);
    let file_part = id.file_scope();
    if file_part.is_empty() || !file_part.ends_with(".php") {
        return None;
    }
    let kind = id.kind()?;

    match kind {
        "class" => {
            for (namespace, dir) in psr4_map {
                if file_part.starts_with(dir.as_str()) {
                    let relative = &file_part[dir.len()..];
                    let without_ext = relative.strip_suffix(".php")?;
                    let ns_path = without_ext.replace('/', "\\");
                    return Some(format!("{namespace}{ns_path}"));
                }
            }
            None
        }
        "method" => {
            let class_name = id.parent_path();
            let method_name = id.leaf_name();
            if class_name.is_empty() || method_name.is_empty() {
                return None;
            }
            let class_symbol_id = format!("{file_part}::{class_name}#class");
            let class_fqn = symbol_to_fqn(&class_symbol_id, psr4_map)?;
            Some(format!("{class_fqn}::{method_name}"))
        }
        _ => None,
    }
}

/// Convert a fully qualified name back to a symbol_id.
pub fn fqn_to_symbol(
    fqn: &str,
    psr4_map: &Psr4Map,
    source_files: &HashSet<String>,
) -> Option<String> {
    if !fqn.contains('\\') {
        return None;
    }

    if let Some((class_fqn, method_name)) = fqn.rsplit_once("::") {
        let class_symbol = fqn_to_symbol(class_fqn, psr4_map, source_files)?;
        let class_id = SymbolIdRef::new(&class_symbol);
        let file_part = class_id.file_scope();
        let class_name = class_id.leaf_name();
        if class_id.kind() != Some("class") || file_part.is_empty() || class_name.is_empty() {
            return None;
        }
        return Some(format!("{file_part}::{class_name}.{method_name}#method"));
    }

    let mut prefixes: Vec<(&String, &String)> = psr4_map.iter().collect();
    prefixes.sort_by_key(|entry| std::cmp::Reverse(entry.0.len()));

    for (namespace, dir) in prefixes {
        if fqn.starts_with(namespace.as_str()) {
            let relative = &fqn[namespace.len()..];
            let file_path = format!("{}{}.php", dir, relative.replace('\\', "/"));
            if source_files.contains(&file_path) {
                let class_name = fqn.rsplit('\\').next()?;
                return Some(format!("{file_path}::{class_name}#class"));
            }
        }
    }

    None
}
