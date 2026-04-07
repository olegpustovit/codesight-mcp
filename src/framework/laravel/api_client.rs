use regex::Regex;

/// Extract fetch/axios URL patterns from JS/TS content.
pub fn extract_api_calls(content: &str) -> Vec<String> {
    let mut urls = Vec::new();

    // fetch('/api/...') or fetch("/api/...")
    let fetch_re = Regex::new(r#"fetch\(\s*['"](/api/[^'"]+)['"]\s*"#).unwrap();
    for cap in fetch_re.captures_iter(content) {
        urls.push(cap[1].to_string());
    }

    // axios.get/post/put/delete('/api/...')
    let axios_re = Regex::new(r#"axios\.\w+\(\s*['"](/api/[^'"]+)['"]\s*"#).unwrap();
    for cap in axios_re.captures_iter(content) {
        urls.push(cap[1].to_string());
    }

    urls
}
