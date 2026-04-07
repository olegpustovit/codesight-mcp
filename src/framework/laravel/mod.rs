//! Laravel framework context provider.
//!
//! Detects Laravel via `artisan` + `composer.json` with `laravel/framework`,
//! parses models, migrations, routes, Blade templates, Inertia.js bridges,
//! facade resolution, and fetch/axios API-call-to-route matching.

mod api_client;
mod blade;
mod facade;
mod inertia;
mod migrations;
mod models;
mod php_structure;
mod provider;
mod routes;
mod types;

pub use api_client::extract_api_calls;
pub use blade::{blade_component_to_path, blade_dot_to_path, extract_blade_imports};
pub use facade::extract_facade_imports;
pub use inertia::{extract_inertia_renders, resolve_inertia_page};
pub use migrations::parse_migration;
pub use models::{guess_table, parse_model};
pub use provider::{LaravelProvider, LaravelStats};
pub use routes::{match_route, parse_routes, parse_routes_dir};
pub use types::{MigrationInfo, ModelInfo, Relationship, RouteInfo};
