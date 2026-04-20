//! Braced-static family (Java, C#, C++, Dart, Kotlin, Swift, Scala).
//!
//! All share `public/private/protected/internal` visibility, brace nesting,
//! and a `RetType name(ParamType name)` measured-type-density shape. Kotlin,
//! Swift, and Scala instead use `fun name(p: Type): Ret` which the
//! colon-after-name helper handles.

use std::fs;
use std::path::Path;

use crate::traits::complexity::{ComplexityModel, LanguageFamily, NestingMode};
use crate::traits::imports::{ImportExtractionMode, ImportResolution};
use crate::traits::liveness::LivenessPolicy;
use crate::traits::packages::PackageManifest;
use crate::traits::visibility::VisibilityModel;
use crate::type_density::{measure_colon_after_name, measure_type_before_name};

pub struct Java;
pub struct CSharp;
pub struct Cpp;
pub struct Dart;
pub struct Kotlin;
pub struct Swift;

impl ImportResolution for Dart {
    fn extraction_mode(&self) -> Option<ImportExtractionMode> {
        Some(ImportExtractionMode::FallbackOnly)
    }
}

impl PackageManifest for Dart {}

impl PackageManifest for CSharp {
    fn package_name_for_dir(&self, dir: &Path) -> Option<String> {
        let entries = fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("csproj") {
                continue;
            }
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            if let Some(name) = extract_csproj_tag(&content, "PackageName") {
                return Some(name);
            }
            if let Some(name) = extract_csproj_tag(&content, "AssemblyName") {
                return Some(name);
            }
        }
        None
    }
}

fn extract_csproj_tag(content: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = content.find(&open)?;
    let after_open = start + open.len();
    let end = content[after_open..].find(&close)?;
    let value = content[after_open..after_open + end].trim();
    if value.is_empty() { None } else { Some(value.to_string()) }
}

fn cfamily_is_restricted(source_text: &str, _name: &str) -> bool {
    let trimmed = source_text.trim_start();
    if trimmed.starts_with("private ")
        || trimmed.starts_with("protected ")
        || trimmed.starts_with("internal ")
    {
        return true;
    }
    !trimmed.starts_with("public ")
}

macro_rules! braced_static_type_before_name {
    ($struct:ident, $lang:expr, $entry:expr) => {
        impl LivenessPolicy for $struct {
            fn entrypoint_basenames(&self) -> &'static [&'static str] {
                $entry
            }
        }
        impl VisibilityModel for $struct {
            fn is_restricted(&self, source_text: &str, name: &str) -> bool {
                cfamily_is_restricted(source_text, name)
            }
        }
        impl ComplexityModel for $struct {
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
                measure_type_before_name(source, $lang)
            }
        }
    };
}

braced_static_type_before_name!(Java, "java", &[]);
braced_static_type_before_name!(CSharp, "csharp", &[]);
braced_static_type_before_name!(Cpp, "cpp", &["main.cpp", "main.c"]);
braced_static_type_before_name!(Dart, "dart", &[]);

// Kotlin / Swift use `fun name(p: Ty): Ret { ... }` / `func name(p: Ty) -> Ret { ... }`.
impl LivenessPolicy for Kotlin {}
impl VisibilityModel for Kotlin {
    fn is_restricted(&self, source_text: &str, name: &str) -> bool {
        cfamily_is_restricted(source_text, name)
    }
}
impl ComplexityModel for Kotlin {
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
        measure_colon_after_name(source, "kotlin", false, |_| false)
    }
}

impl LivenessPolicy for Swift {}
impl VisibilityModel for Swift {
    fn is_restricted(&self, source_text: &str, name: &str) -> bool {
        cfamily_is_restricted(source_text, name)
    }
}
impl ComplexityModel for Swift {
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
        measure_colon_after_name(source, "swift", false, |_| false)
    }
}
