//! Tree-sitter query language (.scm files).

use crate::languages::builtins::TREE_SITTER_QUERY_MANIFEST;
use crate::manifest::{LanguageId, LanguageManifest, TREE_SITTER_QUERY};
use crate::traits::Language;

pub struct TreeSitterQuerySupport;

impl Language for TreeSitterQuerySupport {
    fn id(&self) -> LanguageId {
        TREE_SITTER_QUERY
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &TREE_SITTER_QUERY_MANIFEST
    }
    fn is_reference_target(&self) -> bool {
        false
    }
}
