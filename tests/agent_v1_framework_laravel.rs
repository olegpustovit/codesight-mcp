//! Integration tests: agent v1 framework laravel.

use codesight_language_support::frameworks::FrameworkProvider;
use codesight_language_support::languages::php::laravel::*;

// ── _guess_table ────────────────────────────────────────────────────────────
#[test]
fn guess_table_regular_noun() {
    assert_eq!(guess_table("User"), "users");
}
#[test]
fn guess_table_y_suffix() {
    assert_eq!(guess_table("Category"), "categories");
}
#[test]
fn guess_table_already_plural_pattern() {
    assert_eq!(guess_table("Status"), "statuses");
}
#[test]
fn guess_table_camel_case() {
    assert_eq!(guess_table("BlogPost"), "blog_posts");
}

// ── _parse_migration ────────────────────────────────────────────────────────

fn sample_migration() -> &'static str {
    r#"<?php
Schema::create('users', function (Blueprint $table) {
    $table->id();
    $table->string('name');
    $table->string('email')->unique();
    $table->timestamp('email_verified_at')->nullable();
    $table->string('password');
    $table->timestamps();
});
"#
}
#[test]
fn parse_migration_parses_table_name() {
    let info = parse_migration(sample_migration()).unwrap();
    assert_eq!(info.table, "users");
}
#[test]
fn parse_migration_parses_column_names() {
    let info = parse_migration(sample_migration()).unwrap();
    assert!(info.columns.contains_key("name"));
    assert!(info.columns.contains_key("email"));
}
#[test]
fn parse_migration_unique_modifier() {
    let info = parse_migration(sample_migration()).unwrap();
    assert!(info.columns["email"].contains(&"unique".to_string()));
}
#[test]
fn parse_migration_nullable_modifier() {
    let info = parse_migration(sample_migration()).unwrap();
    assert!(info.columns["email_verified_at"].contains(&"nullable".to_string()));
}
#[test]
fn parse_migration_skips_timestamps_helper() {
    let info = parse_migration(sample_migration()).unwrap();
    assert!(!info.columns.contains_key("timestamps"));
}
#[test]
fn parse_migration_no_schema_returns_none() {
    assert!(parse_migration("<?php echo 'hello';").is_none());
}

/// `Schema::create` with whitespace/newlines before `(` — tree-sitter + narrow regex gap-fill.
#[test]
fn parse_migration_schema_create_multiline_open_paren() {
    let src = r#"<?php
Schema::create(
    'items',
    function (Blueprint $table) {
        $table->id();
    }
);
"#;
    let info = parse_migration(src).expect("migration");
    assert_eq!(info.table, "items");
}

// ── _parse_model ────────────────────────────────────────────────────────────

fn sample_model() -> &'static str {
    r#"<?php
class User extends Model {
    protected $fillable = ['name', 'email', 'password'];

    public function posts() {
        return $this->hasMany(Post::class);
    }

    public function team() {
        return $this->belongsTo(Team::class);
    }

    public function scopeActive($query) {
        return $query->where('active', true);
    }
}
"#
}
#[test]
fn parse_model_relationships() {
    let info = parse_model(sample_model());
    let kinds: Vec<&str> = info.relationships.iter().map(|r| r.kind.as_str()).collect();
    assert!(kinds.contains(&"hasMany"));
    assert!(kinds.contains(&"belongsTo"));
}
#[test]
fn parse_model_fillable() {
    let info = parse_model(sample_model());
    assert!(info.fillable.contains(&"name".to_string()));
    assert!(info.fillable.contains(&"email".to_string()));
}
#[test]
fn parse_model_scopes() {
    let info = parse_model(sample_model());
    assert!(info.scopes.contains(&"Active".to_string()));
}
#[test]
fn parse_model_no_relationships() {
    let info = parse_model("<?php class Empty extends Model {}");
    assert!(info.relationships.is_empty());
}

// ── _parse_routes ───────────────────────────────────────────────────────────

fn sample_routes() -> &'static str {
    r#"<?php
