//! Documentation path classification helpers.

pub fn should_skip_relative_doc_path(path: &str) -> bool {
    use crate::db::SKIP_DIRS;
    for seg in path.split(['/', '\\']) {
        if seg.is_empty() {
            continue;
        }
        if SKIP_DIRS.iter().any(|skip| *skip == seg) {
            return true;
        }
        if seg == ".git" {
            return true;
        }
    }
    false
}

/// Detect if a file is a documentation file we should index.
pub fn is_doc_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md")
        || lower.ends_with(".markdown")
        || lower.ends_with(".rst")
        || lower.ends_with(".txt")
        || lower.ends_with(".adoc")
        || lower.ends_with(".asciidoc")
        || lower.ends_with(".html")
        || lower.ends_with(".htm")
        || lower.ends_with(".ipynb")
        || lower.ends_with(".yaml")
        || lower.ends_with(".yml")
        || lower.ends_with(".json")
}
