//! Integration tests: agent v1 framework nextjs.

use dowser_language_support::frameworks::FrameworkProvider;
use dowser_language_support::languages::typescript::nextjs::*;

// ── _next_route_from_path ───────────────────────────────────────────────────
#[test]
fn next_route_from_path_root_page() {
    assert_eq!(next_route_from_path("app/page.tsx"), "/");
}
#[test]
fn next_route_from_path_simple() {
    assert_eq!(next_route_from_path("app/users/page.tsx"), "/users");
}
#[test]
fn next_route_from_path_dynamic_param() {
    assert_eq!(
        next_route_from_path("app/users/[id]/page.tsx"),
        "/users/:id"
    );
}
#[test]
fn next_route_from_path_catch_all() {
    assert_eq!(
        next_route_from_path("app/posts/[...slug]/page.tsx"),
        "/posts/*"
    );
}
#[test]
fn next_route_from_path_route_group_ignored() {
    assert_eq!(next_route_from_path("app/(auth)/login/page.tsx"), "/login");
}
#[test]
fn next_route_from_path_nested() {
    assert_eq!(
        next_route_from_path("app/dashboard/settings/page.tsx"),
        "/dashboard/settings"
    );
}

// ── _next_api_methods_from_content ──────────────────────────────────────────
#[test]
fn next_api_methods_get_and_post() {
    let content = r#"
export async function GET(request: Request) { return Response.json({}); }
export async function POST(request: Request) { return Response.json({}); }
"#;
    let methods = next_api_methods_from_content(content);
    assert!(methods.contains(&"GET".to_string()));
    assert!(methods.contains(&"POST".to_string()));
}
#[test]
fn next_api_methods_no_exports() {
    let content = "const handler = () => {}; export default handler;";
    let methods = next_api_methods_from_content(content);
    assert!(methods.is_empty());
}
#[test]
fn next_api_methods_sync_function() {
    let content = "export function DELETE(req: Request) { return new Response(); }";
    let methods = next_api_methods_from_content(content);
    assert_eq!(methods, vec!["DELETE"]);
}

/// `function` and `GET` separated by newlines: the legacy regex requires them on one line.
#[test]
fn next_api_methods_multiline_export_async_function() {
    let content = r#"export async function
GET(request: Request) {
  return Response.json({});
}"#;
    let methods = next_api_methods_from_content(content);
    assert!(
        methods.contains(&"GET".to_string()),
        "expected GET from tree-sitter, got {methods:?}"
    );
}

// ── NextjsContextProvider detection ─────────────────────────────────────────
#[test]
fn nextjs_detect_js_config() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("next.config.js"), "").unwrap();
    let provider = NextjsProvider::new();
    assert!(provider.detect(tmp.path()));
}
#[test]
fn nextjs_detect_ts_config() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("next.config.ts"), "").unwrap();
    let provider = NextjsProvider::new();
    assert!(provider.detect(tmp.path()));
}
#[test]
fn nextjs_detect_mjs_config() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("next.config.mjs"), "").unwrap();
    let provider = NextjsProvider::new();
    assert!(provider.detect(tmp.path()));
}
#[test]
fn nextjs_detect_no_config() {
    let tmp = tempfile::tempdir().unwrap();
    let provider = NextjsProvider::new();
    assert!(!provider.detect(tmp.path()));
}