Route::get('/users', [UserController::class, 'index'])->name('users.index');
Route::post('/users', [UserController::class, 'store'])->name('users.store');
Route::get('/users/{user}', [UserController::class, 'show']);
"#
}
#[test]
fn parse_routes_array_style() {
    let routes = parse_routes(sample_routes());
    assert_eq!(routes.len(), 3);
    let verbs: Vec<&str> = routes.iter().map(|r| r.verb.as_str()).collect();
    assert!(verbs.contains(&"GET"));
    assert!(verbs.contains(&"POST"));
}
#[test]
fn parse_routes_names_extracted() {
    let routes = parse_routes(sample_routes());
    let names: Vec<&str> = routes.iter().filter_map(|r| r.name.as_deref()).collect();
    assert!(names.contains(&"users.index"));
    assert!(names.contains(&"users.store"));
}
#[test]
fn parse_routes_controller_extracted() {
    let routes = parse_routes(sample_routes());
    assert!(routes.iter().all(|r| r.controller == "UserController"));
}
#[test]
fn parse_routes_missing_dir_returns_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let routes = parse_routes_dir(&tmp.path().join("nonexistent"));
    assert!(routes.is_empty());
}

// ── LaravelContextProvider detection ────────────────────────────────────────

fn make_laravel_project(root: &std::path::Path) {
    std::fs::write(root.join("artisan"), "#!/usr/bin/env php").unwrap();
    std::fs::write(
        root.join("composer.json"),
        r#"{"require": {"laravel/framework": "^10.0"}}"#,
    )
    .unwrap();
}
#[test]
fn laravel_detect_project() {
    let tmp = tempfile::tempdir().unwrap();
    make_laravel_project(tmp.path());
    let provider = LaravelProvider::new();
    assert!(provider.detect(tmp.path()));
}
#[test]
fn laravel_detect_no_artisan() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("composer.json"),
        r#"{"require": {"laravel/framework": "^10.0"}}"#,
    )
    .unwrap();
    let provider = LaravelProvider::new();
    assert!(!provider.detect(tmp.path()));
}
#[test]
fn laravel_detect_wrong_framework() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("artisan"), "").unwrap();
    std::fs::write(
        tmp.path().join("composer.json"),
        r#"{"require": {"symfony/framework-bundle": "^6.0"}}"#,
    )
    .unwrap();
    let provider = LaravelProvider::new();
    assert!(!provider.detect(tmp.path()));
}
#[test]
fn laravel_load_model_enrichment() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);
    std::fs::create_dir_all(root.join("app/Models")).unwrap();
    std::fs::write(
        root.join("app/Models/User.php"),
        "<?php class User extends Model { protected $fillable = ['name']; }",
    )
    .unwrap();

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();

    let ctx = provider.get_file_context("app/Models/User.php").unwrap();
    assert!(ctx.tags.contains(&"eloquent-model".to_string()));
    let desc_lower = ctx.description.to_lowercase();
    assert!(desc_lower.contains("user") || desc_lower.contains("users"));
}
#[test]
fn laravel_load_migration_columns() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);
    std::fs::create_dir_all(root.join("database/migrations")).unwrap();
    std::fs::write(
        root.join("database/migrations/2024_01_01_create_users_table.php"),
        sample_migration(),
    )
    .unwrap();

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();

    let meta = provider.get_metadata();
    let columns = meta["laravel_columns"].as_object().unwrap();
    let users_cols = columns["users"].as_array().unwrap();
    let col_names: Vec<&str> = users_cols.iter().filter_map(|v| v.as_str()).collect();
    assert!(col_names.contains(&"email"));
}
#[test]
fn laravel_load_route_enriches_controller() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);
    std::fs::create_dir_all(root.join("routes")).unwrap();
    std::fs::write(root.join("routes/web.php"), sample_routes()).unwrap();

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();

    let ctx = provider
        .get_file_context("app/Http/Controllers/UserController.php")
        .unwrap();
    assert!(ctx.tags.contains(&"controller".to_string()));
}

