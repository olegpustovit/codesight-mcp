//! Integration tests: agent v1 framework nuxt.

use codesight_framework::FrameworkProvider;
use codesight_framework::nuxt::*;

// ── _nuxt_route_from_path ───────────────────────────────────────────────────
#[test]
fn nuxt_route_from_path_index() {
    assert_eq!(nuxt_route_from_path("pages/index.vue"), "/");
}
#[test]
fn nuxt_route_from_path_simple() {
    assert_eq!(nuxt_route_from_path("pages/users/index.vue"), "/users");
}
#[test]
fn nuxt_route_from_path_non_index() {
    assert_eq!(nuxt_route_from_path("pages/about.vue"), "/about");
}
#[test]
fn nuxt_route_from_path_dynamic_param() {
    assert_eq!(nuxt_route_from_path("pages/users/[id].vue"), "/users/:id");
}
#[test]
fn nuxt_route_from_path_catch_all() {
    assert_eq!(
        nuxt_route_from_path("pages/posts/[...slug].vue"),
        "/posts/*"
    );
}
#[test]
fn nuxt_route_from_path_nested() {
    assert_eq!(
        nuxt_route_from_path("pages/admin/users/[id].vue"),
        "/admin/users/:id"
    );
}

// ── _nuxt_api_route_from_path ───────────────────────────────────────────────
#[test]
fn nuxt_api_route_simple_get() {
    let r = nuxt_api_route_from_path("server/api/users.get.ts");
    assert_eq!(r.method, "GET");
    assert_eq!(r.endpoint, "/api/users");
}
#[test]
fn nuxt_api_route_simple_post() {
    let r = nuxt_api_route_from_path("server/api/users.post.ts");
    assert_eq!(r.method, "POST");
    assert_eq!(r.endpoint, "/api/users");
}
#[test]
fn nuxt_api_route_dynamic_param() {
    let r = nuxt_api_route_from_path("server/api/users/[id].get.ts");
    assert_eq!(r.method, "GET");
    assert_eq!(r.endpoint, "/api/users/:id");
}
#[test]
fn nuxt_api_route_no_method_suffix() {
    let r = nuxt_api_route_from_path("server/api/health.ts");
    assert_eq!(r.method, "ALL");
    assert_eq!(r.endpoint, "/api/health");
}
#[test]
fn nuxt_api_route_nested_path() {
    let r = nuxt_api_route_from_path("server/api/admin/settings.put.ts");
    assert_eq!(r.method, "PUT");
    assert_eq!(r.endpoint, "/api/admin/settings");
}

// ── NuxtContextProvider detection ───────────────────────────────────────────
#[test]
fn nuxt_detect_ts_config() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("nuxt.config.ts"), "").unwrap();
    let provider = NuxtProvider::new();
    assert!(provider.detect(tmp.path()));
}
#[test]
fn nuxt_detect_js_config() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("nuxt.config.js"), "").unwrap();
    let provider = NuxtProvider::new();
    assert!(provider.detect(tmp.path()));
}
#[test]
fn nuxt_detect_no_config() {
    let tmp = tempfile::tempdir().unwrap();
    let provider = NuxtProvider::new();
    assert!(!provider.detect(tmp.path()));
}

// ── NuxtContextProvider page routing ────────────────────────────────────────
#[test]
fn nuxt_page_routing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("nuxt.config.ts"), "").unwrap();
    std::fs::create_dir_all(root.join("pages/users")).unwrap();
    std::fs::write(root.join("pages/index.vue"), "<template></template>").unwrap();
    std::fs::write(root.join("pages/users/[id].vue"), "<template></template>").unwrap();

    let mut provider = NuxtProvider::new();
    provider.load(root).unwrap();

    let ctx_index = provider.get_file_context("pages/index.vue").unwrap();
    assert!(ctx_index.tags.contains(&"nuxt-page".to_string()));
    assert!(
        ctx_index
            .properties
            .iter()
            .any(|(k, v)| k == "route" && v == "/")
    );

    let ctx_user = provider.get_file_context("pages/users/[id].vue").unwrap();
    assert!(ctx_user.tags.contains(&"nuxt-page".to_string()));
    assert!(
        ctx_user
            .properties
            .iter()
            .any(|(k, v)| k == "route" && v.contains(":id"))
    );
}

// ── NuxtContextProvider server API routes ───────────────────────────────────
#[test]
fn nuxt_server_api_routes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("nuxt.config.ts"), "").unwrap();
    std::fs::create_dir_all(root.join("server/api")).unwrap();
    std::fs::write(
        root.join("server/api/users.get.ts"),
        "export default defineEventHandler(() => [])",
    )
    .unwrap();

    let mut provider = NuxtProvider::new();
    provider.load(root).unwrap();

    let ctx = provider
        .get_file_context("server/api/users.get.ts")
        .unwrap();
    assert!(ctx.tags.contains(&"nuxt-api".to_string()));
    assert!(
        ctx.properties
            .iter()
            .any(|(k, v)| k == "method" && v == "GET")
    );
    assert!(
        ctx.properties
            .iter()
            .any(|(k, v)| k == "endpoint" && v == "/api/users")
    );

    let meta = provider.get_metadata();
    let api = meta["nuxt_api"].as_array().unwrap();
    assert!(
        api.iter()
            .any(|v| v.as_str().unwrap().contains("GET /api/users"))
    );
}

