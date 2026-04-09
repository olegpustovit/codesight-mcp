//! Docstring normalization shared across query packs.

pub(super) fn clean_docstring(raw: &str) -> String {
    let trimmed = raw.trim();
    let without_prefix = trimmed.trim_start_matches(['r', 'R', 'u', 'U', 'f', 'F', 'b', 'B']);
    // Strip string delimiters (triple quotes, single quotes, etc.)
    let stripped = without_prefix
        .strip_prefix("\"\"\"")
        .and_then(|s| s.strip_suffix("\"\"\""))
        .or_else(|| {
            without_prefix
                .strip_prefix("'''")
                .and_then(|s| s.strip_suffix("'''"))
        })
        .or_else(|| {
            without_prefix
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
        })
        .or_else(|| {
            without_prefix
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        })
        .unwrap_or(without_prefix);

    stripped
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_common_string_delimiters_and_prefixes() {
        let cleaned = clean_docstring(
            r#"
            r"""
              Line one

              Line two
            """
            "#,
        );

        assert_eq!(cleaned, "Line one Line two");
    }
}
