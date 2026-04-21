//! Markdown language.

use crate::languages::builtins::MARKDOWN_MANIFEST;
use crate::manifest::{LanguageId, LanguageManifest, MARKDOWN};
use crate::traits::Language;

pub struct MarkdownSupport;

impl Language for MarkdownSupport {
    fn id(&self) -> LanguageId {
        MARKDOWN
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &MARKDOWN_MANIFEST
    }
}