// ── NuxtContextProvider auto-import system ──────────────────────────────────
#[test]
fn nuxt_auto_import_composables() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("nuxt.config.ts"), "").unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("composables")).unwrap();
    std::fs::write(
        root.join("pages/index.vue"),
        "<script setup>\nconst auth = useAuth();\nconst cart = useCart();\n</script>",
    )
    .unwrap();
    std::fs::write(
        root.join("composables/useAuth.ts"),
        "export function useAuth() {}",
    )
    .unwrap();
    std::fs::write(
        root.join("composables/useCart.ts"),
        "export function useCart() {}",
    )
    .unwrap();

    let mut provider = NuxtProvider::new();
    provider.load(root).unwrap();

    let imports = provider.get_extra_imports().unwrap();
    let page_imports = imports.get("pages/index.vue").unwrap();
    let specifiers: Vec<&str> = page_imports.iter().map(|i| i.specifier.as_str()).collect();
    assert!(specifiers.contains(&"composables/useAuth.ts"));
    assert!(specifiers.contains(&"composables/useCart.ts"));
}
#[test]
fn nuxt_auto_import_utils() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("nuxt.config.ts"), "").unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("utils")).unwrap();
    std::fs::write(
        root.join("pages/index.vue"),
        "<script setup>\nconst d = formatDate(new Date());\n</script>",
    )
    .unwrap();
    std::fs::write(
        root.join("utils/formatDate.ts"),
        "export function formatDate() {}",
    )
    .unwrap();

    let mut provider = NuxtProvider::new();
    provider.load(root).unwrap();

    let imports = provider.get_extra_imports().unwrap();
    let page_imports = imports.get("pages/index.vue").unwrap();
    assert!(
        page_imports
            .iter()
            .any(|i| i.specifier == "utils/formatDate.ts")
    );
}
#[test]
fn nuxt_no_auto_import_self() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("nuxt.config.ts"), "").unwrap();
    std::fs::create_dir_all(root.join("composables")).unwrap();
    // No pages dir, so this composable won't be scanned as a page
    // The composable calls itself but should not self-import
    std::fs::write(
        root.join("composables/useAuth.ts"),
        "export function useAuth() { return useAuth(); }",
    )
    .unwrap();

    let mut provider = NuxtProvider::new();
    provider.load(root).unwrap();

    let imports = provider.get_extra_imports().unwrap();
    // composables/useAuth.ts should not appear with a self-import
    if let Some(self_imports) = imports.get("composables/useAuth.ts") {
        assert!(
            !self_imports
                .iter()
                .any(|i| i.specifier == "composables/useAuth.ts"),
            "should not self-import"
        );
    }
}

// ── NuxtContextProvider stats and metadata ──────────────────────────────────
#[test]
fn nuxt_stats() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("nuxt.config.ts"), "").unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("server/api")).unwrap();
    std::fs::create_dir_all(root.join("composables")).unwrap();
    std::fs::write(root.join("pages/index.vue"), "<template></template>").unwrap();
    std::fs::write(
        root.join("server/api/users.get.ts"),
        "export default defineEventHandler(() => [])",
    )
    .unwrap();
    std::fs::write(
        root.join("composables/useAuth.ts"),
        "export function useAuth() {}",
    )
    .unwrap();

    let mut provider = NuxtProvider::new();
    provider.load(root).unwrap();
    let stats = provider.stats();
    assert!(stats.page_routes >= 1);
    assert!(stats.api_routes >= 1);
    assert!(stats.auto_import_symbols >= 1);
}

#[test]
fn nuxt_snapshot_exposes_extra_imports_without_provider() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("nuxt.config.ts"), "").unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("composables")).unwrap();
    std::fs::write(
        root.join("pages/index.vue"),
        "<script setup>\nconst auth = useAuth();\n</script>",
    )
    .unwrap();
    std::fs::write(
        root.join("composables/useAuth.ts"),
        "export function useAuth() {}",
    )
    .unwrap();

    let snapshot = NuxtSnapshot::load(root);
    let imports = snapshot.extra_imports();
    let page_imports = imports.get("pages/index.vue").expect("page imports");
    assert!(
        page_imports
            .iter()
            .any(|import| import.specifier == "composables/useAuth.ts")
    );
    assert_eq!(snapshot.stats().auto_import_symbols, 1);
}

#[test]
fn nuxt_no_pages_is_safe() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("nuxt.config.ts"), "").unwrap();

    let mut provider = NuxtProvider::new();
    provider.load(root).unwrap();
    assert_eq!(provider.stats().page_routes, 0);
}
#[test]
fn nuxt_metadata_has_routes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("nuxt.config.ts"), "").unwrap();
    std::fs::create_dir_all(root.join("pages/users")).unwrap();
    std::fs::write(root.join("pages/users/index.vue"), "<template></template>").unwrap();

    let mut provider = NuxtProvider::new();
    provider.load(root).unwrap();

    let meta = provider.get_metadata();
    let routes = meta["nuxt_routes"].as_array().unwrap();
    assert!(routes.iter().any(|r| r.as_str() == Some("/users")));
}
