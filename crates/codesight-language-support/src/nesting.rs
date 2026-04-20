//! Nesting-depth scanners driven by `ComplexityModel::nesting_mode()`.
//!
//! Brace-counting works for C-likes (Rust, Go, Java, TS, JS, C++, PHP, …);
//! indent-counting works for Python, Ruby, Perl, Lua, Elixir. Callers pass
//! a path and source; the façade picks the right scanner.

use crate::facade::nesting_mode_for_path;
use crate::traits::complexity::NestingMode;

/// Compute nesting depth for `source` using the scanner matching `path`'s
/// language. Callers that already have a `NestingMode` in hand can call
/// [`max_nesting_depth_with_mode`] directly.
pub fn max_nesting_depth_for_path(source: &str, path: &str) -> usize {
    max_nesting_depth_with_mode(source, nesting_mode_for_path(path))
}

pub fn max_nesting_depth_with_mode(source: &str, mode: NestingMode) -> usize {
    match mode {
        NestingMode::Brace => brace_nesting_depth(source),
        NestingMode::Indent => indent_nesting_depth(source),
    }
}

fn brace_nesting_depth(source: &str) -> usize {
    let mut depth = 0usize;
    let mut max_depth = 0usize;
    let mut chars = source.chars().peekable();
    let mut in_string = None;
    let mut escape = false;
    let mut in_block_comment = false;
    let mut in_line_comment = false;

    while let Some(ch) = chars.next() {
        if in_line_comment {
            if ch == '\n' {
                in_line_comment = false;
            }
            continue;
        }
        if in_block_comment {
            if ch == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block_comment = false;
            }
            continue;
        }
        if let Some(quote) = in_string {
            if escape {
                escape = false;
                continue;
            }
            if ch == '\\' {
                escape = true;
                continue;
            }
            if ch == quote {
                in_string = None;
            }
            continue;
        }

        if ch == '"' || ch == '`' {
            in_string = Some(ch);
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'/') {
            chars.next();
            in_line_comment = true;
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            in_block_comment = true;
            continue;
        }
        if ch == '{' {
            depth += 1;
            max_depth = max_depth.max(depth);
        } else if ch == '}' {
            depth = depth.saturating_sub(1);
        }
    }

    max_depth
}

fn indent_nesting_depth(source: &str) -> usize {
    let mut stack = vec![0usize];
    let mut max_depth = 0usize;

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with('#')
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*')
        {
            continue;
        }

        let indent = line
            .chars()
            .take_while(|ch| *ch == ' ' || *ch == '\t')
            .fold(0usize, |acc, ch| acc + if ch == '\t' { 4 } else { 1 });

        while stack.last().copied().unwrap_or(0) > indent {
            stack.pop();
        }
        if indent > stack.last().copied().unwrap_or(0) {
            stack.push(indent);
        }
        max_depth = max_depth.max(stack.len().saturating_sub(1));
    }

    max_depth
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brace_nesting_stays_flat_for_match_style_blocks() {
        let source = "fn foo(x: i32) {\n    match x {\n        1 => println!(\"one\"),\n        _ => println!(\"other\"),\n    }\n}";
        assert_eq!(brace_nesting_depth(source), 2);
    }

    #[test]
    fn indent_nesting_tracks_python_blocks() {
        let source = "def foo(x):\n    if x:\n        for item in x:\n            if item:\n                return item\n    return None\n";
        assert_eq!(indent_nesting_depth(source), 4);
    }

    #[test]
    fn facade_routes_path_to_correct_scanner() {
        let rust_body = "fn foo() {\n    if true {\n        println!(\"x\");\n    }\n}";
        assert_eq!(max_nesting_depth_for_path(rust_body, "src/lib.rs"), 2);
        let py_body = "def foo():\n    if True:\n        print('x')\n";
        assert_eq!(max_nesting_depth_for_path(py_body, "src/lib.py"), 2);
    }
}
