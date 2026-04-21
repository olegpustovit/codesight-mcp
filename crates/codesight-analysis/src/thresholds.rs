//! Central analysis thresholds. **Single source of truth** — every
//! percentile / cutoff / bucket boundary lives here. Plain constants, no
//! environment overrides: analysis output must be deterministic across
//! machines and shells.

#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    pub cyclomatic_per_fn: u64,
    pub complexity_low_max: u64,
    pub complexity_medium_max: u64,
    pub file_loc_soft: u64,
    pub navigability_min: f64,
    pub comprehensibility_min: f64,
    pub verifiability_min: f64,
    pub change_safety_min: f64,
    pub agent_readiness_min: f64,
    pub doc_coverage_min: f64,
    pub untested_entry_points_max: u64,
    pub structural_score_min: f64,
    pub blast_radius_files_max: u64,
    /// Instability ≤ this is labelled `Stable`.
    pub coupling_stable_max: f64,
    /// Instability ≥ this is labelled `Unstable`.
    pub coupling_unstable_min: f64,
    /// Hotspot score > this is `High`.
    pub hotspot_high_min: f64,
    /// Hotspot score > this is at least `Medium`.
    pub hotspot_medium_min: f64,
}

pub const THRESHOLDS: Thresholds = Thresholds {
    cyclomatic_per_fn: 20,
    complexity_low_max: 5,
    complexity_medium_max: 10,
    file_loc_soft: 500,
    navigability_min: 0.70,
    comprehensibility_min: 0.70,
    verifiability_min: 0.70,
    change_safety_min: 0.70,
    agent_readiness_min: 0.70,
    doc_coverage_min: 0.60,
    untested_entry_points_max: 0,
    structural_score_min: 0.60,
    blast_radius_files_max: 10,
    coupling_stable_max: 0.30,
    coupling_unstable_min: 0.70,
    hotspot_high_min: 50.0,
    hotspot_medium_min: 20.0,
};

impl Thresholds {
    pub fn as_json(&self) -> serde_json::Value {
        serde_json::json!({
            "cyclomatic_per_fn": self.cyclomatic_per_fn,
            "complexity_low_max": self.complexity_low_max,
            "complexity_medium_max": self.complexity_medium_max,
            "file_loc_soft": self.file_loc_soft,
            "navigability_min": self.navigability_min,
            "comprehensibility_min": self.comprehensibility_min,
            "verifiability_min": self.verifiability_min,
            "change_safety_min": self.change_safety_min,
            "agent_readiness_min": self.agent_readiness_min,
            "doc_coverage_min": self.doc_coverage_min,
            "untested_entry_points_max": self.untested_entry_points_max,
            "structural_score_min": self.structural_score_min,
            "blast_radius_files_max": self.blast_radius_files_max,
            "coupling_stable_max": self.coupling_stable_max,
            "coupling_unstable_min": self.coupling_unstable_min,
            "hotspot_high_min": self.hotspot_high_min,
            "hotspot_medium_min": self.hotspot_medium_min,
        })
    }
}

/// Compatibility accessor — returns a `&'static Thresholds`.
pub fn current_thresholds() -> &'static Thresholds {
    &THRESHOLDS
}
