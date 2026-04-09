use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::framework::FileContext;

use super::middleware::extract_middleware_matcher;
use super::provider::NextjsStats;
use super::render;
use super::routing::{next_api_methods_from_content, next_route_from_path};

#[derive(Debug, Clone, Default)]
pub struct NextjsSnapshot {
    pub(crate) page_routes: HashMap<String, String>,
    pub(crate) api_routes: HashMap<String, Vec<String>>,
    pub(crate) layouts: HashSet<String>,
    pub(crate) middlewares: HashMap<String, Option<String>>,
    pub(crate) errors: HashSet<String>,
}

impl NextjsSnapshot {
    pub fn load(root: &Path) -> Self {
        let mut snapshot = Self::default();
        snapshot.scan_app_dir(root);
        snapshot.scan_root_middlewares(root);
        snapshot
    }

    pub fn stats(&self) -> NextjsStats {
        NextjsStats {
            page_routes: self.page_routes.len(),
            api_routes: self.api_routes.len(),
        }
    }

    pub fn file_context(&self, rel_path: &str) -> Option<FileContext> {
        render::render_file_context(self, rel_path)
    }

    pub fn metadata(&self) -> HashMap<String, serde_json::Value> {
        render::metadata(self)
    }

    fn scan_app_dir(&mut self, root: &Path) {
        let app_dir = root.join("app");
        if !app_dir.exists() {
            return;
        }
        self.scan_dir_recursive(&app_dir, root);
    }

    fn scan_dir_recursive(&mut self, dir: &Path, root: &Path) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.scan_dir_recursive(&path, root);
                continue;
            }
            let rel = match path.strip_prefix(root) {
                Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            let stem = file_name.split('.').next().unwrap_or("");

            match stem {
                "page" => {
                    let route = next_route_from_path(&rel);
                    self.page_routes.insert(rel, route);
                }
                "route" => {
                    let content = std::fs::read_to_string(&path).unwrap_or_default();
                    let methods = next_api_methods_from_content(&content);
                    self.api_routes.insert(rel, methods);
                }
                "layout" => {
                    self.layouts.insert(rel);
                }
                "error" => {
                    self.errors.insert(rel);
                }
                _ => {}
            }
        }
    }

    fn scan_root_middlewares(&mut self, root: &Path) {
        for middleware_name in ["middleware.ts", "middleware.js"] {
            let path = root.join(middleware_name);
            if !path.exists() {
                continue;
            }
            let matcher = std::fs::read_to_string(&path)
                .ok()
                .and_then(|content| extract_middleware_matcher(&content));
            self.middlewares
                .insert(middleware_name.to_string(), matcher);
        }
    }
}
