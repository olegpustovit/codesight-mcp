//! Go language implementation.

use std::fs;
use std::path::Path;

use crate::traits::complexity::{ComplexityModel, LanguageFamily, NestingMode};
use crate::traits::imports::{ImportExtractionMode, ImportResolution};
use crate::traits::liveness::LivenessPolicy;
use crate::traits::packages::PackageManifest;
use crate::traits::visibility::VisibilityModel;
use crate::type_density::measure_go_signature;

pub struct Go;

fn go_root_package(specifier: &str) -> String {
    let parts: Vec<&str> = specifier.split('/').collect();
    if parts.len() <= 3 {
        if parts[0].contains('.') && parts.len() > 3 {
            parts[..3].join("/")
        } else {
            specifier.to_string()
        }
    } else if parts[0].contains('.') {
        parts[..3].join("/")
    } else {
        specifier.to_string()
    }
}

impl ImportResolution for Go {
    fn extraction_mode(&self) -> Option<ImportExtractionMode> {
        Some(ImportExtractionMode::TreeSitterFirst)
    }
    fn root_package_from_specifier(&self, specifier: &str) -> String {
        go_root_package(specifier)
    }
}

impl PackageManifest for Go {
    fn root_package_from_specifier(&self, specifier: &str) -> String {
        go_root_package(specifier)
    }

    fn package_name_for_dir(&self, dir: &Path) -> Option<String> {
        let go_mod = dir.join("go.mod");
        if !go_mod.is_file() {
            return None;
        }
        let content = fs::read_to_string(&go_mod).ok()?;
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(module_path) = trimmed.strip_prefix("module ") {
                let module_path = module_path.trim();
                if !module_path.is_empty() {
                    return Some(module_path.to_string());
                }
            }
        }
        None
    }
}

impl LivenessPolicy for Go {
    fn entrypoint_basenames(&self) -> &'static [&'static str] {
        &["main.go"]
    }
    fn is_test_file(&self, path: &str) -> bool {
        path.ends_with("_test.go")
    }
}

impl VisibilityModel for Go {
    fn is_restricted(&self, _source_text: &str, name: &str) -> bool {
        name.chars().next().map(|ch| !ch.is_uppercase()).unwrap_or(true)
    }
}

impl ComplexityModel for Go {
    fn family(&self) -> LanguageFamily {
        LanguageFamily::BracedStatic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Brace
    }
    fn supports_measured_types(&self) -> bool {
        true
    }
    fn measured_type_density(&self, source: &str) -> Option<f64> {
        measure_go_signature(source)
    }
}
