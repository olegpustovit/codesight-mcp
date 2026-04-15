use std::collections::HashMap;
use std::path::Path;

use crate::{ExtraImport, rel_path, walk_files};

use super::routing::{NuxtApiRoute, nuxt_api_route_from_path, nuxt_route_from_path};

#[derive(Debug, Clone, Default)]
pub struct NuxtStats {
    pub page_routes: usize,
    pub api_routes: usize,
    pub auto_import_symbols: usize,
}

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

    pub fn extra_imports(&self) -> HashMap<String, Vec<ExtraImport>> {
        self.extra_imports.clone()
    }

    fn scan_pages(&mut self, root: &Path) {
        let pages_dir = root.join("pages");
        if !pages_dir.exists() {
            return;
        }
        for path in walk_files(&pages_dir) {
            if path.extension().is_none_or(|ext| ext != "vue") {
                continue;
            }
            if let Some(rel) = rel_path(&path, root) {
                let route = nuxt_route_from_path(&rel);
                self.page_routes.insert(rel, route);
            }
        }
    }

    fn scan_server_api(&mut self, root: &Path) {
        let api_dir = root.join("server").join("api");
        if !api_dir.exists() {
            return;
        }
        for path in walk_files(&api_dir) {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "ts" && ext != "js" {
                continue;
            }
            if let Some(rel) = rel_path(&path, root) {
                self.api_routes
                    .insert(rel.clone(), nuxt_api_route_from_path(&rel));
            }
        }
    }

    fn scan_auto_imports(&mut self, root: &Path) {
        for dir_name in ["composables", "utils"] {
            let dir = root.join(dir_name);
            if !dir.exists() {
                continue;
            }
            // Only scan top-level files (not recursive).
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
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
                if let Some(rel) = rel_path(&path, root) {
                    self.auto_import_map.insert(stem, rel);
                }
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
