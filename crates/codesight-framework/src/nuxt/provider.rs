use super::render::{metadata, render_file_context};
use super::snapshot::{NuxtSnapshot, NuxtStats};

crate::impl_framework_provider! {
    provider: NuxtProvider,
    snapshot: NuxtSnapshot,
    stats: NuxtStats,
    name: "nuxt",
    detect(root) {
        root.join("nuxt.config.ts").exists() || root.join("nuxt.config.js").exists()
    },
    load(self, root) {
        self.snapshot = Some(NuxtSnapshot::load(root));
        Ok(())
    },
    extra_imports(self) {
        Ok(self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.extra_imports.clone())
            .unwrap_or_default())
    }
}
