//! Heuristic cyclomatic-style complexity for indexed source snippets.

/// Count cyclomatic complexity from source text.
///
/// Counts decision points: `if`, `elif`/`else if`, `for`, `while`, `except`/`catch`,
/// `case` (switch arms), `&&`/`and`, `||`/`or`, ternary `?`, avoiding `?.` / `??`.
/// The base complexity is 1 (one path through the function).
/// Comment-only lines (`#`, `//`, `*`) are skipped.
pub fn cyclomatic_complexity(source: &str) -> usize {
    let mut complexity: usize = 1; // base path

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.starts_with("//") || trimmed.starts_with('*') {
            continue;
        }

        let words: Vec<&str> = trimmed
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .collect();
        for word in &words {
            match *word {
                "if" | "elif" | "for" | "while" | "except" | "catch" | "case" => {
                    complexity += 1;
                }
                "and" | "or" => {
                    complexity += 1;
                }
                _ => {}
            }
        }

        complexity += trimmed.matches("&&").count();
        complexity += trimmed.matches("||").count();
        let bytes = trimmed.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'?' {
                let next = bytes.get(i + 1).copied().unwrap_or(0);
                if next != b'.' && next != b'?' {
                    complexity += 1;
                }
            }
        }
    }
    complexity
}

/// Severity label for [`cyclomatic_complexity`] totals.
pub fn complexity_assessment(cyclomatic: usize) -> &'static str {
    match cyclomatic {
        0..=5 => "low",
        6..=10 => "medium",
        _ => "high",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_comment_only_lines() {
        let src = "// if if if\nfn f() {}\n";
        assert_eq!(cyclomatic_complexity(src), 1);
    }

    #[test]
    fn counts_keywords_and_boolean_ops() {
        let src = "fn f() {\n  if a && b { }\n  while c { }\n}\n";
        assert!(cyclomatic_complexity(src) >= 4);
    }

    #[test]
    fn assessment_buckets() {
        assert_eq!(complexity_assessment(3), "low");
        assert_eq!(complexity_assessment(8), "medium");
        assert_eq!(complexity_assessment(20), "high");
    }
}
