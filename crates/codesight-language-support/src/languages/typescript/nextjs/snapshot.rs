use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::frameworks::{rel_path, walk_files};

use super::middleware::extract_middleware_matcher;
use super::routing::{next_api_methods_from_content, next_route_from_path};

#[derive(Debug, Clone, Default)]
pub struct NextjsStats {
    pub page_routes: usize,
    pub api_routes: usize,
}

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

    fn scan_app_dir(&mut self, root: &Path) {
        let app_dir = root.join("app");
        if !app_dir.exists() {
            return;
        }
        for path in walk_files(&app_dir) {
            let Some(rel) = rel_path(&path, root) else {
                continue;
            };
            let stem = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .split('.')
                .next()
                .unwrap_or("");

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
