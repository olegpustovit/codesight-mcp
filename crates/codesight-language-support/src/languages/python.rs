//! Python language implementation.

use std::fs;
use std::path::Path;

use crate::traits::complexity::{ComplexityModel, LanguageFamily, NestingMode};
use crate::traits::imports::{ImportExtractionMode, ImportResolution};
use crate::traits::liveness::LivenessPolicy;
use crate::traits::packages::PackageManifest;
use crate::traits::visibility::VisibilityModel;
use crate::type_density::{is_python_receiver, measure_colon_after_name};

pub struct Python;

fn python_root_package(specifier: &str) -> String {
    if specifier.starts_with('.') {
        return String::new();
    }
    specifier.split('.').next().unwrap_or("").to_string()
}

impl ImportResolution for Python {
    fn extraction_mode(&self) -> Option<ImportExtractionMode> {
        Some(ImportExtractionMode::TreeSitterFirst)
    }

    fn root_package_from_specifier(&self, specifier: &str) -> String {
        python_root_package(specifier)
    }
}

impl PackageManifest for Python {
    fn root_package_from_specifier(&self, specifier: &str) -> String {
        python_root_package(specifier)
    }

    fn package_name_for_dir(&self, dir: &Path) -> Option<String> {
        let pyproject = dir.join("pyproject.toml");
        if pyproject.is_file()
            && let Ok(content) = fs::read_to_string(&pyproject)
            && let Some(name) = parse_pyproject_name(&content)
        {
            return Some(name.replace('_', "-"));
        }
        let setup_cfg = dir.join("setup.cfg");
        if setup_cfg.is_file()
            && let Ok(content) = fs::read_to_string(&setup_cfg)
        {
            return parse_setup_cfg_name(&content);
        }
        None
    }
}

fn parse_pyproject_name(content: &str) -> Option<String> {
    let table: toml::Value = content.parse().ok()?;
    table
        .get("project")?
        .get("name")?
        .as_str()
        .map(str::to_string)
}

fn parse_setup_cfg_name(content: &str) -> Option<String> {
    let mut in_metadata = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_metadata = trimmed == "[metadata]";
            continue;
        }
        if in_metadata && let Some(rest) = trimmed.strip_prefix("name") {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=') {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

impl LivenessPolicy for Python {
    fn entrypoint_basenames(&self) -> &'static [&'static str] {
        &[
            "main.py",
            "__main__.py",
            "__init__.py",
            "app.py",
            "application.py",
            "wsgi.py",
            "asgi.py",
            "manage.py",
            "setup.py",
            "conftest.py",
        ]
    }

    fn is_test_file(&self, path: &str) -> bool {
        let basename = path.rsplit('/').next().unwrap_or(path);
        let lower = path.to_ascii_lowercase();
        basename.starts_with("test_")
            || basename.starts_with("tests")
            || basename.ends_with("_test.py")
            || lower.contains("/tests/")
            || lower.contains("/test/")
            || lower.contains("/__tests__/")
            || lower.contains("/spec/")
    }

    fn is_conventional_path(&self, path: &str) -> bool {
        path.rsplit('/').next().unwrap_or(path) == "conftest.py"
    }

    fn additional_live_root_files(
        &self,
        root: Option<&Path>,
        all_files: &[String],
    ) -> Vec<String> {
        let Some(root) = root else {
            return Vec::new();
        };
        let mut files = Vec::new();
        for file in all_files {
            if !file.ends_with(".py") {
                continue;
            }
            let abs = root.join(file);
            if let Ok(content) = fs::read_to_string(&abs)
                && content.contains("__name__")
                && content.contains("__main__")
            {
                files.push(file.clone());
            }
        }
        files
    }
}

impl VisibilityModel for Python {
    fn is_restricted(&self, _source_text: &str, name: &str) -> bool {
        name.starts_with('_')
    }
}

impl ComplexityModel for Python {
    fn family(&self) -> LanguageFamily {
        LanguageFamily::ScriptedDynamic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
    fn fallback_type_baseline(&self) -> Option<f64> {
        Some(0.4)
    }
    fn supports_measured_types(&self) -> bool {
        true
    }
    fn measured_type_density(&self, source: &str) -> Option<f64> {
        measure_colon_after_name(source, "python", false, is_python_receiver)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pyproject_name_wins_over_setup_cfg() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("pyproject.toml"),
            "[project]\nname = \"my_lib\"\n",
        )
        .unwrap();
        assert_eq!(
            Python.package_name_for_dir(tmp.path()),
            Some("my-lib".into())
        );
    }

    #[test]
    fn setup_cfg_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("setup.cfg"),
            "[metadata]\nname = other-lib\n",
        )
        .unwrap();
        assert_eq!(
            Python.package_name_for_dir(tmp.path()),
            Some("other-lib".into())
        );
    }
}
