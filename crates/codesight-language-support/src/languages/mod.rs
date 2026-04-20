//! Per-language implementations (LANGUAGES.md §2).
//!
//! Each `<lang>/` module carries the manifest and the axis-trait impls the
//! language owns. `support_glue` wires `LanguageSupport` to the concrete
//! structs; `builtins` registers them into the process-wide registry.

pub mod braced_static;
pub mod builtins;
pub mod cpp;
pub mod declarative;
pub mod functional;
pub mod go;
pub mod php;
pub mod python;
pub mod rust;
pub mod scripted_dynamic;
pub mod sql;
pub mod support_glue;
pub mod typescript;
pub mod vue;
pub mod yaml;

pub use builtins::register_builtins;
