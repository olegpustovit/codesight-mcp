use std::collections::HashMap;

use crate::frameworks::FileContext;

use super::routing::next_route_from_path;
use super::snapshot::NextjsSnapshot;

pub fn render_file_context(snapshot: &NextjsSnapshot, rel_path: &str) -> Option<FileContext> {
    let normalized = rel_path.replace('\\', "/");

    if let Some(route) = snapshot.page_routes.get(&normalized) {
        return Some(FileContext {
            description: format!("Next.js page route: {route}"),
            tags: vec!["nextjs-page".into()],
            properties: vec![("route".into(), route.clone())],
        });
    }

    if let Some(methods) = snapshot.api_routes.get(&normalized) {
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

    if snapshot.layouts.contains(&normalized) {
        return Some(FileContext {
            description: "Next.js layout".into(),
            tags: vec!["nextjs-layout".into()],
            properties: vec![],
        });
    }

    if snapshot.errors.contains(&normalized) {
        return Some(FileContext {
            description: "Next.js error boundary".into(),
            tags: vec!["nextjs-error".into()],
            properties: vec![],
        });
    }

    if let Some(matcher) = snapshot.middlewares.get(&normalized) {
        let mut properties = Vec::new();
        if let Some(matcher) = matcher {
            properties.push(("matcher".into(), matcher.clone()));
        }
        return Some(FileContext {
            description: "Next.js middleware".into(),
            tags: vec!["nextjs-middleware".into()],
            properties,
        });
    }

    None
}

pub fn metadata(snapshot: &NextjsSnapshot) -> HashMap<String, serde_json::Value> {
    let mut meta = HashMap::new();

    let routes: Vec<serde_json::Value> = snapshot
        .page_routes
        .values()
        .map(|route| serde_json::Value::String(route.clone()))
        .collect();
    meta.insert("nextjs_routes".into(), serde_json::Value::Array(routes));

    let api: Vec<serde_json::Value> = snapshot
        .api_routes
        .iter()
        .map(|(path, methods)| serde_json::Value::String(format!("{} {}", methods.join(","), path)))
        .collect();
    meta.insert("nextjs_api".into(), serde_json::Value::Array(api));

    meta
}
