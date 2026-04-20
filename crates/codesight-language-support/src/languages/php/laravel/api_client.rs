use super::regex::RE;

/// Extract fetch/axios URL patterns from JS/TS content.
pub fn extract_api_calls(content: &str) -> Vec<String> {
    let mut urls = Vec::new();

    // fetch('/api/...') or fetch("/api/...")
    for cap in RE.fetch_api.captures_iter(content) {
        urls.push(cap[1].to_string());
    }

    // axios.get/post/put/delete('/api/...')
    for cap in RE.axios_api.captures_iter(content) {
        urls.push(cap[1].to_string());
    }

    urls
}