#[test]
fn laravel_snapshot_exposes_context_and_imports_without_provider() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);
    std::fs::create_dir_all(root.join("routes")).unwrap();
    std::fs::create_dir_all(root.join("app/Http/Controllers")).unwrap();
    std::fs::write(root.join("routes/web.php"), sample_routes()).unwrap();
    std::fs::write(
        root.join("app/Http/Controllers/UserController.php"),
        "<?php",
    )
    .unwrap();

    let mut snapshot = LaravelSnapshot::load(root);
    snapshot.extra_imports = compute_extra_imports(&snapshot, root);
    let ctx = render_file_context(&snapshot, "app/Http/Controllers/UserController.php")
        .expect("controller context");
    assert!(ctx.tags.contains(&"controller".to_string()));
    let imports = snapshot.extra_imports();
    assert!(imports.contains_key("routes/web.php"));
    assert_eq!(snapshot.stats().routes, 3);
}

#[test]
fn laravel_stats() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);
    std::fs::create_dir_all(root.join("app/Models")).unwrap();
    std::fs::write(
        root.join("app/Models/User.php"),
        "<?php class User extends Model {}",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("database/migrations")).unwrap();
    std::fs::write(
        root.join("database/migrations/001_users.php"),
        sample_migration(),
    )
    .unwrap();

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();
    let stats = provider.stats();
    assert!(stats.models >= 1);
    assert!(stats.migration_tables >= 1);
}
#[test]
fn laravel_blade_context_returned() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);
    std::fs::create_dir_all(root.join("resources/views")).unwrap();
    std::fs::write(
        root.join("resources/views/welcome.blade.php"),
        "<h1>Hello</h1>",
    )
    .unwrap();

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();

    let ctx = provider
        .get_file_context("resources/views/welcome.blade.php")
        .unwrap();
    assert!(ctx.tags.contains(&"blade".to_string()));
}
#[test]
fn laravel_non_laravel_file_returns_none() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();

    assert!(provider.get_file_context("src/something.go").is_none());
}

// ── Blade dot-notation helpers ──────────────────────────────────────────────
#[test]
fn blade_dot_to_path_simple_view() {
    assert_eq!(
        blade_dot_to_path("welcome"),
        "resources/views/welcome.blade.php"
    );
}
#[test]
fn blade_dot_to_path_nested_view() {
    assert_eq!(
        blade_dot_to_path("layouts.app"),
        "resources/views/layouts/app.blade.php"
    );
}
#[test]
fn blade_dot_to_path_deeply_nested() {
    assert_eq!(
        blade_dot_to_path("admin.users.index"),
        "resources/views/admin/users/index.blade.php"
    );
}
#[test]
fn blade_component_to_path_simple() {
    assert_eq!(
        blade_component_to_path("alert"),
        "resources/views/components/alert.blade.php"
    );
}
#[test]
fn blade_component_to_path_nested() {
    assert_eq!(
        blade_component_to_path("forms.input"),
        "resources/views/components/forms/input.blade.php"
    );
}
#[test]
fn blade_component_to_path_hyphenated() {
    assert_eq!(
        blade_component_to_path("nav-link"),
        "resources/views/components/nav-link.blade.php"
    );
}

