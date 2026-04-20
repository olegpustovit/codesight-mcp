//! TypeScript / TSX / JavaScript / JSX language implementations.

pub mod nextjs;

use std::fs;
use std::path::Path;

use crate::traits::complexity::{ComplexityModel, LanguageFamily, NestingMode};
use crate::traits::imports::{ImportExtractionMode, ImportResolution};
use crate::traits::liveness::LivenessPolicy;
use crate::traits::packages::PackageManifest;
use crate::traits::visibility::VisibilityModel;
use crate::type_density::measure_colon_after_name;

pub struct TypeScript;
pub struct Tsx;
pub struct JavaScript;
pub struct Jsx;

fn js_root_package(specifier: &str) -> String {
    if specifier.starts_with('.') {
        return String::new();
    }
    if specifier.starts_with('@') {
        let parts: Vec<&str> = specifier.splitn(3, '/').collect();
        if parts.len() >= 2 {
            return format!("{}/{}", parts[0], parts[1]);
        }
        return specifier.to_string();
    }
    specifier.split('/').next().unwrap_or("").to_string()
}

fn js_package_name_for_dir(dir: &Path) -> Option<String> {
    let package_json = dir.join("package.json");
    if !package_json.is_file() {
        return None;
    }
    let content = fs::read_to_string(&package_json).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v.get("name")?.as_str().map(str::to_string)
}

macro_rules! js_family_imports {
    ($($ty:ident),+ $(,)?) => {
        $(
            impl ImportResolution for $ty {
                fn extraction_mode(&self) -> Option<ImportExtractionMode> {
                    Some(ImportExtractionMode::TreeSitterFirst)
                }
                fn root_package_from_specifier(&self, specifier: &str) -> String {
                    js_root_package(specifier)
                }
            }
            impl PackageManifest for $ty {
                fn root_package_from_specifier(&self, specifier: &str) -> String {
                    js_root_package(specifier)
                }
                fn package_name_for_dir(&self, dir: &Path) -> Option<String> {
                    js_package_name_for_dir(dir)
                }
            }
        )+
    };
}

js_family_imports!(TypeScript, Tsx, JavaScript, Jsx);

fn js_is_test_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("/__tests__/")
        || lower.contains("/tests/")
        || lower.contains("/test/")
        || lower.contains("/spec/")
        || lower.ends_with(".test.ts")
        || lower.ends_with(".test.tsx")
        || lower.ends_with(".test.js")
        || lower.ends_with(".test.jsx")
        || lower.ends_with("_test.ts")
        || lower.ends_with("_test.js")
        || lower.ends_with("_spec.ts")
        || lower.ends_with("_spec.js")
}

fn js_is_restricted(source_text: &str, _name: &str) -> bool {
    let trimmed = source_text.trim_start();
    if trimmed.starts_with("private ") || trimmed.starts_with("protected ") {
        return true;
    }
    !trimmed.starts_with("export ")
}

const JS_ENTRYPOINTS: &[&str] = &[
    "main.js", "index.js", "app.js", "server.js", "main.ts", "index.ts", "app.ts", "server.ts",
];

impl LivenessPolicy for TypeScript {
    fn entrypoint_basenames(&self) -> &'static [&'static str] {
        JS_ENTRYPOINTS
    }
    fn is_test_file(&self, path: &str) -> bool {
        js_is_test_file(path)
    }
}

impl VisibilityModel for TypeScript {
    fn is_restricted(&self, source_text: &str, name: &str) -> bool {
        js_is_restricted(source_text, name)
    }
}

impl ComplexityModel for TypeScript {
    fn family(&self) -> LanguageFamily {
        LanguageFamily::ScriptedDynamic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Brace
    }
    fn fallback_type_baseline(&self) -> Option<f64> {
        Some(0.7)
    }
    fn supports_measured_types(&self) -> bool {
        true
    }
    fn measured_type_density(&self, source: &str) -> Option<f64> {
        measure_colon_after_name(source, "typescript", false, |_| false)
    }
}

impl LivenessPolicy for Tsx {
    fn entrypoint_basenames(&self) -> &'static [&'static str] {
        JS_ENTRYPOINTS
    }
    fn is_test_file(&self, path: &str) -> bool {
        js_is_test_file(path)
    }
}

impl VisibilityModel for Tsx {
    fn is_restricted(&self, source_text: &str, name: &str) -> bool {
        js_is_restricted(source_text, name)
    }
}

impl ComplexityModel for Tsx {
    fn family(&self) -> LanguageFamily {
        LanguageFamily::ScriptedDynamic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Brace
    }
    fn fallback_type_baseline(&self) -> Option<f64> {
        Some(0.7)
    }
    fn supports_measured_types(&self) -> bool {
        true
    }
    fn measured_type_density(&self, source: &str) -> Option<f64> {
        measure_colon_after_name(source, "tsx", false, |_| false)
    }
}

impl LivenessPolicy for JavaScript {
    fn entrypoint_basenames(&self) -> &'static [&'static str] {
        JS_ENTRYPOINTS
    }
    fn is_test_file(&self, path: &str) -> bool {
        js_is_test_file(path)
    }
}

impl VisibilityModel for JavaScript {
    fn is_restricted(&self, source_text: &str, name: &str) -> bool {
        js_is_restricted(source_text, name)
    }
}

impl ComplexityModel for JavaScript {
    fn family(&self) -> LanguageFamily {
        LanguageFamily::ScriptedDynamic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Brace
    }
    fn fallback_type_baseline(&self) -> Option<f64> {
        Some(0.1)
    }
    fn supports_measured_types(&self) -> bool {
        true
    }
    fn measured_type_density(&self, source: &str) -> Option<f64> {
        measure_colon_after_name(source, "javascript", false, |_| false)
    }
}

impl LivenessPolicy for Jsx {
    fn entrypoint_basenames(&self) -> &'static [&'static str] {
        JS_ENTRYPOINTS
    }
    fn is_test_file(&self, path: &str) -> bool {
        js_is_test_file(path)
    }
}

impl VisibilityModel for Jsx {
    fn is_restricted(&self, source_text: &str, name: &str) -> bool {
        js_is_restricted(source_text, name)
    }
}

impl ComplexityModel for Jsx {
    fn family(&self) -> LanguageFamily {
        LanguageFamily::ScriptedDynamic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Brace
    }
    fn fallback_type_baseline(&self) -> Option<f64> {
        Some(0.1)
    }
    fn supports_measured_types(&self) -> bool {
        true
    }
    fn measured_type_density(&self, source: &str) -> Option<f64> {
        measure_colon_after_name(source, "jsx", false, |_| false)
    }
}
