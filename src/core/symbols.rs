use serde::{Deserialize, Serialize};
use strum::{EnumString, IntoStaticStr};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, EnumString, IntoStaticStr,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum SymbolKind {
    Function,
    Class,
    Method,
    Constant,
    Type,
    Template,
    Import,
}

impl SymbolKind {
    pub fn as_str(self) -> &'static str {
        self.into()
    }

    pub fn from_db(value: &str) -> Option<Self> {
        value.parse().ok()
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, EnumString, IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ExtractionTier {
    CuratedQueryPack,
    GenericAstFallback,
    ManualOverride,
    Profiled,
}

impl ExtractionTier {
    pub fn as_str(self) -> &'static str {
        self.into()
    }

    pub fn from_db(value: &str) -> Option<Self> {
        value.parse().ok()
    }

    /// Coarse quality for analysis and retrieval (REFACTOR_PLAN: typed, not numeric).
    #[must_use]
    pub fn fact_quality(self) -> FactQuality {
        match self {
            ExtractionTier::CuratedQueryPack => FactQuality::TrustedStructural,
            ExtractionTier::GenericAstFallback | ExtractionTier::Profiled => {
                FactQuality::BaselineStructural
            }
            ExtractionTier::ManualOverride => FactQuality::Heuristic,
        }
    }
}

/// Structural vs heuristic contribution of indexed symbol facts (decision §3 in REFACTOR_PLAN).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, EnumString, IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FactQuality {
    /// Curated query pack (or equivalent) with no heuristic merge in the primary path.
    TrustedStructural,
    /// Grammar-backed structural path that is not a curated pack (generic AST, profiled alias base).
    BaselineStructural,
    /// Residual manual / regex / text fallbacks and similar.
    Heuristic,
}

impl FactQuality {
    pub fn as_str(self) -> &'static str {
        self.into()
    }

    /// Facts suitable as primary evidence for graph, dead-code, and strict quality analysis.
    #[must_use]
    pub fn suitable_for_strict_analysis(self) -> bool {
        matches!(
            self,
            FactQuality::TrustedStructural | FactQuality::BaselineStructural
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{ExtractionTier, FactQuality};

    #[test]
    fn extraction_tier_fact_quality_mapping() {
        assert_eq!(
            ExtractionTier::CuratedQueryPack.fact_quality(),
            FactQuality::TrustedStructural
        );
        assert_eq!(
            ExtractionTier::GenericAstFallback.fact_quality(),
            FactQuality::BaselineStructural
        );
        assert_eq!(
            ExtractionTier::Profiled.fact_quality(),
            FactQuality::BaselineStructural
        );
        assert_eq!(
            ExtractionTier::ManualOverride.fact_quality(),
            FactQuality::Heuristic
        );
    }

    #[test]
    fn strict_analysis_gate_excludes_heuristic_only_tier() {
        assert!(FactQuality::TrustedStructural.suitable_for_strict_analysis());
        assert!(FactQuality::BaselineStructural.suitable_for_strict_analysis());
        assert!(!FactQuality::Heuristic.suitable_for_strict_analysis());
    }
}