// ── Blade import extraction ─────────────────────────────────────────────────
#[test]
fn blade_imports_extracted() {
    let content = r#"
@extends('layouts.app')
@include('partials.header')
@includeWhen($show, 'partials.sidebar')
<x-alert />
<x-forms.input />
@includeFirst(['partials.primary', 'partials.fallback'])
"#;
    let imports = extract_blade_imports(content);
    assert!(imports.contains(&"resources/views/layouts/app.blade.php".to_string()));
    assert!(imports.contains(&"resources/views/partials/header.blade.php".to_string()));
    assert!(imports.contains(&"resources/views/partials/sidebar.blade.php".to_string()));
    assert!(imports.contains(&"resources/views/components/alert.blade.php".to_string()));
    assert!(imports.contains(&"resources/views/components/forms/input.blade.php".to_string()));
    // includeFirst: only first candidate
    assert!(imports.contains(&"resources/views/partials/primary.blade.php".to_string()));
    // Fallback should NOT be included
    assert!(!imports.contains(&"resources/views/partials/fallback.blade.php".to_string()));
}
#[test]
fn blade_layout_has_no_imports() {
    let content = "@yield('content')";
    let imports = extract_blade_imports(content);
    assert!(imports.is_empty());
}
#[test]
fn blade_no_views_dir_is_safe() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();

    // No blade files scanned
    let imports = provider.get_extra_imports().unwrap();
    let blade_imports: Vec<_> = imports
        .iter()
        .filter(|(k, _)| k.contains("blade"))
        .collect();
    assert!(blade_imports.is_empty());
}

// ── Route -> Controller import extraction ───────────────────────────────────
#[test]
fn route_imports_controller() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);
    std::fs::create_dir_all(root.join("routes")).unwrap();
    std::fs::write(root.join("routes/web.php"), sample_routes()).unwrap();

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();

    let imports = provider.get_extra_imports().unwrap();
    let route_imports = imports.get("routes/web.php").unwrap();
    let specifiers: Vec<&str> = route_imports.iter().map(|i| i.specifier.as_str()).collect();
    assert!(specifiers.iter().any(|s| s.contains("UserController")));
}
#[test]
fn route_imports_deduplicated() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);
    std::fs::create_dir_all(root.join("routes")).unwrap();
    std::fs::write(root.join("routes/web.php"), sample_routes()).unwrap();

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();

    let imports = provider.get_extra_imports().unwrap();
    let route_imports = imports.get("routes/web.php").unwrap();
    let user_ctrl_count = route_imports
        .iter()
        .filter(|i| i.specifier.contains("UserController"))
        .count();
    assert_eq!(
        user_ctrl_count, 1,
        "3 routes to same controller => 1 import"
    );
}
#[test]
fn route_imports_inline_fqn_controller() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);
    std::fs::create_dir_all(root.join("routes")).unwrap();
    std::fs::write(
        root.join("routes/web.php"),
        r#"Route::get('/users', [\App\Http\Controllers\UserController::class, 'index']);"#,
    )
    .unwrap();

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();

    let imports = provider.get_extra_imports().unwrap();
    let route_imports = imports.get("routes/web.php").unwrap();
    assert!(route_imports.iter().any(|i| {
        i.specifier
            .contains(r"\App\Http\Controllers\UserController")
    }));
}
#[test]
fn route_imports_multiple_controllers() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);
    std::fs::create_dir_all(root.join("routes")).unwrap();
    std::fs::write(
        root.join("routes/web.php"),
        r#"
Route::get('/users', [UserController::class, 'index']);
Route::get('/posts', [PostController::class, 'index']);
"#,
    )
    .unwrap();

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();

    let imports = provider.get_extra_imports().unwrap();
    let route_imports = imports.get("routes/web.php").unwrap();
    let specifiers: Vec<&str> = route_imports.iter().map(|i| i.specifier.as_str()).collect();
    assert!(specifiers.iter().any(|s| s.contains("UserController")));
    assert!(specifiers.iter().any(|s| s.contains("PostController")));
}

