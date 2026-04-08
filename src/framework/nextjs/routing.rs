use regex::Regex;
use tree_sitter::Node;

use crate::extractors::common::parse_tree;

/// Convert an App Router file path to a URL route.
///
/// - `app/page.tsx` -> `/`
/// - `app/users/[id]/page.tsx` -> `/users/:id`
/// - `app/posts/[...slug]/page.tsx` -> `/posts/*`
/// - `app/(auth)/login/page.tsx` -> `/login` (route groups stripped)
pub fn next_route_from_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");

    // Strip leading "app/" prefix
    let without_app = normalized.strip_prefix("app/").unwrap_or(&normalized);

    // Remove the filename (page.tsx, route.ts, etc.)
    let dir = match without_app.rfind('/') {
        Some(idx) => &without_app[..idx],
        None => "", // root page
    };

    if dir.is_empty() {
        return "/".into();
    }

    // Process segments
    let segments: Vec<String> = dir
        .split('/')
        .filter(|seg| {
            // Filter out route groups like (auth)
            !(seg.starts_with('(') && seg.ends_with(')'))
        })
        .map(|seg| {
            if seg.starts_with("[...") && seg.ends_with(']') {
                // Catch-all: [...slug] -> *
                "*".into()
            } else if seg.starts_with('[') && seg.ends_with(']') {
                // Dynamic: [id] -> :id
                format!(":{}", &seg[1..seg.len() - 1])
            } else {
                seg.to_string()
            }
        })
        .collect();

    if segments.is_empty() {
        "/".into()
    } else {
        format!("/{}", segments.join("/"))
    }
}

/// Extract exported HTTP method names from route handler source.
///
/// **Primary:** tree-sitter **TypeScript** then **TSX** — walks `function_declaration` names.
/// **Fallback:** regex for `export (async )?function METHOD` (single-line export header).
pub fn next_api_methods_from_content(content: &str) -> Vec<String> {
    let mut ts = next_api_methods_from_ts_typescript(content);
    if ts.is_empty() {
        ts = next_api_methods_from_ts_tsx(content);
    }
    if !ts.is_empty() {
        return ts;
    }
    next_api_methods_regex(content)
}

fn next_api_methods_regex(content: &str) -> Vec<String> {
    let re =
        Regex::new(r"export\s+(?:async\s+)?function\s+(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\b")
            .unwrap();
    let mut methods: Vec<String> = Vec::new();
    for cap in re.captures_iter(content) {
        let m = cap[1].to_string();
        if !methods.contains(&m) {
            methods.push(m);
        }
    }
    methods
}

fn next_api_methods_from_ts_typescript(content: &str) -> Vec<String> {
    let Ok(tree) = parse_tree(tree_sitter_typescript::LANGUAGE_TYPESCRIPT, content) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect_ts_route_http_handlers(tree.root_node(), content, &mut out);
    out
}

fn next_api_methods_from_ts_tsx(content: &str) -> Vec<String> {
    let Ok(tree) = parse_tree(tree_sitter_typescript::LANGUAGE_TSX, content) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect_ts_route_http_handlers(tree.root_node(), content, &mut out);
    out
}

fn collect_ts_route_http_handlers(node: Node<'_>, source: &str, out: &mut Vec<String>) {
    if node.kind() == "function_declaration"
        && let Some(name_n) = node.child_by_field_name("name")
        && let Ok(name) = name_n.utf8_text(source.as_bytes())
        && is_next_http_verb(name)
        && !out.contains(&name.to_string())
    {
        out.push(name.to_string());
    }
    let mut c = node.walk();
    for ch in node.named_children(&mut c) {
        collect_ts_route_http_handlers(ch, source, out);
    }
}

fn is_next_http_verb(s: &str) -> bool {
    matches!(
        s,
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
    )
}
