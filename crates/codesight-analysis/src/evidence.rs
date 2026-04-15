use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource {
    Structural,
    Heuristic,
    Mixed,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvidenceNote {
    pub signal: String,
    pub source: EvidenceSource,
    pub detail: String,
}

impl EvidenceNote {
    pub fn structural(signal: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            signal: signal.into(),
            source: EvidenceSource::Structural,
            detail: detail.into(),
        }
    }

    pub fn heuristic(signal: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            signal: signal.into(),
            source: EvidenceSource::Heuristic,
            detail: detail.into(),
        }
    }

    pub fn mixed(signal: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            signal: signal.into(),
            source: EvidenceSource::Mixed,
            detail: detail.into(),
        }
    }
}
