use std::collections::HashMap;
use std::path::Path;

use regex::Regex;

/// Parse `{% docs <name> %}...{% enddocs %}` blocks from markdown content.
pub fn parse_doc_blocks_from_content(content: &str) -> HashMap<String, String> {
    let re = Regex::new(r"(?s)\{%\s*docs\s+(\S+)\s*%\}(.*?)\{%\s*enddocs\s*%\}").unwrap();
    let mut blocks = HashMap::new();
    for cap in re.captures_iter(content) {
        let name = cap[1].to_string();
        let body = cap[2].trim().to_string();
        blocks.insert(name, body);
    }
    blocks
}

/// Parse doc blocks from all `.md` files in a directory (non-recursive).
pub fn parse_doc_blocks(dir: &Path) -> HashMap<String, String> {
    let mut blocks = HashMap::new();
    if !dir.exists() {
        return blocks;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md")
                && let Ok(content) = std::fs::read_to_string(&path)
            {
                blocks.extend(parse_doc_blocks_from_content(&content));
            }
        }
    }
    blocks
}

/// Parse doc blocks from multiple directories.
pub fn parse_doc_blocks_multi(dirs: &[&Path]) -> HashMap<String, String> {
    let mut blocks = HashMap::new();
    for dir in dirs {
        blocks.extend(parse_doc_blocks(dir));
    }
    blocks
}

/// Resolve `{{ doc('name') }}` or `{{ doc("name") }}` references in a description.
/// Supports inline substitution (surrounding text preserved).
pub fn resolve_description(desc: &str, doc_blocks: &HashMap<String, String>) -> String {
    if desc.is_empty() {
        return String::new();
    }
    let re = Regex::new(r#"\{\{\s*doc\(\s*['"](\w+)['"]\s*\)\s*\}\}"#).unwrap();
    let result = re.replace_all(desc, |caps: &regex::Captures| {
        let name = &caps[1];
        doc_blocks.get(name).cloned().unwrap_or_default()
    });
    result.into_owned()
}
