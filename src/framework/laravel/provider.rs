use std::collections::HashMap;
use std::path::Path;

use regex::Regex;

use crate::framework::{ExtraImport, FileContext, FrameworkProvider};

use super::api_client::extract_api_calls;
use super::blade::extract_blade_imports;
use super::facade::extract_facade_imports;
use super::inertia::{extract_inertia_renders, resolve_inertia_page};
use super::migrations::parse_migration;
use super::models::{guess_table, parse_model};
use super::routes::{match_route, parse_routes, parse_routes_dir};
use super::types::{MigrationInfo, ModelInfo, RouteInfo};

#[derive(Debug, Clone, Default)]
pub struct LaravelStats {
    pub models: usize,
    pub migration_tables: usize,
    pub routes: usize,
    pub blade_files: usize,
    pub route_files_with_imports: usize,
    pub blade_files_with_imports: usize,
}

pub struct LaravelProvider {
    root: Option<std::path::PathBuf>,
    models: HashMap<String, ModelInfo>, // rel_path -> ModelInfo
    migrations: HashMap<String, MigrationInfo>, // table_name -> MigrationInfo
    routes: Vec<RouteInfo>,
    blade_files: Vec<String>,
    file_contexts: HashMap<String, FileContext>,
    has_inertia: bool,
}

impl LaravelProvider {
    pub fn new() -> Self {
        Self {
            root: None,
            models: HashMap::new(),
            migrations: HashMap::new(),
            routes: Vec::new(),
            blade_files: Vec::new(),
            file_contexts: HashMap::new(),
            has_inertia: false,
        }
    }

    pub fn stats(&self) -> LaravelStats {
        LaravelStats {
            models: self.models.len(),
            migration_tables: self.migrations.len(),
            routes: self.routes.len(),
            blade_files: self.blade_files.len(),
            route_files_with_imports: if self.routes.is_empty() { 0 } else { 1 },
            blade_files_with_imports: self.blade_files.len(),
        }
    }

    fn detect_inertia(&mut self, root: &Path) {
        // Check composer.json for inertia
        if let Ok(content) = std::fs::read_to_string(root.join("composer.json")) {
            if content.contains("inertiajs") {
                self.has_inertia = true;
                return;
            }
        }
        // Check for HandleInertiaRequests middleware
        let middleware_path = root.join("app/Http/Middleware/HandleInertiaRequests.php");
        if middleware_path.exists() {
            self.has_inertia = true;
        }
    }

    fn scan_models(&mut self, root: &Path) {
        let models_dir = root.join("app/Models");
        if !models_dir.exists() {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(&models_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(true, |e| e != "php") {
                    continue;
                }
                let rel = match path.strip_prefix(root) {
                    Ok(r) => r.to_string_lossy().replace('\\', "/"),
                    Err(_) => continue,
                };
                let content = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let model_info = parse_model(&content);
                let model_name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                let table = guess_table(&model_name);

                self.file_contexts.insert(
                    rel.clone(),
                    FileContext {
                        description: format!("Eloquent model {model_name} (table: {table})"),
                        tags: vec!["eloquent-model".into()],
                        properties: vec![("table".into(), table.clone())],
                    },
                );
                self.models.insert(rel, model_info);
            }
        }
    }

    fn scan_migrations(&mut self, root: &Path) {
        let migrations_dir = root.join("database/migrations");
        if !migrations_dir.exists() {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(&migrations_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(true, |e| e != "php") {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some(migration) = parse_migration(&content) {
                        self.migrations.insert(migration.table.clone(), migration);
                    }
                }
            }
        }
    }

    fn scan_routes(&mut self, root: &Path) {
        let routes_dir = root.join("routes");
        self.routes = parse_routes_dir(&routes_dir);

        // Create file contexts for controllers referenced by routes
        for route in &self.routes {
            let controller_path = format!("app/Http/Controllers/{}.php", route.controller);
            if !self.file_contexts.contains_key(&controller_path) {
                self.file_contexts.insert(
                    controller_path,
                    FileContext {
                        description: format!("Laravel controller: {}", route.controller),
                        tags: vec!["controller".into()],
                        properties: vec![],
                    },
                );
            }
        }
    }

    fn scan_blade_files(&mut self, root: &Path) {
        let views_dir = root.join("resources/views");
        if !views_dir.exists() {
            return;
        }
        self.scan_blade_recursive(&views_dir, root);
    }

