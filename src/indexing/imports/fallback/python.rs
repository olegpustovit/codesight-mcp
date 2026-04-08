use crate::indexing::imports::ImportEdge;

pub(crate) fn extract_imports(content: &str) -> Vec<ImportEdge> {
    let mut edges = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("from ") {
            if let Some(rest) = trimmed.strip_prefix("from ") {
                let parts: Vec<&str> = rest.splitn(2, " import ").collect();
                if parts.len() == 2 {
                    let module = parts[0].trim();
                    if module == "__future__" {
                        continue;
                    }
                    let imports_str = parts[1].trim();
                    let names: Vec<String> = if imports_str == "*" {
                        Vec::new()
                    } else {
                        imports_str
                            .split(',')
                            .map(|part| {
                                let part = part.trim();
                                part.split_whitespace().next().unwrap_or(part).to_string()
                            })
                            .filter(|part| !part.is_empty())
                            .collect()
                    };
                    edges.push(ImportEdge {
                        specifier: module.to_string(),
                        names,
                    });
                }
            }
        } else if trimmed.starts_with("import ")
            && let Some(rest) = trimmed.strip_prefix("import ")
        {
            let module = rest
                .split_whitespace()
                .next()
                .unwrap_or(rest)
                .trim_end_matches(',');
            if !module.is_empty() {
                edges.push(ImportEdge {
                    specifier: module.to_string(),
                    names: Vec::new(),
                });
            }
        }
    }
    edges
}
