//! `SimpleLanguage` — the default shape (LANGUAGES.md §2.2 case 1).
//!
//! Grammar + builtin query pack + every default heuristic. One value per
//! language; nothing more. The vast majority of builtin languages end up
//! as `SimpleLanguage` entries in Phase C.

use crate::manifest::{LanguageId, LanguageManifest};
use crate::support::LanguageSupport;

pub struct SimpleLanguage {
    pub id: LanguageId,
    pub manifest: &'static LanguageManifest,
}

impl LanguageSupport for SimpleLanguage {
    fn id(&self) -> LanguageId {
        self.id
    }
    fn manifest(&self) -> &'static LanguageManifest {
        self.manifest
    }
}
