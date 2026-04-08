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
}
