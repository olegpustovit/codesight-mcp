/// Convert a Nuxt pages path to a URL route.
///
/// - `pages/index.vue` -> `/`
/// - `pages/users/index.vue` -> `/users`
/// - `pages/about.vue` -> `/about`
/// - `pages/users/[id].vue` -> `/users/:id`
/// - `pages/posts/[...slug].vue` -> `/posts/*`
pub fn nuxt_route_from_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");

    // Strip leading "pages/" prefix
    let without_pages = normalized.strip_prefix("pages/").unwrap_or(&normalized);

    // Remove extension
    let without_ext = without_pages
        .rsplit_once('.')
        .map(|(base, _)| base)
        .unwrap_or(without_pages);

    // Split into segments
    let segments: Vec<&str> = without_ext.split('/').collect();

    // Process segments, removing trailing "index"
    let mut route_parts: Vec<String> = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        if i == segments.len() - 1 && *seg == "index" {
            continue; // Skip trailing index
        }
        if seg.starts_with("[...") && seg.ends_with(']') {
            route_parts.push("*".into());
        } else if seg.starts_with('[') && seg.ends_with(']') {
            route_parts.push(format!(":{}", &seg[1..seg.len() - 1]));
        } else {
            route_parts.push(seg.to_string());
        }
    }

    if route_parts.is_empty() {
        "/".into()
    } else {
        format!("/{}", route_parts.join("/"))
    }
}

/// Parsed Nuxt server API route info.
#[derive(Debug, Clone)]
pub struct NuxtApiRoute {
    pub method: String,
    pub endpoint: String,
}

/// Parse a Nuxt server API file path into method + endpoint.
///
/// - `server/api/users.get.ts` -> GET /api/users
/// - `server/api/health.ts` -> ALL /api/health
/// - `server/api/users/[id].get.ts` -> GET /api/users/:id
pub fn nuxt_api_route_from_path(path: &str) -> NuxtApiRoute {
    let normalized = path.replace('\\', "/");

    // Strip "server/" prefix
    let without_server = normalized.strip_prefix("server/").unwrap_or(&normalized);

    // Remove extension
    let without_ext = without_server
        .rsplit_once('.')
        .map(|(base, _ext)| {
            // Check if the "extension" is actually a method suffix
            let methods = ["get", "post", "put", "patch", "delete", "head", "options"];
            if let Some((inner_base, method_part)) = base.rsplit_once('.')
                && methods.contains(&method_part)
            {
                return (inner_base, Some(method_part));
            }
            (base, None)
        })
        .unwrap_or((without_server, None));

    let (base_path, method_suffix) = without_ext;

    let method = method_suffix
        .map(|m| m.to_uppercase())
        .unwrap_or_else(|| "ALL".into());

    // Process segments for dynamic params
    let segments: Vec<String> = base_path
        .split('/')
        .map(|seg| {
            if seg.starts_with("[...") && seg.ends_with(']') {
                "*".into()
            } else if seg.starts_with('[') && seg.ends_with(']') {
                format!(":{}", &seg[1..seg.len() - 1])
            } else {
                seg.to_string()
            }
        })
        .collect();

    let endpoint = format!("/{}", segments.join("/"));

    NuxtApiRoute { method, endpoint }
}
