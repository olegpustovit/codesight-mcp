use rustc_hash::FxHashMap;

use crate::frameworks::FileContext;

use super::snapshot::NuxtSnapshot;

pub fn render_file_context(snapshot: &NuxtSnapshot, rel_path: &str) -> Option<FileContext> {
    let normalized = rel_path.replace('\\', "/");

    if let Some(route) = snapshot.page_routes.get(&normalized) {
        return Some(FileContext {
            description: format!("Nuxt page route: {route}"),
            tags: vec!["nuxt-page".into()],
            properties: vec![("route".into(), route.clone())],
        });
    }

    if let Some(api_route) = snapshot.api_routes.get(&normalized) {
        return Some(FileContext {
            description: format!(
                "Nuxt API route: {} {}",
                api_route.method, api_route.endpoint
            ),
            tags: vec!["nuxt-api".into()],
            properties: vec![
                ("method".into(), api_route.method.clone()),
                ("endpoint".into(), api_route.endpoint.clone()),
            ],
        });
    }

    None
}

pub fn metadata(snapshot: &NuxtSnapshot) -> FxHashMap<String, serde_json::Value> {
    let mut meta = FxHashMap::default();

    let routes: Vec<serde_json::Value> = snapshot
        .page_routes
        .values()
        .map(|route| serde_json::Value::String(route.clone()))
        .collect();
    meta.insert("nuxt_routes".into(), serde_json::Value::Array(routes));

    let api: Vec<serde_json::Value> = snapshot
        .api_routes
        .values()
        .map(|route| serde_json::Value::String(format!("{} {}", route.method, route.endpoint)))
        .collect();
    meta.insert("nuxt_api".into(), serde_json::Value::Array(api));

    meta
}
