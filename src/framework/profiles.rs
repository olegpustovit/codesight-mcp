//! Framework profile detection and metadata.
//!
//! Detects which web framework a project uses based on marker files and
//! returns a `FrameworkProfile` with ignore patterns, entry points, and
//! layer definitions.

use std::path::Path;

/// Metadata about a detected framework.
#[derive(Debug, Clone)]
pub struct FrameworkProfile {
    pub name: String,
    pub ignore_patterns: Vec<String>,
    pub entry_point_patterns: Vec<String>,
    pub layer_definitions: Vec<LayerDefinition>,
}

/// A named architectural layer (e.g. "pages", "components").
#[derive(Debug, Clone)]
pub struct LayerDefinition {
    pub name: String,
    pub pattern: String,
}

/// Convert a profile to a JSON-like metadata map.
pub fn profile_to_meta(
    profile: &FrameworkProfile,
) -> std::collections::HashMap<String, serde_json::Value> {
    let mut meta = std::collections::HashMap::new();

    let mut fp = serde_json::Map::new();
    fp.insert(
        "name".into(),
        serde_json::Value::String(profile.name.clone()),
    );
    fp.insert(
        "entry_point_patterns".into(),
        serde_json::Value::Array(
            profile
                .entry_point_patterns
                .iter()
                .map(|s| serde_json::Value::String(s.clone()))
                .collect(),
        ),
    );
    fp.insert(
        "ignore_patterns".into(),
        serde_json::Value::Array(
            profile
                .ignore_patterns
                .iter()
                .map(|s| serde_json::Value::String(s.clone()))
                .collect(),
        ),
    );
    fp.insert(
        "layer_definitions".into(),
        serde_json::Value::Array(
            profile
                .layer_definitions
                .iter()
                .map(|ld| {
                    let mut m = serde_json::Map::new();
                    m.insert("name".into(), serde_json::Value::String(ld.name.clone()));
                    m.insert(
                        "pattern".into(),
                        serde_json::Value::String(ld.pattern.clone()),
                    );
                    serde_json::Value::Object(m)
                })
                .collect(),
        ),
    );

    meta.insert("framework_profile".into(), serde_json::Value::Object(fp));
    meta
}

// ── Detection ──────────────────────────────────────────────────────────────

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

fn nuxt_profile() -> FrameworkProfile {
    FrameworkProfile {
        name: "nuxt".into(),
        ignore_patterns: vec![".nuxt/".into(), "node_modules/".into(), ".output/".into()],
        entry_point_patterns: vec![
            "pages/**/*.vue".into(),
            "server/api/**".into(),
            "nuxt.config.*".into(),
        ],
        layer_definitions: vec![
            LayerDefinition {
                name: "pages".into(),
                pattern: "pages/".into(),
            },
            LayerDefinition {
                name: "components".into(),
                pattern: "components/".into(),
            },
            LayerDefinition {
                name: "composables".into(),
                pattern: "composables/".into(),
            },
            LayerDefinition {
                name: "server".into(),
                pattern: "server/".into(),
            },
        ],
    }
}

fn next_profile() -> FrameworkProfile {
    FrameworkProfile {
        name: "next".into(),
        ignore_patterns: vec![".next/".into(), "node_modules/".into(), "out/".into()],
        entry_point_patterns: vec![
            "app/**/page.*".into(),
            "app/**/route.*".into(),
            "next.config.*".into(),
        ],
        layer_definitions: vec![
            LayerDefinition {
                name: "pages".into(),
                pattern: "app/".into(),
            },
            LayerDefinition {
                name: "components".into(),
                pattern: "components/".into(),
            },
            LayerDefinition {
                name: "lib".into(),
                pattern: "lib/".into(),
            },
        ],
    }
}

fn vue_spa_profile() -> FrameworkProfile {
    FrameworkProfile {
        name: "vue-spa".into(),
        ignore_patterns: vec!["node_modules/".into(), "dist/".into()],
        entry_point_patterns: vec!["src/App.vue".into(), "src/main.*".into()],
        layer_definitions: vec![
            LayerDefinition {
                name: "components".into(),
                pattern: "src/components/".into(),
            },
            LayerDefinition {
                name: "views".into(),
                pattern: "src/views/".into(),
            },
        ],
    }
}

fn react_spa_profile() -> FrameworkProfile {
    FrameworkProfile {
        name: "react-spa".into(),
        ignore_patterns: vec!["node_modules/".into(), "dist/".into(), "build/".into()],
        entry_point_patterns: vec!["src/App.*".into(), "src/main.*".into()],
        layer_definitions: vec![
            LayerDefinition {
                name: "components".into(),
                pattern: "src/components/".into(),
            },
            LayerDefinition {
                name: "pages".into(),
                pattern: "src/pages/".into(),
            },
        ],
    }
}

/// Detect the primary framework for a project root.
///
/// Detection order determines precedence: Laravel > Nuxt > Next > vue-spa >
/// react-spa.  Returns `None` if nothing matches.
pub fn detect_framework(root: &Path) -> Option<FrameworkProfile> {
    // Laravel: artisan + composer.json with laravel/framework
    if root.join("artisan").exists() {
        if let Ok(contents) = std::fs::read_to_string(root.join("composer.json")) {
            if contents.contains("laravel/framework") {
                return Some(laravel_profile());
            }
        }
    }

    // Nuxt: nuxt.config.ts or nuxt.config.js
    if root.join("nuxt.config.ts").exists() || root.join("nuxt.config.js").exists() {
        return Some(nuxt_profile());
    }

    // Next: next.config.js, .mjs, or .ts
    if root.join("next.config.js").exists()
        || root.join("next.config.mjs").exists()
        || root.join("next.config.ts").exists()
    {
        return Some(next_profile());
    }

    // Vite + Vue SPA
    if (root.join("vite.config.ts").exists() || root.join("vite.config.js").exists())
        && root.join("src/App.vue").exists()
    {
        return Some(vue_spa_profile());
    }

    // Vite + React SPA
    if (root.join("vite.config.ts").exists() || root.join("vite.config.js").exists())
        && root.join("src/App.tsx").exists()
    {
        return Some(react_spa_profile());
    }

    None
}
