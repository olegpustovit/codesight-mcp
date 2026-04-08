//! Docstring normalization shared across query packs.

pub(super) fn clean_docstring(raw: &str) -> String {
    let trimmed = raw.trim();
    // Strip string delimiters (triple quotes, single quotes, etc.)
    let stripped = trimmed
        .strip_prefix("\"\"\"")
        .and_then(|s| s.strip_suffix("\"\"\""))
        .or_else(|| {
            trimmed
                .strip_prefix("'''")
                .and_then(|s| s.strip_suffix("'''"))
        })
        .or_else(|| trimmed.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
        .or_else(|| {
            trimmed
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        })
        .unwrap_or(trimmed);

    // Also strip leading string prefix chars (r, u, f, b)
    let stripped = stripped.trim_start_matches(['r', 'R', 'u', 'U', 'f', 'F', 'b', 'B']);

    stripped
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
