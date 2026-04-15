use super::render::{metadata, render_file_context};
use super::snapshot::{NextjsSnapshot, NextjsStats};

crate::impl_framework_provider! {
    provider: NextjsProvider,
    snapshot: NextjsSnapshot,
    stats: NextjsStats,
    name: "nextjs",
    detect(root) {
        root.join("next.config.js").exists()
            || root.join("next.config.mjs").exists()
            || root.join("next.config.ts").exists()
    },
    load(self, root) {
        self.snapshot = Some(NextjsSnapshot::load(root));
        Ok(())
    }
}
