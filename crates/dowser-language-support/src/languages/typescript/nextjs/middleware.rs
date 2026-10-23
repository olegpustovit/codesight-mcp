use regex::Regex;

pub(super) fn extract_middleware_matcher(content: &str) -> Option<String> {
    let re = Regex::new(r#"matcher\s*[:=]\s*['"]([^'"]+)['"]"#).ok()?;
    re.captures(content).map(|c| c[1].to_string())
}