// ── Eloquent relationship -> import edges ───────────────────────────────────
#[test]
fn eloquent_relationship_creates_import_edge() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);
    std::fs::create_dir_all(root.join("app/Models")).unwrap();
    std::fs::write(root.join("app/Models/User.php"), sample_model()).unwrap();

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();

    let imports = provider.get_extra_imports().unwrap();
    let model_imports = imports.get("app/Models/User.php").unwrap();
    let specifiers: Vec<&str> = model_imports.iter().map(|i| i.specifier.as_str()).collect();
    assert!(specifiers.iter().any(|s| s.contains("Post")));
    assert!(specifiers.iter().any(|s| s.contains("Team")));
}
#[test]
fn eloquent_relationship_with_fqn() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);
    std::fs::create_dir_all(root.join("app/Models")).unwrap();
    std::fs::write(
        root.join("app/Models/Order.php"),
        r#"<?php class Order extends Model {
    public function items() {
        return $this->hasMany(\App\Models\OrderItem::class);
    }
}"#,
    )
    .unwrap();

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();

    let imports = provider.get_extra_imports().unwrap();
    let model_imports = imports.get("app/Models/Order.php").unwrap();
    assert!(
        model_imports
            .iter()
            .any(|i| i.specifier.contains(r"\App\Models\OrderItem"))
    );
}
#[test]
fn eloquent_no_relationships_no_imports() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);
    std::fs::create_dir_all(root.join("app/Models")).unwrap();
    std::fs::write(
        root.join("app/Models/Setting.php"),
        "<?php class Setting extends Model {}",
    )
    .unwrap();

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();

    let imports = provider.get_extra_imports().unwrap();
    assert!(!imports.contains_key("app/Models/Setting.php"));
}
#[test]
fn eloquent_deduplicated_relationships() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);
    std::fs::create_dir_all(root.join("app/Models")).unwrap();
    std::fs::write(
        root.join("app/Models/Author.php"),
        r#"<?php class Author extends Model {
    public function published() { return $this->hasMany(Post::class); }
    public function draft() { return $this->hasOne(Post::class); }
}"#,
    )
    .unwrap();

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();

    let imports = provider.get_extra_imports().unwrap();
    let model_imports = imports.get("app/Models/Author.php").unwrap();
    let post_count = model_imports
        .iter()
        .filter(|i| i.specifier.contains("Post"))
        .count();
    assert_eq!(post_count, 1);
}

// ── Facade resolution -> import edges ───────────────────────────────────────
#[test]
fn facade_call_creates_import() {
    let imports = extract_facade_imports(
        r#"
Cache::get('key');
DB::table('users');
Log::info('message');
"#,
    );
    let specifiers: Vec<&str> = imports.iter().map(|i| i.specifier.as_str()).collect();
    assert!(specifiers.iter().any(|s| s.contains("CacheManager")));
    assert!(specifiers.iter().any(|s| s.contains("DatabaseManager")));
    assert!(specifiers.iter().any(|s| s.contains("LogManager")));
}
#[test]
fn facade_class_reference_not_matched() {
    let imports = extract_facade_imports("Cache::class");
    assert!(imports.is_empty());
}
#[test]
fn facade_non_facade_static_call_ignored() {
    let imports = extract_facade_imports("MyCustomClass::doSomething()");
    assert!(imports.is_empty());
}
#[test]
fn facade_deduplicated_in_same_file() {
    let imports = extract_facade_imports("Cache::get('a');\nCache::put('b', 'c');");
    let cache_count = imports
        .iter()
        .filter(|i| i.names.contains(&"Cache".to_string()))
        .count();
    assert_eq!(cache_count, 1);
}
#[test]
fn facade_stats_include_import_counts() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);
    std::fs::create_dir_all(root.join("routes")).unwrap();
    std::fs::write(root.join("routes/web.php"), sample_routes()).unwrap();
    std::fs::create_dir_all(root.join("resources/views")).unwrap();
    std::fs::write(
        root.join("resources/views/home.blade.php"),
        "@extends('layouts.app')",
    )
    .unwrap();

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();
    let stats = provider.stats();
    assert!(stats.route_files_with_imports >= 1);
    assert!(stats.blade_files_with_imports >= 1);
}

