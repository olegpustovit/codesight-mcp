//! Query pack `tags.scm` header comments (`; language:`, `; grammar:`, …).

#[derive(Debug, Clone, Default)]
pub struct ScmMetadata {
    pub name: String,
    pub extensions: Vec<String>,
    pub grammar: String,
    pub override_extractor: Option<String>,
}

/// Parse `; key: value` header lines from the top of a `tags.scm` file.
pub fn parse_scm_header(source: &str) -> ScmMetadata {
    let mut meta = ScmMetadata::default();
    for line in source.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with(';') {
            if !trimmed.is_empty() {
                break; // end of header
            }
            continue;
        }
        let comment = trimmed.trim_start_matches(';').trim();
        if let Some((key, value)) = comment.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "language" => meta.name = value.to_string(),
                "extensions" => {
                    meta.extensions = value.split_whitespace().map(String::from).collect()
                }
                "grammar" => meta.grammar = value.to_string(),
                "override" | "override_extractor" => {
                    meta.override_extractor = Some(value.to_string())
                }
                _ => {}
            }
        }
    }
    meta
}
