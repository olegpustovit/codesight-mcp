use super::imports::compute_extra_imports;
use super::render::{metadata, render_file_context};
use super::snapshot::{LaravelSnapshot, LaravelStats};

crate::impl_framework_provider! {
    provider: LaravelProvider,
    snapshot: LaravelSnapshot,
    stats: LaravelStats,
    name: "laravel",
    detect(root) {
        if !root.join("artisan").exists() {
            return false;
        }
        match std::fs::read_to_string(root.join("composer.json")) {
            Ok(content) => content.contains("laravel/framework"),
            Err(_) => false,
        }
    },
    load(self, root) {
        let mut snapshot = LaravelSnapshot::load(root);
        snapshot.extra_imports = compute_extra_imports(&snapshot, root);
        self.snapshot = Some(snapshot);
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
