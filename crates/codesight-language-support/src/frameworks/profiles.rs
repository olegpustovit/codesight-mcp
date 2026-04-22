//! Framework profile types and cross-language dispatcher.
//!
//! Per-framework detection (Laravel, Nuxt, Next, Vue-SPA, React-SPA) lives
//! on the owning `Language` via `Language::simple_framework_profile`. This
//! module owns the shared types plus the language-level precedence order.

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

/// Detect the primary framework for a project root.
///
/// Iterates every registered language in [`crate::builtin_registry`] and
/// returns the first `Some` returned by `Language::simple_framework_profile`.
/// Precedence falls out of registry order — no extra priority knob.
pub fn detect_framework(root: &Path) -> Option<FrameworkProfile> {
    crate::builtin_registry()
        .all()
        .find_map(|lang| lang.simple_framework_profile(root))
}
