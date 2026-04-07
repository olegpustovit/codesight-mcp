use std::path::Path;

use regex::Regex;

use super::php_structure;
use super::types::RouteInfo;

/// Parse Laravel route definitions from PHP content.
///
/// **tree-sitter PHP** is primary for verb/URI/controller/action; chained `->name('...')` is
/// filled from the regex pass when the AST node omits the chain.
pub fn parse_routes(content: &str) -> Vec<RouteInfo> {
    let mut ts = php_structure::routes_from_tree(content);
    let rx = parse_routes_regex(content);
    if ts.is_empty() {
        return rx;
    }
    if ts.len() == rx.len() {
        for (a, b) in ts.iter_mut().zip(rx.iter()) {
            if a.name.is_none() {
                a.name.clone_from(&b.name);
            }
        }
    }
    ts
}

fn parse_routes_regex(content: &str) -> Vec<RouteInfo> {
    let mut routes = Vec::new();

    let route_re = Regex::new(
        r#"Route::(get|post|put|delete|patch)\(\s*['"]([^'"]+)['"]\s*,\s*\[([A-Za-z\\]+)::class\s*,\s*['"](\w+)['"]\s*\]\s*\)(?:\s*->\s*name\(\s*['"]([^'"]+)['"]\s*\))?"#,
    )
    .unwrap();

    for cap in route_re.captures_iter(content) {
        let verb = cap[1].to_uppercase();
        let uri = cap[2].to_string();
        let controller_full = cap[3].to_string();
        let controller = controller_full
            .rsplit('\\')
            .next()
            .unwrap_or(&controller_full)
            .to_string();
        let action = cap[4].to_string();
        let name = cap.get(5).map(|m| m.as_str().to_string());

        routes.push(RouteInfo {
            verb,
            uri,
            controller,
            action,
            name,
        });
    }

    routes
}

/// Parse all route files in a directory.
pub fn parse_routes_dir(dir: &Path) -> Vec<RouteInfo> {
    if !dir.exists() {
        return Vec::new();
    }
    let mut all_routes = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(true, |e| e != "php") {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                all_routes.extend(parse_routes(&content));
            }
        }
    }
    all_routes
}

/// Match a URL against route definitions, supporting dynamic segments.
pub fn match_route<'a>(url: &str, routes: &'a [RouteInfo]) -> Option<&'a RouteInfo> {
    let url_segments: Vec<&str> = url.trim_matches('/').split('/').collect();

    for route in routes {
        let route_segments: Vec<&str> = route.uri.trim_matches('/').split('/').collect();
        if url_segments.len() != route_segments.len() {
            continue;
        }
        let matches = url_segments
            .iter()
            .zip(&route_segments)
            .all(|(u, r)| r.starts_with('{') && r.ends_with('}') || u == r);
        if matches {
            return Some(route);
        }
    }

    None
}
