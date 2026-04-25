//! JS/TS module-resolution helpers.
//!
//! These live on the owning language so that non-JS callers of
//! `resolve_specifier` do not inherit JS extension guessing.

use rustc_hash::FxHashSet;

use path_clean::PathClean;

/// Resolve a `./` or `../` relative specifier using JS module semantics:
/// normalize the path against `from_file`, then try the configured
/// extension table and index files.
pub fn resolve_js_relative(
    specifier: &str,
    from_file: &str,
    source_files: &FxHashSet<String>,
) -> Option<String> {
    let dir = parent_dir(from_file);
    let resolved = std::path::Path::new(&format!("{dir}/{specifier}"))
        .clean()
        .to_string_lossy()
        .replace('\\', "/");
    resolve_js_path(&resolved, source_files)
}

/// JS-style extension / index-file resolver.
///
/// Tries (in order): the exact base, each of `.ts/.tsx/.js/.jsx/.vue/.mjs/.cjs`,
/// `index.{ts,tsx,js,jsx}`, and finally `.js → .ts/.tsx` remap for projects
/// that kept `.js` imports after a TypeScript migration.
pub fn resolve_js_path(base: &str, source_files: &FxHashSet<String>) -> Option<String> {
    if source_files.contains(base) {
        return Some(base.to_string());
    }

    let extensions = [".ts", ".tsx", ".js", ".jsx", ".vue", ".mjs", ".cjs"];
    for ext in &extensions {
        let with_ext = format!("{base}{ext}");
        if source_files.contains(&with_ext) {
            return Some(with_ext);
        }
    }

    let index_extensions = ["index.ts", "index.tsx", "index.js", "index.jsx"];
    for idx in &index_extensions {
        let with_index = format!("{base}/{idx}");
        if source_files.contains(&with_index) {
            return Some(with_index);
        }
    }

    if let Some(without) = base.strip_suffix(".js") {
        let ts_extensions = [".ts", ".tsx"];
        for ext in &ts_extensions {
            let mapped = format!("{without}{ext}");
            if source_files.contains(&mapped) {
                return Some(mapped);
            }
        }
    }

    None
}

fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(idx) => path[..idx].to_string(),
        None => ".".to_string(),
    }
}
