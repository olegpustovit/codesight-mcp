use std::collections::HashMap;
use std::path::Path;

use crate::framework::{ExtraImport, FileContext, FrameworkProvider};

use super::routing::{NuxtApiRoute, nuxt_api_route_from_path, nuxt_route_from_path};

#[derive(Debug, Clone, Default)]
pub struct NuxtStats {
    pub page_routes: usize,
    pub api_routes: usize,
    pub auto_import_symbols: usize,
}

pub struct NuxtProvider {
    root: Option<std::path::PathBuf>,
    page_routes: HashMap<String, String>, // rel_path -> route
    api_routes: HashMap<String, NuxtApiRoute>, // rel_path -> NuxtApiRoute
    auto_import_map: HashMap<String, String>, // symbol_name -> source file
}

impl NuxtProvider {
    pub fn new() -> Self {
        Self {
            root: None,
            page_routes: HashMap::new(),
            api_routes: HashMap::new(),
            auto_import_map: HashMap::new(),
        }
    }

    pub fn stats(&self) -> NuxtStats {
        NuxtStats {
            page_routes: self.page_routes.len(),
            api_routes: self.api_routes.len(),
            auto_import_symbols: self.auto_import_map.len(),
        }
    }

    fn scan_pages(&mut self, root: &Path) {
        let pages_dir = root.join("pages");
        if !pages_dir.exists() {
            return;
        }
        self.scan_pages_recursive(&pages_dir, root);
    }

    fn scan_pages_recursive(&mut self, dir: &Path, root: &Path) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.scan_pages_recursive(&path, root);
                continue;
            }
            if path.extension().is_none_or(|e| e != "vue") {
                continue;
            }
            let rel = match path.strip_prefix(root) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let route = nuxt_route_from_path(&rel);
            self.page_routes.insert(rel, route);
        }
    }

    fn scan_server_api(&mut self, root: &Path) {
        let api_dir = root.join("server").join("api");
        if !api_dir.exists() {
            return;
        }
        self.scan_api_recursive(&api_dir, root);
    }

    fn scan_api_recursive(&mut self, dir: &Path, root: &Path) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.scan_api_recursive(&path, root);
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "ts" && ext != "js" {
                continue;
            }
            let rel = match path.strip_prefix(root) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let api_route = nuxt_api_route_from_path(&rel);
            self.api_routes.insert(rel, api_route);
        }
    }

    fn scan_auto_imports(&mut self, root: &Path) {
        for dir_name in &["composables", "utils"] {
            let dir = root.join(dir_name);
            if !dir.exists() {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        continue;
                    }
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if ext != "ts" && ext != "js" {
                        continue;
                    }
                    let stem = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string();
                    if stem.is_empty() {
                        continue;
                    }
                    let rel = match path.strip_prefix(root) {
                        Ok(r) => r.to_string_lossy().replace('\\', "/"),
                        Err(_) => continue,
                    };
                    self.auto_import_map.insert(stem, rel);
                }
            }
        }
    }
}

impl Default for NuxtProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameworkProvider for NuxtProvider {
    fn name(&self) -> &str {
        "nuxt"
    }

    fn detect(&self, root: &Path) -> bool {
        root.join("nuxt.config.ts").exists() || root.join("nuxt.config.js").exists()
    }

    fn load(&mut self, root: &Path) -> anyhow::Result<()> {
        self.root = Some(root.to_path_buf());
        self.scan_pages(root);
        self.scan_server_api(root);
        self.scan_auto_imports(root);
        Ok(())
    }

    fn get_file_context(&self, rel_path: &str) -> Option<FileContext> {
        let normalized = rel_path.replace('\\', "/");

        // Page routes
        if let Some(route) = self.page_routes.get(&normalized) {
            return Some(FileContext {
                description: format!("Nuxt page route: {route}"),
                tags: vec!["nuxt-page".into()],
                properties: vec![("route".into(), route.clone())],
            });
        }

        // API routes
        if let Some(api) = self.api_routes.get(&normalized) {
            return Some(FileContext {
                description: format!("Nuxt API route: {} {}", api.method, api.endpoint),
                tags: vec!["nuxt-api".into()],
                properties: vec![
                    ("method".into(), api.method.clone()),
                    ("endpoint".into(), api.endpoint.clone()),
                ],
            });
        }

        None
    }

    fn get_metadata(&self) -> HashMap<String, serde_json::Value> {
        let mut meta = HashMap::new();

        let routes: Vec<serde_json::Value> = self
            .page_routes
            .values()
            .map(|r| serde_json::Value::String(r.clone()))
            .collect();
        meta.insert("nuxt_routes".into(), serde_json::Value::Array(routes));

        let api: Vec<serde_json::Value> = self
            .api_routes
            .values()
            .map(|a| serde_json::Value::String(format!("{} {}", a.method, a.endpoint)))
            .collect();
        meta.insert("nuxt_api".into(), serde_json::Value::Array(api));

        meta
    }

    fn get_extra_imports(&self) -> anyhow::Result<HashMap<String, Vec<ExtraImport>>> {
        let root = match &self.root {
            Some(r) => r,
            None => return Ok(HashMap::new()),
        };

        let mut imports: HashMap<String, Vec<ExtraImport>> = HashMap::new();

        // Scan all page files for auto-import usage
        for page_rel in self.page_routes.keys() {
            let page_path = root.join(page_rel);
            let content = match std::fs::read_to_string(&page_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let mut page_imports: Vec<ExtraImport> = Vec::new();
            for (symbol_name, source_file) in &self.auto_import_map {
                // Don't auto-import self
                if page_rel == source_file {
                    continue;
                }
                // Simple heuristic: check if the symbol name appears as a function call
                let pattern = format!("{}(", symbol_name);
                if content.contains(&pattern) {
                    page_imports.push(ExtraImport {
                        specifier: source_file.clone(),
                        names: vec![symbol_name.clone()],
                    });
                }
            }
            if !page_imports.is_empty() {
                imports.insert(page_rel.clone(), page_imports);
            }
        }

        Ok(imports)
    }
}
