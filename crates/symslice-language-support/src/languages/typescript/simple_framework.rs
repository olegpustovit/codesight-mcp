//! Config-file-driven detection for Nuxt, Next, Vue-SPA, and React-SPA
//! under the simple `FrameworkProfile` surface used by
//! `frameworks::profiles::detect_framework`. Priority order encoded here:
//! Nuxt > Next > vue-spa > react-spa.

use std::path::Path;

use crate::frameworks::profiles::{FrameworkProfile, LayerDefinition};

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

pub fn detect(root: &Path) -> Option<FrameworkProfile> {
    if root.join("nuxt.config.ts").exists() || root.join("nuxt.config.js").exists() {
        return Some(nuxt_profile());
    }
    if root.join("next.config.js").exists()
        || root.join("next.config.mjs").exists()
        || root.join("next.config.ts").exists()
    {
        return Some(next_profile());
    }
    if (root.join("vite.config.ts").exists() || root.join("vite.config.js").exists())
        && root.join("src/App.vue").exists()
    {
        return Some(vue_spa_profile());
    }
    if (root.join("vite.config.ts").exists() || root.join("vite.config.js").exists())
        && root.join("src/App.tsx").exists()
    {
        return Some(react_spa_profile());
    }
    None
}
