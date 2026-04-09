use std::collections::HashMap;
use std::path::Path;

use crate::framework::{ExtraImport, FileContext};

use super::provider::NuxtStats;
use super::render;
use super::routing::{NuxtApiRoute, nuxt_api_route_from_path, nuxt_route_from_path};

#[derive(Debug, Clone, Default)]
pub struct NuxtSnapshot {
    pub(crate) page_routes: HashMap<String, String>,
    pub(crate) api_routes: HashMap<String, NuxtApiRoute>,
    pub(crate) auto_import_map: HashMap<String, String>,
    pub(crate) extra_imports: HashMap<String, Vec<ExtraImport>>,
}

impl NuxtSnapshot {
    pub fn load(root: &Path) -> Self {
        let mut snapshot = Self::default();
        snapshot.scan_pages(root);
        snapshot.scan_server_api(root);
        snapshot.scan_auto_imports(root);
        snapshot.compute_extra_imports(root);
        snapshot
    }

    pub fn stats(&self) -> NuxtStats {
        NuxtStats {
            page_routes: self.page_routes.len(),
            api_routes: self.api_routes.len(),
            auto_import_symbols: self.auto_import_map.len(),
        }
    }

    pub fn file_context(&self, rel_path: &str) -> Option<FileContext> {
        render::render_file_context(self, rel_path)
    }

    pub fn metadata(&self) -> HashMap<String, serde_json::Value> {
        render::metadata(self)
    }

    pub fn extra_imports(&self) -> HashMap<String, Vec<ExtraImport>> {
        self.extra_imports.clone()
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
            Ok(entries) => entries,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.scan_pages_recursive(&path, root);
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "vue") {
                continue;
            }
            let rel = match path.strip_prefix(root) {
                Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
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
            Ok(entries) => entries,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.scan_api_recursive(&path, root);
                continue;
            }
            let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
            if ext != "ts" && ext != "js" {
                continue;
            }
            let rel = match path.strip_prefix(root) {
                Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            self.api_routes
                .insert(rel.clone(), nuxt_api_route_from_path(&rel));
        }
    }

    fn scan_auto_imports(&mut self, root: &Path) {
        for dir_name in ["composables", "utils"] {
            let dir = root.join(dir_name);
            if !dir.exists() {
                continue;
            }
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    continue;
                }
                let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
                if ext != "ts" && ext != "js" {
                    continue;
                }
                let stem = path
                    .file_stem()
                    .and_then(|file_stem| file_stem.to_str())
                    .unwrap_or("")
                    .to_string();
                if stem.is_empty() {
                    continue;
                }
                let rel = match path.strip_prefix(root) {
                    Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
                    Err(_) => continue,
                };
                self.auto_import_map.insert(stem, rel);
            }
        }
    }

    fn compute_extra_imports(&mut self, root: &Path) {
        for page_rel in self.page_routes.keys() {
            let page_path = root.join(page_rel);
            let content = match std::fs::read_to_string(&page_path) {
                Ok(content) => content,
                Err(_) => continue,
            };

            let mut page_imports = Vec::new();
            for (symbol_name, source_file) in &self.auto_import_map {
                if page_rel == source_file {
                    continue;
                }
                let pattern = format!("{symbol_name}(");
                if content.contains(&pattern) {
                    page_imports.push(ExtraImport {
                        specifier: source_file.clone(),
                        names: vec![symbol_name.clone()],
                    });
                }
            }
            if !page_imports.is_empty() {
                self.extra_imports.insert(page_rel.clone(), page_imports);
            }
        }
    }
}
