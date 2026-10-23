//! Generic cyclomatic-complexity counter. The per-language `Language` trait
//! picks the keywords, operators, and comment prefixes; this function does
//! the bookkeeping. The base complexity is `1` (one path through the body).

/// Count cyclomatic complexity of `source` using the provided language-
/// specific vocabulary.
///
/// - `branch_keywords` — identifiers that contribute one decision point each
///   (e.g. `["if", "for", "while", "match"]`). Tokens split on non-identifier
///   boundaries so `if_statement` does not match.
/// - `branch_operators` — inline operator strings that contribute one
///   decision point per occurrence (e.g. `["&&", "||"]`).
/// - `line_comment_prefixes` — prefixes to skip comment-only lines.
///
/// Ternary `?` is counted as a decision point unless it appears as part of
/// `?.` / `??` (common in TS / JS / Swift / Kotlin / C# nullable chaining).
/// Languages without ternary should omit `?` from their branch operators
/// (it is not part of the default).
pub fn count_cyclomatic(
    source: &str,
    branch_keywords: &[&str],
    branch_operators: &[&str],
    line_comment_prefixes: &[&str],
) -> usize {
    let mut complexity: usize = 1;

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if line_comment_prefixes
            .iter()
            .any(|prefix| trimmed.starts_with(prefix))
            || trimmed.starts_with('*')
        {
            continue;
        }

        let words: Vec<&str> = trimmed
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .collect();
        for word in &words {
            if word.is_empty() {
                continue;
            }
            if branch_keywords.contains(word) {
                complexity += 1;
            }
        }

        for op in branch_operators {
            if !op.is_empty() {
                complexity += trimmed.matches(op).count();
            }
        }

        let bytes = trimmed.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'?' {
                let next = bytes.get(i + 1).copied().unwrap_or(0);
                if next == b'.' || next == b'?' {
                    // `?.` / `??` are nullable-chaining operators — consume
                    // both bytes so the second `?` doesn't re-fire as a
                    // ternary.
                    i += 2;
                    continue;
                }
                complexity += 1;
            }
            i += 1;
        }
    }

    complexity
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_complexity_is_one() {
        assert_eq!(count_cyclomatic("", &[], &[], &[]), 1);
    }

    #[test]
    fn counts_branch_keywords() {
        let src = "if a { for b { while c { } } }";
        let n = count_cyclomatic(src, &["if", "for", "while"], &[], &["//"]);
        assert_eq!(n, 4);
    }

    #[test]
    fn counts_short_circuit_operators() {
        let src = "x && y || z";
        let n = count_cyclomatic(src, &[], &["&&", "||"], &["//"]);
        assert_eq!(n, 3);
    }

    #[test]
    fn skips_line_comments() {
        let src = "// if if if\nreturn 1\n";
        let n = count_cyclomatic(src, &["if"], &[], &["//"]);
        assert_eq!(n, 1);
    }

    #[test]
    fn ternary_but_not_null_chain() {
        let src = "a ? b : c.d ?? e.f?.g";
        // One `?` (ternary) + two skipped (`??`, `?.`).
        let n = count_cyclomatic(src, &[], &[], &[]);
        assert_eq!(n, 2);
    }
}
