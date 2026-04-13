use super::project::detect_dbt_project;
use super::render::{metadata, render_file_context};
use super::snapshot::{DbtSnapshot, DbtStats};

crate::framework::impl_framework_provider! {
    provider: DbtContextProvider,
    snapshot: DbtSnapshot,
    stats: DbtStats,
    name: "dbt",
    detect(root) {
        detect_dbt_project(root).is_some()
    },
    load(self, root) {
        self.snapshot = Some(DbtSnapshot::load(root)?);
        Ok(())
    }
}
