//! Savings ledger, response envelope builder, and per-session counters.

mod baseline;
mod ledger;
mod meta_builder;
mod pricing;
mod session;

pub(in crate::service) use baseline::search_rg_equivalent_bytes;
pub use codesight_core::Confidence;
pub use ledger::{SavingsLedger, ToolCallRecord};
pub use meta_builder::{Meta, MetaBuilder, RetrievalClass};
pub use session::{
    QualityHotspotMarker, QualitySnapshotMarker, ReadContentStats, SessionState,
    StaleWarningRecord, ToolSessionStats,
};

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{MetaBuilder, RetrievalClass, SavingsLedger};

    #[test]
    fn records_codesight_read_paths_and_detects_double_read() {
        let dir = tempdir().unwrap();
        let ledger = SavingsLedger::new(dir.path().join("_savings.json"));
        ledger
            .record_codesight_read_paths(&["src/foo.rs".to_string(), "src/bar.rs".to_string()])
            .unwrap();
        // Absolute path from the hook side should match on suffix.
        let was_read = ledger
            .record_edit_and_check_double_read("/tmp/repo/src/foo.rs")
            .unwrap();
        assert!(was_read);
        // Unseen file is not a double-read.
        let not_read = ledger
            .record_edit_and_check_double_read("/tmp/repo/src/baz.rs")
            .unwrap();
        assert!(!not_read);
        let session = ledger.current_session().unwrap();
        assert_eq!(session.edits_after_codesight_read, 1);
    }

    #[test]
    fn savings_ledger_rounds_accumulated_costs() {
        let dir = tempdir().unwrap();
        let ledger = SavingsLedger::new(dir.path().join("_savings.json"));
        ledger.record(123_456).unwrap();
        ledger.record(123_456).unwrap();
        let (_tokens, costs) = ledger.current_state().unwrap();
        for value in costs.values() {
            let scaled = value * 10_000.0;
            assert!(
                (scaled - scaled.round()).abs() < 1e-9,
                "cost should be rounded to 4 decimals: {value}"
            );
        }
    }

    #[test]
    fn meta_builder_without_class_omits_retrieval_class() {
        let dir = tempdir().unwrap();
        let ledger = SavingsLedger::new(dir.path().join("_savings.json"));
        let body = serde_json::json!({"hello": "world"});
        let meta = MetaBuilder::new(1024).success(&body, &ledger).unwrap();
        assert!(meta.retrieval_class.is_none());
        // Round-trips via JSON without serializing the missing field.
        let encoded = serde_json::to_string(&meta).unwrap();
        assert!(
            !encoded.contains("retrieval_class"),
            "retrieval_class must be omitted: {encoded}",
        );
    }

    #[test]
    fn meta_builder_with_class_surfaces_it_in_serialized_output() {
        let dir = tempdir().unwrap();
        let ledger = SavingsLedger::new(dir.path().join("_savings.json"));
        let body = serde_json::json!({"hello": "world"});
        let meta = MetaBuilder::new(1024)
            .with_class(RetrievalClass::WholeFileRead)
            .success(&body, &ledger)
            .unwrap();
        assert_eq!(meta.retrieval_class, Some(RetrievalClass::WholeFileRead));
        let encoded = serde_json::to_value(&meta).unwrap();
        assert_eq!(
            encoded["retrieval_class"],
            serde_json::json!("whole_file_read")
        );
    }

    #[test]
    fn retrieval_class_snake_cases_all_variants() {
        for (variant, expected) in [
            (RetrievalClass::Structural, "structural"),
            (RetrievalClass::TargetedRange, "targeted_range"),
            (RetrievalClass::WholeFileRead, "whole_file_read"),
            (RetrievalClass::MetadataOnly, "metadata_only"),
            (RetrievalClass::Search, "search"),
        ] {
            let encoded = serde_json::to_value(variant).unwrap();
            assert_eq!(encoded, serde_json::json!(expected));
        }
    }
}
