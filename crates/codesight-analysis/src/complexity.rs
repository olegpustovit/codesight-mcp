//! Path-aware cyclomatic complexity façade + severity classification.
//!
//! The counting logic lives on `Language::cyclomatic_complexity` — each
//! language owns its branch keywords, operators, and comment prefixes.
//! Severity bucketing lives here (where the threshold table is) and
//! returns a typed [`ComplexityAssessment`] enum, not a string.

use std::path::Path;

use codesight_language_support::{Language, builtin_registry};
use serde::Serialize;

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
        None => codesight_language_support::fallback::FALLBACK_LANGUAGE.cyclomatic_complexity(source),
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
        assert_eq!(ComplexityAssessment::classify(8), ComplexityAssessment::Medium);
        assert_eq!(ComplexityAssessment::classify(20), ComplexityAssessment::High);
    }
}