    fn scan_blade_recursive(&mut self, dir: &Path, root: &Path) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.scan_blade_recursive(&path, root);
                continue;
            }
            let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !fname.ends_with(".blade.php") {
                continue;
            }
            let rel = match path.strip_prefix(root) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            self.blade_files.push(rel.clone());
            self.file_contexts.insert(
                rel,
                FileContext {
                    description: "Blade template".into(),
                    tags: vec!["blade".into()],
                    properties: vec![],
                },
            );
        }
    }
}

impl Default for LaravelProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameworkProvider for LaravelProvider {
    fn name(&self) -> &str {
        "laravel"
    }

    fn detect(&self, root: &Path) -> bool {
        if !root.join("artisan").exists() {
            return false;
        }
        match std::fs::read_to_string(root.join("composer.json")) {
            Ok(content) => content.contains("laravel/framework"),
            Err(_) => false,
        }
    }

    fn load(&mut self, root: &Path) -> anyhow::Result<()> {
        self.root = Some(root.to_path_buf());
        self.detect_inertia(root);
        self.scan_models(root);
        self.scan_migrations(root);
        self.scan_routes(root);
        self.scan_blade_files(root);
        Ok(())
    }

    fn get_file_context(&self, rel_path: &str) -> Option<FileContext> {
        let normalized = rel_path.replace('\\', "/");
        self.file_contexts.get(&normalized).cloned()
    }

    fn get_metadata(&self) -> HashMap<String, serde_json::Value> {
        let mut meta = HashMap::new();

        // Columns by table
        let mut columns_map = serde_json::Map::new();
        for (table, migration) in &self.migrations {
            let cols: Vec<serde_json::Value> = migration
                .columns
                .keys()
                .map(|c| serde_json::Value::String(c.clone()))
                .collect();
            columns_map.insert(table.clone(), serde_json::Value::Array(cols));
        }
        meta.insert(
            "laravel_columns".into(),
            serde_json::Value::Object(columns_map),
        );

        meta
    }

