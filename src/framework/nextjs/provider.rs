use std::collections::HashMap;
use std::path::Path;

use crate::framework::{FileContext, FrameworkProvider};

use super::middleware::extract_middleware_matcher;
use super::routing::{next_api_methods_from_content, next_route_from_path};

#[derive(Debug, Clone, Default)]
pub struct NextjsStats {
    pub page_routes: usize,
    pub api_routes: usize,
}

pub struct NextjsProvider {
    root: Option<std::path::PathBuf>,
    page_routes: HashMap<String, String>, // rel_path -> route
    api_routes: HashMap<String, Vec<String>>, // rel_path -> methods
    layouts: Vec<String>,
    middlewares: Vec<String>,
    errors: Vec<String>,
}

impl NextjsProvider {
    pub fn new() -> Self {
        Self {
            root: None,
            page_routes: HashMap::new(),
            api_routes: HashMap::new(),
            layouts: Vec::new(),
            middlewares: Vec::new(),
            errors: Vec::new(),
        }
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
        self.scan_dir_recursive(&app_dir, root);
    }

    fn scan_dir_recursive(&mut self, dir: &Path, root: &Path) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.scan_dir_recursive(&path, root);
                continue;
            }
            let rel = match path.strip_prefix(root) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let file_name = entry.file_name();
            let fname = file_name.to_string_lossy();
            let stem = fname.split('.').next().unwrap_or("");

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
                    self.layouts.push(rel);
                }
                "error" => {
                    self.errors.push(rel);
                }
                _ => {}
            }
        }

        // Check for middleware at root level
        let middleware_path = root.join("middleware.ts");
        if middleware_path.exists() {
            let rel = "middleware.ts".to_string();
            if !self.middlewares.contains(&rel) {
                self.middlewares.push(rel);
            }
        }
        let middleware_path_js = root.join("middleware.js");
        if middleware_path_js.exists() {
            let rel = "middleware.js".to_string();
            if !self.middlewares.contains(&rel) {
                self.middlewares.push(rel);
            }
        }
    }
}

impl Default for NextjsProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameworkProvider for NextjsProvider {
    fn name(&self) -> &str {
        "nextjs"
    }

    fn detect(&self, root: &Path) -> bool {
        root.join("next.config.js").exists()
            || root.join("next.config.mjs").exists()
            || root.join("next.config.ts").exists()
    }

    fn load(&mut self, root: &Path) -> anyhow::Result<()> {
        self.root = Some(root.to_path_buf());
        self.scan_app_dir(root);
        Ok(())
    }

    fn get_file_context(&self, rel_path: &str) -> Option<FileContext> {
        let normalized = rel_path.replace('\\', "/");

        // Page routes
        if let Some(route) = self.page_routes.get(&normalized) {
            return Some(FileContext {
                description: format!("Next.js page route: {route}"),
                tags: vec!["nextjs-page".into()],
                properties: vec![("route".into(), route.clone())],
            });
        }

        // API routes
        if let Some(methods) = self.api_routes.get(&normalized) {
            let route = next_route_from_path(&normalized);
            return Some(FileContext {
                description: format!("Next.js API route: {route}"),
                tags: vec!["nextjs-api".into()],
                properties: vec![
                    ("route".into(), route),
                    ("methods".into(), methods.join(", ")),
                ],
            });
        }

        // Layouts
        if self.layouts.contains(&normalized) {
            return Some(FileContext {
                description: "Next.js layout".into(),
                tags: vec!["nextjs-layout".into()],
                properties: vec![],
            });
        }

        // Error boundaries
        if self.errors.contains(&normalized) {
            return Some(FileContext {
                description: "Next.js error boundary".into(),
                tags: vec!["nextjs-error".into()],
                properties: vec![],
            });
        }

        // Middleware
        if self.middlewares.contains(&normalized) {
            let content = self
                .root
                .as_ref()
                .and_then(|r| std::fs::read_to_string(r.join(&normalized)).ok())
                .unwrap_or_default();
            let matcher = extract_middleware_matcher(&content);
            let mut props = vec![];
            if let Some(m) = matcher {
                props.push(("matcher".into(), m));
            }
            return Some(FileContext {
                description: "Next.js middleware".into(),
                tags: vec!["nextjs-middleware".into()],
                properties: props,
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
        meta.insert("nextjs_routes".into(), serde_json::Value::Array(routes));

        let api: Vec<serde_json::Value> = self
            .api_routes
            .iter()
            .map(|(path, methods)| {
                serde_json::Value::String(format!("{} {}", methods.join(","), path))
            })
            .collect();
        meta.insert("nextjs_api".into(), serde_json::Value::Array(api));

        meta
    }
}
