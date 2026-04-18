use std::sync::OnceLock;

#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    pub cyclomatic_per_fn: u64,
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
}

impl Thresholds {
    pub const fn defaults() -> Self {
        Self {
            cyclomatic_per_fn: 20,
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
        }
    }

    pub fn as_json(self) -> serde_json::Value {
        serde_json::json!({
            "cyclomatic_per_fn": self.cyclomatic_per_fn,
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
        })
    }
}

pub fn current_thresholds() -> &'static Thresholds {
    static THRESHOLDS: OnceLock<Thresholds> = OnceLock::new();
    THRESHOLDS.get_or_init(|| {
        let defaults = Thresholds::defaults();
        Thresholds {
            cyclomatic_per_fn: env_u64(
                "CODESIGHT_THRESHOLD_CYCLOMATIC_PER_FN",
                defaults.cyclomatic_per_fn,
            ),
            file_loc_soft: env_u64("CODESIGHT_THRESHOLD_FILE_LOC_SOFT", defaults.file_loc_soft),
            navigability_min: env_f64(
                "CODESIGHT_THRESHOLD_NAVIGABILITY_MIN",
                defaults.navigability_min,
            ),
            comprehensibility_min: env_f64(
                "CODESIGHT_THRESHOLD_COMPREHENSIBILITY_MIN",
                defaults.comprehensibility_min,
            ),
            verifiability_min: env_f64(
                "CODESIGHT_THRESHOLD_VERIFIABILITY_MIN",
                defaults.verifiability_min,
            ),
            change_safety_min: env_f64(
                "CODESIGHT_THRESHOLD_CHANGE_SAFETY_MIN",
                defaults.change_safety_min,
            ),
            agent_readiness_min: env_f64(
                "CODESIGHT_THRESHOLD_AGENT_READINESS_MIN",
                defaults.agent_readiness_min,
            ),
            doc_coverage_min: env_f64(
                "CODESIGHT_THRESHOLD_DOC_COVERAGE_MIN",
                defaults.doc_coverage_min,
            ),
            untested_entry_points_max: env_u64(
                "CODESIGHT_THRESHOLD_UNTESTED_ENTRY_POINTS_MAX",
                defaults.untested_entry_points_max,
            ),
            structural_score_min: env_f64(
                "CODESIGHT_THRESHOLD_STRUCTURAL_SCORE_MIN",
                defaults.structural_score_min,
            ),
            blast_radius_files_max: env_u64(
                "CODESIGHT_THRESHOLD_BLAST_RADIUS_FILES_MAX",
                defaults.blast_radius_files_max,
            ),
        }
    })
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(default)
}