    fn get_extra_imports(&self) -> anyhow::Result<HashMap<String, Vec<ExtraImport>>> {
        let root = match &self.root {
            Some(r) => r,
            None => return Ok(HashMap::new()),
        };

        let mut imports: HashMap<String, Vec<ExtraImport>> = HashMap::new();

        // Route -> Controller imports
        if !self.routes.is_empty() {
            let route_files = ["routes/web.php", "routes/api.php"];
            for rf in &route_files {
                let rf_path = root.join(rf);
                if !rf_path.exists() {
                    continue;
                }
                let content = match std::fs::read_to_string(&rf_path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let parsed = parse_routes(&content);
                if parsed.is_empty() {
                    continue;
                }

                let mut seen_controllers = std::collections::HashSet::new();
                let mut route_imports = Vec::new();

                // Also check for inline FQN controllers
                let fqn_re = Regex::new(r#"(\\[A-Za-z\\]+Controller)::class"#).unwrap();
                for cap in fqn_re.captures_iter(&content) {
                    let fqn = cap[1].to_string();
                    if seen_controllers.insert(fqn.clone()) {
                        route_imports.push(ExtraImport {
                            specifier: fqn.clone(),
                            names: vec![fqn.rsplit('\\').next().unwrap_or(&fqn).to_string()],
                        });
                    }
                }

                for route in &parsed {
                    if seen_controllers.insert(route.controller.clone()) {
                        route_imports.push(ExtraImport {
                            specifier: route.controller.clone(),
                            names: vec![route.controller.clone()],
                        });
                    }
                }

                if !route_imports.is_empty() {
                    imports.insert(rf.to_string(), route_imports);
                }
            }
        }

        // Model relationship imports
        for (model_path, model_info) in &self.models {
            if model_info.relationships.is_empty() {
                continue;
            }
            let mut seen = std::collections::HashSet::new();
            let mut model_imports = Vec::new();

            // Check full content for FQN relationships
            if let Ok(content) = std::fs::read_to_string(root.join(model_path)) {
                let fqn_re = Regex::new(r#"(\\[A-Za-z\\]+)::class"#).unwrap();
                for cap in fqn_re.captures_iter(&content) {
                    let fqn = cap[1].to_string();
                    let short = fqn.rsplit('\\').next().unwrap_or(&fqn).to_string();
                    // Only add if it's a relationship target
                    if model_info
                        .relationships
                        .iter()
                        .any(|r| r.target == short || fqn.ends_with(&r.target))
                    {
                        if seen.insert(fqn.clone()) {
                            model_imports.push(ExtraImport {
                                specifier: fqn,
                                names: vec![short],
                            });
                        }
                    }
                }
            }

            // Add short-name relationships not covered by FQN
            for rel in &model_info.relationships {
                if seen.insert(rel.target.clone()) {
                    model_imports.push(ExtraImport {
                        specifier: rel.target.clone(),
                        names: vec![rel.target.clone()],
                    });
                }
            }

            if !model_imports.is_empty() {
                imports.insert(model_path.clone(), model_imports);
            }
        }

        // Blade imports
        for blade_path in &self.blade_files {
            let full_path = root.join(blade_path);
            let content = match std::fs::read_to_string(&full_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let blade_imports = extract_blade_imports(&content);
            if !blade_imports.is_empty() {
                let extra: Vec<ExtraImport> = blade_imports
                    .into_iter()
                    .map(|spec| ExtraImport {
                        specifier: spec,
                        names: vec![],
                    })
                    .collect();
                imports.insert(blade_path.clone(), extra);
            }
        }

        // Facade imports from PHP files
        self.collect_facade_imports(root, &mut imports);

        // Inertia imports
        if self.has_inertia {
            self.collect_inertia_imports(root, &mut imports);
        }

        // API call matching
        self.collect_api_call_imports(root, &mut imports);

        Ok(imports)
    }
}

impl LaravelProvider {
    fn collect_facade_imports(&self, root: &Path, imports: &mut HashMap<String, Vec<ExtraImport>>) {
        // Scan PHP files for facade usage
        let dirs = ["app/Http/Controllers", "app/Services", "app/Models"];
        for dir in &dirs {
            let full = root.join(dir);
            if !full.exists() {
                continue;
            }
            self.scan_php_for_facades(&full, root, imports);
        }
    }

    fn scan_php_for_facades(
        &self,
        dir: &Path,
        root: &Path,
        imports: &mut HashMap<String, Vec<ExtraImport>>,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.scan_php_for_facades(&path, root, imports);
                continue;
            }
            if path.extension().map_or(true, |e| e != "php") {
                continue;
            }
            let rel = match path.strip_prefix(root) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let facade_imports = extract_facade_imports(&content);
            if !facade_imports.is_empty() {
                let entry = imports.entry(rel).or_default();
                for fi in facade_imports {
                    if !entry.iter().any(|e| e.specifier == fi.specifier) {
                        entry.push(fi);
                    }
                }
            }
        }
    }

    fn collect_inertia_imports(
        &self,
        root: &Path,
        imports: &mut HashMap<String, Vec<ExtraImport>>,
    ) {
        let controllers_dir = root.join("app/Http/Controllers");
        if !controllers_dir.exists() {
            return;
        }
        let entries = match std::fs::read_dir(&controllers_dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(true, |e| e != "php") {
                continue;
            }
            let rel = match path.strip_prefix(root) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let pages = extract_inertia_renders(&content);
            let mut seen = std::collections::HashSet::new();
            for page in pages {
                if let Some(resolved) = resolve_inertia_page(root, &page) {
                    if seen.insert(resolved.clone()) {
                        let entry = imports.entry(rel.clone()).or_default();
                        entry.push(ExtraImport {
                            specifier: resolved,
                            names: vec![],
                        });
                    }
                }
            }
        }
    }

    fn collect_api_call_imports(
        &self,
        root: &Path,
        imports: &mut HashMap<String, Vec<ExtraImport>>,
    ) {
        if self.routes.is_empty() {
            return;
        }

        // Scan JS/TS files in resources for fetch/axios calls
        let js_dir = root.join("resources/js");
        if !js_dir.exists() {
            return;
        }
        self.scan_js_for_api_calls(&js_dir, root, imports);
    }

    fn scan_js_for_api_calls(
        &self,
        dir: &Path,
        root: &Path,
        imports: &mut HashMap<String, Vec<ExtraImport>>,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.scan_js_for_api_calls(&path, root, imports);
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !["js", "ts", "jsx", "tsx", "vue"].contains(&ext) {
                continue;
            }
            let rel = match path.strip_prefix(root) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let urls = extract_api_calls(&content);
            for url in urls {
                if let Some(route) = match_route(&url, &self.routes) {
                    let controller_path = format!("app/Http/Controllers/{}.php", route.controller);
                    let entry = imports.entry(rel.clone()).or_default();
                    if !entry.iter().any(|e| e.specifier == controller_path) {
                        entry.push(ExtraImport {
                            specifier: controller_path,
                            names: vec![route.controller.clone()],
                        });
                    }
                }
            }
        }
    }
}
