//! Path-aware cyclomatic complexity façade + severity classification.
//!
//! The counting logic lives on `Language::cyclomatic_complexity` — each
//! language owns its branch keywords, operators, and comment prefixes.
//! Severity bucketing lives here (where the threshold table is) and
//! returns a typed [`ComplexityAssessment`] enum, not a string.

use std::path::Path;

use serde::Serialize;
use symslice_language_support::{Language, builtin_registry};

use crate::thresholds::THRESHOLDS;

/// Severity bucket for a cyclomatic-complexity value. Thresholds come from
/// [`THRESHOLDS`]; use [`Self::classify`] to map a raw count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComplexityAssessment {
    Low,
    Medium,
    High,
}

impl ComplexityAssessment {
    pub fn classify(cyclomatic: usize) -> Self {
        let c = cyclomatic as u64;
        if c <= THRESHOLDS.complexity_low_max {
            Self::Low
        } else if c <= THRESHOLDS.complexity_medium_max {
            Self::Medium
        } else {
            Self::High
        }
    }
}

/// Cyclomatic complexity for a source snippet, dispatching through the
/// language trait based on `file_path`. Files with no recognized language
/// fall back to the `FallbackLanguage` default (C-family + Python union).
pub fn cyclomatic_complexity_for_path(source: &str, file_path: &str) -> usize {
    match builtin_registry().for_path(Path::new(file_path)) {
        Some(lang) => lang.cyclomatic_complexity(source),
        None => {
            symslice_language_support::fallback::FALLBACK_LANGUAGE.cyclomatic_complexity(source)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_counts_elif_and_and() {
        let src = "def f(x):\n    if x and y:\n        return 1\n    elif x or z:\n        return 2\n    return 0\n";
        assert!(cyclomatic_complexity_for_path(src, "m.py") >= 4);
    }

    #[test]
    fn rust_counts_match_arms_via_fat_arrow() {
        let src = "fn f(x: i32) -> i32 {\n    match x {\n        1 => 1,\n        2 => 2,\n        _ => 0,\n    }\n}\n";
        assert!(cyclomatic_complexity_for_path(src, "lib.rs") >= 4);
    }

    #[test]
    fn yaml_is_flat() {
        let src = "if: true\nname: foo\n";
        assert_eq!(cyclomatic_complexity_for_path(src, "deploy.yaml"), 1);
    }

    #[test]
    fn unknown_extension_uses_fallback() {
        let src = "if a && b { return 1; }\n";
        assert!(cyclomatic_complexity_for_path(src, "x.mystery") >= 3);
    }

    #[test]
    fn complexity_assessment_buckets() {
        assert_eq!(ComplexityAssessment::classify(3), ComplexityAssessment::Low);
        assert_eq!(
            ComplexityAssessment::classify(8),
            ComplexityAssessment::Medium
        );
        assert_eq!(
            ComplexityAssessment::classify(20),
            ComplexityAssessment::High
        );
    }

    /// Dispatch budget guard: storage-time indexing calls
    /// `cyclomatic_complexity_for_path` once per extracted symbol, so the
    /// per-call cost has to stay well below AGENTS.md's <200 ms cold
    /// dispatch budget. Hot path: phf lookup + vtable call.
    #[test]
    fn dispatch_stays_under_budget() {
        let src = "fn f(x: i32) -> i32 {\n    if x > 0 { 1 } else if x < 0 { -1 } else { 0 }\n}\n";
        // Per-invocation guards; this scalar facade is not a complete MCP request.
        let first_start = std::time::Instant::now();
        let first = cyclomatic_complexity_for_path(src, "lib.rs");
        let first_elapsed = first_start.elapsed();
        assert!(first > 0);
        assert!(
            first_elapsed < std::time::Duration::from_millis(200),
            "first invocation took {first_elapsed:?}"
        );
        let next_start = std::time::Instant::now();
        assert_eq!(cyclomatic_complexity_for_path(src, "lib.rs"), first);
        let next_elapsed = next_start.elapsed();
        assert!(
            next_elapsed < std::time::Duration::from_millis(50),
            "subsequent invocation took {next_elapsed:?}"
        );
        let start = std::time::Instant::now();
        let mut total = 0usize;
        for _ in 0..10_000 {
            total += cyclomatic_complexity_for_path(src, "lib.rs");
        }
        let elapsed = start.elapsed();
        assert!(total > 0, "sanity check: fn should report >0 complexity");
        eprintln!(
            "observed 10k-call microbenchmark: {elapsed:?}; first={first_elapsed:?}, subsequent={next_elapsed:?}; aggregate is not one dispatch SLA"
        );
    }
}