// ── Inertia.js cross-language bridge ────────────────────────────────────────
#[test]
fn inertia_render_creates_import() {
    let renders = extract_inertia_renders("Inertia::render('Users/Index')");
    assert_eq!(renders, vec!["Users/Index"]);
}
#[test]
fn inertia_helper_function() {
    let renders = extract_inertia_renders("return inertia('Dashboard')");
    assert_eq!(renders, vec!["Dashboard"]);
}
#[test]
fn inertia_tsx_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join("resources/js/Pages")).unwrap();
    std::fs::write(root.join("resources/js/Pages/Settings.tsx"), "").unwrap();

    let resolved = resolve_inertia_page(root, "Settings").unwrap();
    assert!(resolved.ends_with("Settings.tsx"));
}
#[test]
fn inertia_no_dependency_no_imports() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);
    std::fs::create_dir_all(root.join("app/Http/Controllers")).unwrap();
    std::fs::write(
        root.join("app/Http/Controllers/HomeController.php"),
        "<?php Inertia::render('Home')",
    )
    .unwrap();

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();

    // No inertia dependency => no Inertia page imports
    let imports = provider.get_extra_imports().unwrap();
    let ctrl_imports = imports.get("app/Http/Controllers/HomeController.php");
    let has_inertia = ctrl_imports
        .map(|imps| imps.iter().any(|i| i.specifier.contains("Pages")))
        .unwrap_or(false);
    assert!(
        !has_inertia,
        "should not have Inertia imports without dependency"
    );
}
#[test]
fn inertia_detected_via_middleware() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);
    std::fs::create_dir_all(root.join("app/Http/Middleware")).unwrap();
    std::fs::write(
        root.join("app/Http/Middleware/HandleInertiaRequests.php"),
        "<?php class HandleInertiaRequests {}",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("app/Http/Controllers")).unwrap();
    std::fs::write(
        root.join("app/Http/Controllers/TestController.php"),
        "<?php Inertia::render('Dashboard')",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("resources/js/Pages")).unwrap();
    std::fs::write(root.join("resources/js/Pages/Dashboard.vue"), "").unwrap();

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();

    let imports = provider.get_extra_imports().unwrap();
    let ctrl_imports = imports
        .get("app/Http/Controllers/TestController.php")
        .unwrap();
    assert!(
        ctrl_imports
            .iter()
            .any(|i| i.specifier.contains("Dashboard"))
    );
}
#[test]
fn inertia_multiple_renders_deduplicated() {
    let renders = extract_inertia_renders("Inertia::render('Home')\nInertia::render('Home')");
    assert_eq!(renders.len(), 1);
}
#[test]
fn inertia_page_with_extension_not_duplicated() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join("resources/js/Pages")).unwrap();
    std::fs::write(root.join("resources/js/Pages/Home.vue"), "").unwrap();

    let resolved = resolve_inertia_page(root, "Home.vue");
    assert!(resolved.is_some());
    let path = resolved.unwrap();
    // Should NOT produce Home.vue.vue
    assert!(!path.ends_with(".vue.vue"));
}

// ── fetch/axios API call -> route matching ──────────────────────────────────
#[test]
fn api_call_fetch_matches_route() {
    let urls = extract_api_calls("fetch('/api/users')");
    assert!(urls.contains(&"/api/users".to_string()));
}
#[test]
fn api_call_axios_matches_route() {
    let urls = extract_api_calls("axios.post('/api/orders')");
    assert!(urls.contains(&"/api/orders".to_string()));
}
#[test]
fn api_call_dynamic_segment_matched() {
    let routes = vec![RouteInfo {
        verb: "GET".into(),
        uri: "/api/users/{user}".into(),
        controller: "UserController".into(),
        action: "show".into(),
        name: None,
    }];
    let matched = match_route("/api/users/42", &routes);
    assert!(matched.is_some());
    assert_eq!(matched.unwrap().controller, "UserController");
}
#[test]
fn api_call_no_routes_no_imports() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_laravel_project(root);

    let mut provider = LaravelProvider::new();
    provider.load(root).unwrap();

    // No routes => no API call matching
    let imports = provider.get_extra_imports().unwrap();
    let api_imports: Vec<_> = imports
        .iter()
        .filter(|(k, _)| k.contains("resources/js"))
        .collect();
    assert!(api_imports.is_empty());
}
#[test]
fn api_call_non_api_fetch_ignored() {
    let urls = extract_api_calls("fetch('https://external.api.com/data')");
    assert!(urls.is_empty());
}
