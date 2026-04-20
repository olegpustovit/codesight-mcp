//! Scripted-dynamic family with indent-based nesting (Ruby, Perl, Lua).

use crate::traits::complexity::{ComplexityModel, LanguageFamily, NestingMode};
use crate::traits::liveness::LivenessPolicy;
use crate::traits::visibility::VisibilityModel;

pub struct Ruby;
pub struct Perl;
pub struct Lua;
pub struct Php;

fn underscore_restricted(_source_text: &str, name: &str) -> bool {
    name.starts_with('_')
}

impl LivenessPolicy for Ruby {
    fn is_test_file(&self, path: &str) -> bool {
        let lower = path.to_ascii_lowercase();
        lower.ends_with("_spec.rb") || lower.contains("/spec/")
    }
}
impl VisibilityModel for Ruby {
    fn is_restricted(&self, source_text: &str, name: &str) -> bool {
        underscore_restricted(source_text, name)
    }
}
impl ComplexityModel for Ruby {
    fn family(&self) -> LanguageFamily {
        LanguageFamily::ScriptedDynamic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
    fn fallback_type_baseline(&self) -> Option<f64> {
        Some(0.1)
    }
}

impl LivenessPolicy for Perl {}
impl VisibilityModel for Perl {
    fn is_restricted(&self, source_text: &str, name: &str) -> bool {
        underscore_restricted(source_text, name)
    }
}
impl ComplexityModel for Perl {
    fn family(&self) -> LanguageFamily {
        LanguageFamily::ScriptedDynamic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
    fn fallback_type_baseline(&self) -> Option<f64> {
        Some(0.1)
    }
}

impl LivenessPolicy for Lua {}
impl VisibilityModel for Lua {
    fn is_restricted(&self, source_text: &str, name: &str) -> bool {
        underscore_restricted(source_text, name)
    }
}
impl ComplexityModel for Lua {
    fn family(&self) -> LanguageFamily {
        LanguageFamily::ScriptedDynamic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
    fn fallback_type_baseline(&self) -> Option<f64> {
        Some(0.1)
    }
}

// PHP uses braces but scripted-dynamic family, with `public/private/protected`
// visibility, a `Type $name` signature shape, and a 0.3 type baseline.
impl LivenessPolicy for Php {}
impl VisibilityModel for Php {
    fn is_restricted(&self, source_text: &str, _name: &str) -> bool {
        let trimmed = source_text.trim_start();
        if trimmed.starts_with("private ")
            || trimmed.starts_with("protected ")
            || trimmed.starts_with("internal ")
        {
            return true;
        }
        !trimmed.starts_with("public ")
    }
}
impl ComplexityModel for Php {
    fn family(&self) -> LanguageFamily {
        LanguageFamily::ScriptedDynamic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Brace
    }
    fn fallback_type_baseline(&self) -> Option<f64> {
        Some(0.3)
    }
    fn supports_measured_types(&self) -> bool {
        true
    }
    fn measured_type_density(&self, source: &str) -> Option<f64> {
        crate::type_density::measure_php_signature(source)
    }
}
