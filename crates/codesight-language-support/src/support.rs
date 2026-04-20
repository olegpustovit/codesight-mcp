//! The top-level `LanguageSupport` trait (LANGUAGES.md §3.1).
//!
//! Every language implementation — simple, folder, host-profile-carrying,
//! wrapper, or manual — implements `LanguageSupport`. Downstream crates
//! always obtain a `&dyn LanguageSupport` from `LanguageRegistry`, never a
//! typed concrete struct.

use crate::manifest::{LanguageId, LanguageManifest};
use crate::traits::{
    ComplexityModel, DefaultComplexity, ImportResolution, LivenessPolicy, NoopExtraction,
    NoopImports, NoopLiveness, NoopPackages, PackageManifest, PermissiveVisibility,
    SymbolExtraction, VisibilityModel,
};

/// The only trait crates above L0.75 are allowed to hold.
///
/// Provided defaults keep simple languages zero-boilerplate: only override
/// the axes whose behavior actually differs from the defaults.
pub trait LanguageSupport: Send + Sync {
    fn id(&self) -> LanguageId;
    fn manifest(&self) -> &'static LanguageManifest;

    fn extraction(&self) -> &dyn SymbolExtraction {
        &NoopExtraction
    }
    fn imports(&self) -> &dyn ImportResolution {
        &NoopImports
    }
    fn packages(&self) -> &dyn PackageManifest {
        &NoopPackages
    }
    fn liveness(&self) -> &dyn LivenessPolicy {
        &NoopLiveness
    }
    fn visibility(&self) -> &dyn VisibilityModel {
        &PermissiveVisibility
    }
    fn complexity(&self) -> &dyn ComplexityModel {
        &DefaultComplexity
    }
}