// ── NextjsContextProvider page routing ──────────────────────────────────────
#[test]
fn nextjs_page_routing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("next.config.js"), "").unwrap();
    std::fs::create_dir_all(root.join("app/users/[id]")).unwrap();
    std::fs::write(root.join("app/page.tsx"), "<Home />").unwrap();
    std::fs::write(root.join("app/users/[id]/page.tsx"), "<User />").unwrap();

    let mut provider = NextjsProvider::new();
    provider.load(root).unwrap();

    let ctx_root = provider.get_file_context("app/page.tsx").unwrap();
    assert!(ctx_root.tags.contains(&"nextjs-page".to_string()));
    assert!(
        ctx_root
            .properties
            .iter()
            .any(|(k, v)| k == "route" && v == "/")
    );

    let ctx_user = provider
        .get_file_context("app/users/[id]/page.tsx")
        .unwrap();
    assert!(ctx_user.tags.contains(&"nextjs-page".to_string()));
    assert!(
        ctx_user
            .properties
            .iter()
            .any(|(k, v)| k == "route" && v.contains(":id"))
    );
}
#[test]
fn nextjs_layout_detected() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("next.config.js"), "").unwrap();
    std::fs::create_dir_all(root.join("app")).unwrap();
    std::fs::write(root.join("app/layout.tsx"), "<Layout />").unwrap();

    let mut provider = NextjsProvider::new();
    provider.load(root).unwrap();

    let ctx = provider.get_file_context("app/layout.tsx").unwrap();
    assert!(ctx.tags.contains(&"nextjs-layout".to_string()));
}
#[test]
fn nextjs_api_route_handler() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("next.config.js"), "").unwrap();
    std::fs::create_dir_all(root.join("app/api/users")).unwrap();
    std::fs::write(
        root.join("app/api/users/route.ts"),
        "export async function GET() {}\nexport async function POST() {}",
    )
    .unwrap();

    let mut provider = NextjsProvider::new();
    provider.load(root).unwrap();

    let ctx = provider.get_file_context("app/api/users/route.ts").unwrap();
    assert!(ctx.tags.contains(&"nextjs-api".to_string()));
    let methods = ctx.properties.iter().find(|(k, _)| k == "methods").unwrap();
    assert!(methods.1.contains("GET"));
    assert!(methods.1.contains("POST"));
}
#[test]
fn nextjs_middleware_detected() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("next.config.js"), "").unwrap();
    std::fs::create_dir_all(root.join("app")).unwrap();
    std::fs::write(root.join("app/page.tsx"), "").unwrap();
    std::fs::write(
        root.join("middleware.ts"),
        "export const config = { matcher: '/dashboard/:path*' };",
    )
    .unwrap();

    let mut provider = NextjsProvider::new();
    provider.load(root).unwrap();

    let ctx = provider.get_file_context("middleware.ts").unwrap();
    assert!(ctx.tags.contains(&"nextjs-middleware".to_string()));
}
#[test]
fn nextjs_error_boundary() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("next.config.js"), "").unwrap();
    std::fs::create_dir_all(root.join("app")).unwrap();
    std::fs::write(root.join("app/error.tsx"), "<Error />").unwrap();

    let mut provider = NextjsProvider::new();
    provider.load(root).unwrap();

    let ctx = provider.get_file_context("app/error.tsx").unwrap();
    assert!(ctx.tags.contains(&"nextjs-error".to_string()));
}
#[test]
fn nextjs_metadata_has_routes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("next.config.js"), "").unwrap();
    std::fs::create_dir_all(root.join("app/api/users")).unwrap();
    std::fs::write(root.join("app/page.tsx"), "").unwrap();
    std::fs::write(
        root.join("app/api/users/route.ts"),
        "export async function GET() {}",
    )
    .unwrap();

    let mut provider = NextjsProvider::new();
    provider.load(root).unwrap();
    let meta = provider.get_metadata();

    assert!(meta.contains_key("nextjs_routes"));
    let routes = meta["nextjs_routes"].as_array().unwrap();
    assert!(routes.iter().any(|r| r.as_str() == Some("/")));

    assert!(meta.contains_key("nextjs_api"));
}
#[test]
fn nextjs_stats() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("next.config.js"), "").unwrap();
    std::fs::create_dir_all(root.join("app/api/users")).unwrap();
    std::fs::write(root.join("app/page.tsx"), "").unwrap();
    std::fs::write(
        root.join("app/api/users/route.ts"),
        "export async function GET() {}",
    )
    .unwrap();

    let mut provider = NextjsProvider::new();
    provider.load(root).unwrap();
    let stats = provider.stats();
    assert!(stats.page_routes >= 1);
    assert!(stats.api_routes >= 1);
}

#[test]
fn nextjs_snapshot_exposes_file_context_without_provider() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("next.config.js"), "").unwrap();
    std::fs::create_dir_all(root.join("app")).unwrap();
    std::fs::write(root.join("app/page.tsx"), "<Home />").unwrap();

    let snapshot = NextjsSnapshot::load(root);
    let ctx = render_file_context(&snapshot, "app/page.tsx").expect("page context");
    assert!(ctx.tags.contains(&"nextjs-page".to_string()));
    assert_eq!(snapshot.stats().page_routes, 1);
}

#[test]
fn nextjs_no_app_dir_is_safe() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("next.config.js"), "").unwrap();

    let mut provider = NextjsProvider::new();
    provider.load(root).unwrap();
    assert_eq!(provider.stats().page_routes, 0);
}
#[test]
fn nextjs_route_group_not_in_url() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("next.config.js"), "").unwrap();
    std::fs::create_dir_all(root.join("app/(marketing)/about")).unwrap();
    std::fs::write(root.join("app/(marketing)/about/page.tsx"), "").unwrap();

    let mut provider = NextjsProvider::new();
    provider.load(root).unwrap();

    let ctx = provider
        .get_file_context("app/(marketing)/about/page.tsx")
        .unwrap();
    let route = ctx.properties.iter().find(|(k, _)| k == "route").unwrap();
    assert_eq!(route.1, "/about");
}
