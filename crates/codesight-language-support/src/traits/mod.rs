//! The six analysis axes (LANGUAGES.md §3.1 / §4).
//!
//! Each axis is a trait with a default no-op / permissive / default-complexity
//! implementation. A language only implements the axes whose behavior it
//! actually overrides — everything else falls through to the defaults via
//! `LanguageSupport`'s provided methods.

pub mod complexity;
pub mod extraction;
pub mod imports;
pub mod liveness;
pub mod packages;
pub mod visibility;

pub use complexity::{ComplexityModel, DefaultComplexity, LanguageFamily, NestingMode};
pub use extraction::{NoopExtraction, SymbolExtraction};
pub use imports::{ImportExtractionMode, ImportResolution, NoopImports};
pub use liveness::{LivenessPolicy, NoopLiveness};
pub use packages::{NoopPackages, PackageManifest};
pub use visibility::{PermissiveVisibility, VisibilityModel};
