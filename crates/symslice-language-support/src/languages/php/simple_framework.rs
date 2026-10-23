//! Laravel detection for the simple `FrameworkProfile` surface used by
//! `frameworks::profiles::detect_framework`. The rich `FrameworkProvider`
//! Laravel implementation lives in `languages/php/laravel/`.

use std::path::Path;

use crate::frameworks::profiles::{FrameworkProfile, LayerDefinition};

fn laravel_profile() -> FrameworkProfile {
    FrameworkProfile {
        name: "laravel".into(),
        ignore_patterns: vec![
            "vendor/".into(),
            "node_modules/".into(),
            "storage/".into(),
            "bootstrap/cache/".into(),
        ],
        entry_point_patterns: vec![
            "routes/web.php".into(),
            "routes/api.php".into(),
            "app/Http/Controllers/**".into(),
        ],
        layer_definitions: vec![
            LayerDefinition {
                name: "controllers".into(),
                pattern: "app/Http/Controllers/".into(),
            },
            LayerDefinition {
                name: "models".into(),
                pattern: "app/Models/".into(),
            },
            LayerDefinition {
                name: "views".into(),
                pattern: "resources/views/".into(),
            },
            LayerDefinition {
                name: "routes".into(),
                pattern: "routes/".into(),
            },
            LayerDefinition {
                name: "migrations".into(),
                pattern: "database/migrations/".into(),
            },
        ],
    }
}

pub fn detect(root: &Path) -> Option<FrameworkProfile> {
    if root.join("artisan").exists()
        && let Ok(contents) = std::fs::read_to_string(root.join("composer.json"))
        && contents.contains("laravel/framework")
    {
        return Some(laravel_profile());
    }
    None
}
