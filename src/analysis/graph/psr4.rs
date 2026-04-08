//! PSR-4 autoload map from composer.json and FQN ↔ symbol_id conversion.

use std::collections::{HashMap, HashSet};

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
                // Normalize: ensure trailing "/" on directory
                let dir_norm = if dir_str.ends_with('/') {
                    dir_str.to_string()
                } else {
                    format!("{dir_str}/")
                };
                map.insert(ns.clone(), dir_norm);
            }
        }
    }

    // Also check autoload-dev
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
///
/// Input: `"app/Models/User.php::User#class"` with PSR-4 map `{"App\\": "app/"}`
/// Output: `Some("App\\Models\\User")`
///
/// For method symbols: `"app/Models/User.php::User.posts#method"` → `"App\\Models\\User::posts"`
pub fn symbol_to_fqn(symbol_id: &str, psr4_map: &Psr4Map) -> Option<String> {
    let (file_part, symbol_part) = symbol_id.split_once("::")?;

    // Must be a PHP file
    if !file_part.ends_with(".php") {
        return None;
    }

    // Parse the symbol part: "ClassName#class" or "ClassName.method#method"
    let (name_part, kind) = symbol_part.rsplit_once('#')?;

    // Only class and method symbols have FQNs
    match kind {
        "class" => {
            // Find matching PSR-4 prefix
            for (namespace, dir) in psr4_map {
                if file_part.starts_with(dir.as_str()) {
                    let relative = &file_part[dir.len()..];
                    // Remove .php extension
                    let without_ext = relative.strip_suffix(".php")?;
                    // Convert path separators to namespace separators
                    let ns_path = without_ext.replace('/', "\\");
                    return Some(format!("{namespace}{ns_path}"));
                }
            }
            None
        }
        "method" => {
            // "ClassName.method_name" → extract class part and method
            let (class_name, method_name) = name_part.split_once('.')?;
            // Build class FQN first
            let class_symbol_id = format!("{file_part}::{class_name}#class");
            let class_fqn = symbol_to_fqn(&class_symbol_id, psr4_map)?;
            Some(format!("{class_fqn}::{method_name}"))
        }
        _ => None, // functions, constants, etc. don't have PSR-4 FQNs
    }
}

/// Convert a fully qualified name back to a symbol_id.
///
/// Input: `"App\\Models\\User"` with PSR-4 map and source files
/// Output: `Some("app/Models/User.php::User#class")`
///
/// For method FQNs: `"App\\Models\\User::posts"` → `Some("app/Models/User.php::User.posts#method")`
pub fn fqn_to_symbol(
    fqn: &str,
    psr4_map: &Psr4Map,
    source_files: &HashSet<String>,
) -> Option<String> {
    // Must contain at least one backslash for namespace
    if !fqn.contains('\\') {
        return None;
    }

    // Check if this is a method FQN (contains ::)
    if let Some((class_fqn, method_name)) = fqn.rsplit_once("::") {
        // Resolve class first
        let class_symbol = fqn_to_symbol(class_fqn, psr4_map, source_files)?;
        // class_symbol is "path/File.php::ClassName#class"
        let (file_part, rest) = class_symbol.split_once("::")?;
        let class_name = rest.strip_suffix("#class")?;
        return Some(format!("{file_part}::{class_name}.{method_name}#method"));
    }

    // Find matching PSR-4 namespace prefix
    // Sort by longest prefix first for specificity
    let mut prefixes: Vec<(&String, &String)> = psr4_map.iter().collect();
    prefixes.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

    for (namespace, dir) in prefixes {
        if fqn.starts_with(namespace.as_str()) {
            let relative = &fqn[namespace.len()..];
            let file_path = format!("{}{}.php", dir, relative.replace('\\', "/"));
            if source_files.contains(&file_path) {
                // Extract class name (last component)
                let class_name = fqn.rsplit('\\').next()?;
                return Some(format!("{file_path}::{class_name}#class"));
            }
        }
    }

    None
}
