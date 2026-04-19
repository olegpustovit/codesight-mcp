//! `.codesight.lsp.toml` loader.
//!
//! Shape:
//!
//! ```toml
//! [rust]
//! command = "rust-analyzer"
//! file_extensions = ["rs"]
//! args = []
//! initialization_options = {}
//! timeout_ms = 3000
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

fn default_timeout_ms() -> u64 {
    3000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageSpec {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub file_extensions: Vec<String>,
    #[serde(default)]
    pub initialization_options: serde_json::Value,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LspConfig {
    #[serde(flatten)]
    pub languages: BTreeMap<String, LanguageSpec>,
}

impl LspConfig {
    pub fn from_toml_str(raw: &str) -> anyhow::Result<Self> {
        Ok(toml::from_str(raw)?)
    }

    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        Self::from_toml_str(&raw)
    }

    /// Bundled rust-analyzer profile so binaries don't require a config
    /// file for the common case.
    pub fn rust_analyzer_default() -> Self {
        let mut cfg = Self::default();
        cfg.languages.insert(
            "rust".into(),
            LanguageSpec {
                command: "rust-analyzer".into(),
                args: Vec::new(),
                file_extensions: vec!["rs".into()],
                initialization_options: serde_json::json!({}),
                timeout_ms: default_timeout_ms(),
            },
        );
        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rust_analyzer_block() {
        let toml = r#"
[rust]
command = "rust-analyzer"
file_extensions = ["rs"]
"#;
        let cfg = LspConfig::from_toml_str(toml).unwrap();
        assert_eq!(cfg.languages.len(), 1);
        let rust = &cfg.languages["rust"];
        assert_eq!(rust.command, "rust-analyzer");
        assert_eq!(rust.file_extensions, vec!["rs".to_string()]);
        assert_eq!(rust.timeout_ms, 3000);
    }

    #[test]
    fn default_profile_includes_rust() {
        let cfg = LspConfig::rust_analyzer_default();
        assert!(cfg.languages.contains_key("rust"));
    }
}
