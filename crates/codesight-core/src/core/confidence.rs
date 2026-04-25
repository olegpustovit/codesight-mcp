//! Unified trust signal for tool responses.
//!
//! Every consolidated tool surfaces a `_meta.confidence` describing how the
//! payload was derived: from authoritative semantic analysis (`Safe`), from a
//! mix of authoritative + heuristic methods (`Partial`), or from a best-effort
//! string/regex/dispatch heuristic (`Heuristic`). Calling agents branch on
//! this for safety-critical decisions vs. exploration.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Trust level for a tool response.
///
/// Ordering follows trust: `Heuristic < Partial < Safe`. This makes
/// [`Confidence::worst_of`] a plain `min` over the iterator.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// Best-effort string / regex / dispatch heuristic. Caller should verify.
    Heuristic,
    /// Combined authoritative + heuristic, or known-incomplete coverage.
    Partial,
    /// Authoritative semantic analysis with no heuristic shortcuts.
    Safe,
}

impl Confidence {
    /// Lowercase serialized form: `"safe"`, `"partial"`, or `"heuristic"`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Partial => "partial",
            Self::Heuristic => "heuristic",
        }
    }

    /// Return the lowest-trust value in the iterator, or `None` when empty.
    ///
    /// Tools that combine multiple signals call this with each contributing
    /// confidence so the response collapses to its weakest link.
    pub fn worst_of<I>(others: I) -> Option<Self>
    where
        I: IntoIterator<Item = Self>,
    {
        others.into_iter().min()
    }
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Confidence::Safe).unwrap(), "\"safe\"");
        assert_eq!(
            serde_json::to_string(&Confidence::Partial).unwrap(),
            "\"partial\""
        );
        assert_eq!(
            serde_json::to_string(&Confidence::Heuristic).unwrap(),
            "\"heuristic\""
        );
    }

    #[test]
    fn display_matches_serialize() {
        assert_eq!(Confidence::Safe.to_string(), "safe");
        assert_eq!(Confidence::Partial.to_string(), "partial");
        assert_eq!(Confidence::Heuristic.to_string(), "heuristic");
    }

    #[test]
    fn worst_of_picks_lowest_trust() {
        assert_eq!(
            Confidence::worst_of([Confidence::Safe, Confidence::Partial]),
            Some(Confidence::Partial)
        );
        assert_eq!(
            Confidence::worst_of([
                Confidence::Safe,
                Confidence::Partial,
                Confidence::Heuristic,
            ]),
            Some(Confidence::Heuristic)
        );
        assert_eq!(
            Confidence::worst_of([Confidence::Safe, Confidence::Safe]),
            Some(Confidence::Safe)
        );
        assert_eq!(Confidence::worst_of(std::iter::empty()), None);
    }

    #[test]
    fn ordering_is_heuristic_partial_safe() {
        assert!(Confidence::Heuristic < Confidence::Partial);
        assert!(Confidence::Partial < Confidence::Safe);
    }
}
