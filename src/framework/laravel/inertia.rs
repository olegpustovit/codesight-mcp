use std::path::Path;

use regex::Regex;

/// Extract Inertia page references from PHP content.
/// Returns page component paths relative to resources/js/Pages/.
pub fn extract_inertia_renders(content: &str) -> Vec<String> {
    let mut pages = Vec::new();

    // Inertia::render('Page/Name') or Inertia::render("Page/Name")
    let render_re = Regex::new(r#"Inertia::render\(\s*['"]([^'"]+)['"]\s*"#).unwrap();
    for cap in render_re.captures_iter(content) {
        let page = cap[1].to_string();
        if !pages.contains(&page) {
            pages.push(page);
        }
    }

    // inertia('Page/Name') helper function — avoid matching Inertia::inertia(
    // by requiring that the preceding character (if any) is not `:` or a word char.
    let helper_re = Regex::new(r#"(?:^|[^:\w])inertia\(\s*['"]([^'"]+)['"]\s*"#).unwrap();
    for cap in helper_re.captures_iter(content) {
        let page = cap[1].to_string();
        if !pages.contains(&page) {
            pages.push(page);
        }
    }

    pages
}

/// Resolve an Inertia page name to a file path, checking for .vue then .tsx.
pub fn resolve_inertia_page(root: &Path, page_name: &str) -> Option<String> {
    // If it already has an extension, use it as-is
    if page_name.ends_with(".vue") || page_name.ends_with(".tsx") || page_name.ends_with(".jsx") {
        let path = format!("resources/js/Pages/{page_name}");
        if root.join(&path).exists() {
            return Some(path);
        }
        return None;
    }

    // Try .vue first, then .tsx
    let vue_path = format!("resources/js/Pages/{page_name}.vue");
    if root.join(&vue_path).exists() {
        return Some(vue_path);
    }
    let tsx_path = format!("resources/js/Pages/{page_name}.tsx");
    if root.join(&tsx_path).exists() {
        return Some(tsx_path);
    }
    let jsx_path = format!("resources/js/Pages/{page_name}.jsx");
    if root.join(&jsx_path).exists() {
        return Some(jsx_path);
    }

    None
}
