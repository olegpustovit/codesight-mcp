use std::collections::HashMap;
use std::path::Path;

use crate::frameworks::{ExtraImport, FileContext, rel_path, walk_files};

use super::migrations::parse_migration;
use super::models::{guess_table, parse_model};
use super::routes::parse_routes_dir;
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

#[derive(Debug, Clone, Default)]
pub struct LaravelSnapshot {
    pub(crate) models: HashMap<String, ModelInfo>,
    pub(crate) migrations: HashMap<String, MigrationInfo>,
    pub(crate) routes: Vec<RouteInfo>,
    pub(crate) blade_files: Vec<String>,
    pub(crate) file_contexts: HashMap<String, FileContext>,
    pub(crate) has_inertia: bool,
    pub extra_imports: HashMap<String, Vec<ExtraImport>>,
}

impl LaravelSnapshot {
    pub fn load(root: &Path) -> Self {
        let mut snapshot = Self::default();
        snapshot.detect_inertia(root);
        snapshot.scan_models(root);
        snapshot.scan_migrations(root);
        snapshot.scan_routes(root);
        snapshot.scan_blade_files(root);
        snapshot
    }

    pub fn stats(&self) -> LaravelStats {
        LaravelStats {
            models: self.models.len(),
            migration_tables: self.migrations.len(),
            routes: self.routes.len(),
            blade_files: self.blade_files.len(),
            route_files_with_imports: usize::from(!self.routes.is_empty()),
            blade_files_with_imports: self.blade_files.len(),
        }
    }

    pub fn extra_imports(&self) -> HashMap<String, Vec<ExtraImport>> {
        self.extra_imports.clone()
    }

    fn detect_inertia(&mut self, root: &Path) {
        if let Ok(content) = std::fs::read_to_string(root.join("composer.json"))
            && content.contains("inertiajs")
        {
            self.has_inertia = true;
            return;
        }
        if root
            .join("app/Http/Middleware/HandleInertiaRequests.php")
            .exists()
        {
            self.has_inertia = true;
        }
    }

    fn scan_models(&mut self, root: &Path) {
        let models_dir = root.join("app/Models");
        if !models_dir.exists() {
            return;
        }
        for path in walk_files(&models_dir) {
            if path.extension().is_none_or(|ext| ext != "php") {
                continue;
            }
            let Some(rel) = rel_path(&path, root) else {
                continue;
            };
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let model_info = parse_model(&content);
            let model_name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("")
                .to_string();
            let table = guess_table(&model_name);

            self.file_contexts.insert(
                rel.clone(),
                FileContext {
                    description: format!("Eloquent model {model_name} (table: {table})"),
                    tags: vec!["eloquent-model".into()],
                    properties: vec![("table".into(), table)],
                },
            );
            self.models.insert(rel, model_info);
        }
    }

    fn scan_migrations(&mut self, root: &Path) {
        let migrations_dir = root.join("database/migrations");
        if !migrations_dir.exists() {
            return;
        }
        for path in walk_files(&migrations_dir) {
            if path.extension().is_none_or(|ext| ext != "php") {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path)
                && let Some(migration) = parse_migration(&content)
            {
                self.migrations.insert(migration.table.clone(), migration);
            }
        }
    }

    fn scan_routes(&mut self, root: &Path) {
        self.routes = parse_routes_dir(&root.join("routes"));
        for route in &self.routes {
            let controller_path = format!("app/Http/Controllers/{}.php", route.controller);
            self.file_contexts
                .entry(controller_path)
                .or_insert_with(|| FileContext {
                    description: format!("Laravel controller: {}", route.controller),
                    tags: vec!["controller".into()],
                    properties: vec![],
                });
        }
    }

    fn scan_blade_files(&mut self, root: &Path) {
        let views_dir = root.join("resources/views");
        if !views_dir.exists() {
            return;
        }
        for path in walk_files(&views_dir) {
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !file_name.ends_with(".blade.php") {
                continue;
            }
            let Some(rel) = rel_path(&path, root) else {
                continue;
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
