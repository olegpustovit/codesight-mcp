use rustc_hash::{FxHashMap, FxHashSet};
use std::path::Path;

use crate::frameworks::ExtraImport;

use super::api_client::extract_api_calls;
use super::blade::extract_blade_imports;
use super::facade::extract_facade_imports;
use super::inertia::{extract_inertia_renders, resolve_inertia_page};
use super::regex::RE;
use super::routes::{match_route, parse_routes};
use super::snapshot::LaravelSnapshot;

pub fn compute_extra_imports(
    snapshot: &LaravelSnapshot,
    root: &Path,
) -> FxHashMap<String, Vec<ExtraImport>> {
    let mut imports = FxHashMap::default();

    collect_route_controller_imports(snapshot, root, &mut imports);
    collect_model_relationship_imports(snapshot, root, &mut imports);
    collect_blade_imports(snapshot, root, &mut imports);
    collect_facade_imports(root, &mut imports);
    if snapshot.has_inertia {
        collect_inertia_imports(root, &mut imports);
    }
    collect_api_call_imports(snapshot, root, &mut imports);

    imports
}

fn collect_route_controller_imports(
    snapshot: &LaravelSnapshot,
    root: &Path,
    imports: &mut FxHashMap<String, Vec<ExtraImport>>,
) {
    if snapshot.routes.is_empty() {
        return;
    }

    for route_file in ["routes/web.php", "routes/api.php"] {
        let route_path = root.join(route_file);
        if !route_path.exists() {
            continue;
        }
        let content = match std::fs::read_to_string(&route_path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let parsed = parse_routes(&content);
        if parsed.is_empty() {
            continue;
        }

        let mut seen_controllers = FxHashSet::default();
        let mut route_imports = Vec::new();
        for capture in RE.route_controller_fqn.captures_iter(&content) {
            let fqn = capture[1].to_string();
            if seen_controllers.insert(fqn.clone()) {
                route_imports.push(ExtraImport {
                    specifier: fqn.clone(),
                    names: vec![fqn.rsplit('\\').next().unwrap_or(&fqn).to_string()],
                });
            }
        }

        for route in parsed {
            if seen_controllers.insert(route.controller.clone()) {
                route_imports.push(ExtraImport {
                    specifier: route.controller.clone(),
                    names: vec![route.controller.clone()],
                });
            }
        }

        if !route_imports.is_empty() {
            imports.insert(route_file.to_string(), route_imports);
        }
    }
}

fn collect_model_relationship_imports(
    snapshot: &LaravelSnapshot,
    root: &Path,
    imports: &mut FxHashMap<String, Vec<ExtraImport>>,
) {
    for (model_path, model_info) in &snapshot.models {
        if model_info.relationships.is_empty() {
            continue;
        }

        let mut seen = FxHashSet::default();
        let mut model_imports = Vec::new();
        if let Ok(content) = std::fs::read_to_string(root.join(model_path)) {
            for capture in RE.model_fqn_class.captures_iter(&content) {
                let fqn = capture[1].to_string();
                let short = fqn.rsplit('\\').next().unwrap_or(&fqn).to_string();
                if model_info.relationships.iter().any(|relationship| {
                    relationship.target == short || fqn.ends_with(&relationship.target)
                }) && seen.insert(fqn.clone())
                {
                    model_imports.push(ExtraImport {
                        specifier: fqn,
                        names: vec![short],
                    });
                }
            }
        }

        for relationship in &model_info.relationships {
            if seen.insert(relationship.target.clone()) {
                model_imports.push(ExtraImport {
                    specifier: relationship.target.clone(),
                    names: vec![relationship.target.clone()],
                });
            }
        }

        if !model_imports.is_empty() {
            imports.insert(model_path.clone(), model_imports);
        }
    }
}

fn collect_blade_imports(
    snapshot: &LaravelSnapshot,
    root: &Path,
    imports: &mut FxHashMap<String, Vec<ExtraImport>>,
) {
    for blade_path in &snapshot.blade_files {
        let full_path = root.join(blade_path);
        let content = match std::fs::read_to_string(&full_path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let blade_imports = extract_blade_imports(&content);
        if !blade_imports.is_empty() {
            imports.insert(
                blade_path.clone(),
                blade_imports
                    .into_iter()
                    .map(|specifier| ExtraImport {
                        specifier,
                        names: vec![],
                    })
                    .collect(),
            );
        }
    }
}

fn collect_facade_imports(root: &Path, imports: &mut FxHashMap<String, Vec<ExtraImport>>) {
    for dir in ["app/Http/Controllers", "app/Services", "app/Models"] {
        let full = root.join(dir);
        if full.exists() {
            scan_php_for_facades(&full, root, imports);
        }
    }
}

fn scan_php_for_facades(dir: &Path, root: &Path, imports: &mut FxHashMap<String, Vec<ExtraImport>>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_php_for_facades(&path, root, imports);
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "php") {
            continue;
        }
        let rel = match path.strip_prefix(root) {
            Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let facade_imports = extract_facade_imports(&content);
        if !facade_imports.is_empty() {
            let entry = imports.entry(rel).or_default();
            for facade_import in facade_imports {
                if !entry
                    .iter()
                    .any(|existing| existing.specifier == facade_import.specifier)
                {
                    entry.push(facade_import);
                }
            }
        }
    }
}

fn collect_inertia_imports(root: &Path, imports: &mut FxHashMap<String, Vec<ExtraImport>>) {
    let controllers_dir = root.join("app/Http/Controllers");
    if !controllers_dir.exists() {
        return;
    }

    let entries = match std::fs::read_dir(&controllers_dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "php") {
            continue;
        }
        let rel = match path.strip_prefix(root) {
            Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let pages = extract_inertia_renders(&content);
        let mut seen = FxHashSet::default();
        for page in pages {
            if let Some(resolved) = resolve_inertia_page(root, &page)
                && seen.insert(resolved.clone())
            {
                imports.entry(rel.clone()).or_default().push(ExtraImport {
                    specifier: resolved,
                    names: vec![],
                });
            }
        }
    }
}

fn collect_api_call_imports(
    snapshot: &LaravelSnapshot,
    root: &Path,
    imports: &mut FxHashMap<String, Vec<ExtraImport>>,
) {
    if snapshot.routes.is_empty() {
        return;
    }

    let js_dir = root.join("resources/js");
    if js_dir.exists() {
        scan_js_for_api_calls(&js_dir, root, &snapshot.routes, imports);
    }
}

fn scan_js_for_api_calls(
    dir: &Path,
    root: &Path,
    routes: &[super::types::RouteInfo],
    imports: &mut FxHashMap<String, Vec<ExtraImport>>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_js_for_api_calls(&path, root, routes, imports);
            continue;
        }
        let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        if !["js", "ts", "jsx", "tsx", "vue"].contains(&ext) {
            continue;
        }
        let rel = match path.strip_prefix(root) {
            Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        for url in extract_api_calls(&content) {
            if let Some(route) = match_route(&url, routes) {
                let controller_path = format!("app/Http/Controllers/{}.php", route.controller);
                let entry = imports.entry(rel.clone()).or_default();
                if !entry
                    .iter()
                    .any(|existing| existing.specifier == controller_path)
                {
                    entry.push(ExtraImport {
                        specifier: controller_path,
                        names: vec![route.controller.clone()],
                    });
                }
            }
        }
    }
}
