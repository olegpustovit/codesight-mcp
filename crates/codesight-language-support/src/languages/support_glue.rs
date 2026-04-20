//! Glue: `LanguageSupport` impls that tie every per-language struct to its
//! manifest and the axis traits implemented for it.
//!
//! Phase D wires `liveness`, `visibility`, and `complexity` through each
//! concrete struct; axes a language doesn't override fall through to the
//! blanket defaults in `LanguageSupport`. Extraction/imports/packages arrive
//! in later phases.

use crate::languages::braced_static::{CSharp, Cpp, Dart, Java, Kotlin, Swift};
use crate::languages::builtins::{
    AL_MANIFEST, ASM_MANIFEST, BASH_MANIFEST, BLADE_MANIFEST, CPP_MANIFEST, CSHARP_MANIFEST,
    CSS_MANIFEST, C_MANIFEST, DART_MANIFEST, EJS_MANIFEST, ELIXIR_MANIFEST, ERLANG_MANIFEST,
    FORTRAN_MANIFEST, GLEAM_MANIFEST, GO_MANIFEST, JAVASCRIPT_MANIFEST, JAVA_MANIFEST,
    JSON_MANIFEST, JSX_MANIFEST, KOTLIN_MANIFEST, LESS_MANIFEST, LUAU_MANIFEST, LUA_MANIFEST,
    MARKDOWN_MANIFEST, NIX_MANIFEST, PERL_MANIFEST, PHP_MANIFEST, PYTHON_MANIFEST, RAZOR_MANIFEST,
    RUBY_MANIFEST, RUST_MANIFEST, SASS_MANIFEST, SCSS_MANIFEST, SQL_MANIFEST, STYL_MANIFEST,
    SWIFT_MANIFEST, TOML_MANIFEST, TREE_SITTER_QUERY_MANIFEST, TSX_MANIFEST, TYPESCRIPT_MANIFEST,
    VERSE_MANIFEST, VUE_MANIFEST, XML_MANIFEST, YAML_MANIFEST,
};
use crate::languages::declarative::DeclarativeMarkup;
use crate::languages::functional::{Elixir, Erlang};
use crate::languages::go::Go;
use crate::languages::python::Python;
use crate::languages::rust::Rust;
use crate::languages::scripted_dynamic::{Lua, Perl, Php, Ruby};
use crate::languages::typescript::{JavaScript, Jsx, Tsx, TypeScript};
use crate::manifest::{
    self, AL, ASM, BASH, BLADE, CPP, CSHARP, CSS, DART, EJS, ELIXIR, ERLANG, FORTRAN, GLEAM, GO,
    JAVA, JAVASCRIPT, JSON, JSX, KOTLIN, LESS, LUA, LUAU, LanguageId, LanguageManifest, MARKDOWN,
    NIX, PERL, PHP, PYTHON, RAZOR, RUBY, RUST, SASS, SCSS, SQL, STYL, SWIFT, TOML,
    TREE_SITTER_QUERY, TSX, TYPESCRIPT, VERSE, VUE, XML, YAML,
};
use crate::support::LanguageSupport;
use crate::traits::imports::{ImportExtractionMode, ImportResolution};
use crate::traits::packages::PackageManifest;
use crate::traits::{ComplexityModel, LivenessPolicy, VisibilityModel};

/// Wire `id` + `manifest()` + axis trait impls for a language.
///
/// Shapes supported:
/// - `full`: language that overrides all three of liveness/visibility/complexity
/// - `complexity_only`: declarative-markup languages that only carry a family tag
/// - `liveness_complexity`: languages that override liveness + complexity but not visibility
macro_rules! impl_language_support_full {
    ($struct:ident, $id:expr, $manifest:expr) => {
        impl LanguageSupport for $struct {
            fn id(&self) -> LanguageId {
                $id
            }
            fn manifest(&self) -> &'static LanguageManifest {
                &$manifest
            }
            fn liveness(&self) -> &dyn LivenessPolicy {
                self
            }
            fn visibility(&self) -> &dyn VisibilityModel {
                self
            }
            fn complexity(&self) -> &dyn ComplexityModel {
                self
            }
        }
    };
}

/// Full axes + import resolution + package manifest (Phase D.5).
macro_rules! impl_language_support_full_with_imports {
    ($struct:ident, $id:expr, $manifest:expr) => {
        impl LanguageSupport for $struct {
            fn id(&self) -> LanguageId {
                $id
            }
            fn manifest(&self) -> &'static LanguageManifest {
                &$manifest
            }
            fn liveness(&self) -> &dyn LivenessPolicy {
                self
            }
            fn visibility(&self) -> &dyn VisibilityModel {
                self
            }
            fn complexity(&self) -> &dyn ComplexityModel {
                self
            }
            fn imports(&self) -> &dyn ImportResolution {
                self
            }
            fn packages(&self) -> &dyn PackageManifest {
                self
            }
        }
    };
}

/// Full axes + package manifest only (no import resolution).
/// Used by languages whose symbols are extracted but imports aren't
/// (.NET's C# uses `using` but we don't extract those yet).
macro_rules! impl_language_support_full_with_packages {
    ($struct:ident, $id:expr, $manifest:expr) => {
        impl LanguageSupport for $struct {
            fn id(&self) -> LanguageId {
                $id
            }
            fn manifest(&self) -> &'static LanguageManifest {
                &$manifest
            }
            fn liveness(&self) -> &dyn LivenessPolicy {
                self
            }
            fn visibility(&self) -> &dyn VisibilityModel {
                self
            }
            fn complexity(&self) -> &dyn ComplexityModel {
                self
            }
            fn packages(&self) -> &dyn PackageManifest {
                self
            }
        }
    };
}

/// For languages that only own a complexity profile (declarative markup).
/// Backed by a shared zero-size declarative marker so every declarative
/// language can reuse the same value without duplicating the impl.
macro_rules! impl_language_support_declarative {
    ($struct_name:ident, $id:expr, $manifest:expr) => {
        pub struct $struct_name;
        impl LanguageSupport for $struct_name {
            fn id(&self) -> LanguageId {
                $id
            }
            fn manifest(&self) -> &'static LanguageManifest {
                &$manifest
            }
            fn complexity(&self) -> &dyn ComplexityModel {
                &DeclarativeMarkup
            }
        }
    };
}

/// For residual / unsupported languages — identity only, no axes.
macro_rules! impl_language_support_identity {
    ($struct_name:ident, $id:expr, $manifest:expr) => {
        pub struct $struct_name;
        impl LanguageSupport for $struct_name {
            fn id(&self) -> LanguageId {
                $id
            }
            fn manifest(&self) -> &'static LanguageManifest {
                &$manifest
            }
        }
    };
}

impl_language_support_full_with_imports!(Rust, RUST, RUST_MANIFEST);
impl_language_support_full_with_imports!(Python, PYTHON, PYTHON_MANIFEST);
impl_language_support_full_with_imports!(TypeScript, TYPESCRIPT, TYPESCRIPT_MANIFEST);
impl_language_support_full_with_imports!(Tsx, TSX, TSX_MANIFEST);
impl_language_support_full_with_imports!(JavaScript, JAVASCRIPT, JAVASCRIPT_MANIFEST);
impl_language_support_full_with_imports!(Jsx, JSX, JSX_MANIFEST);
impl_language_support_full_with_imports!(Go, GO, GO_MANIFEST);
impl_language_support_full!(Java, JAVA, JAVA_MANIFEST);
impl_language_support_full_with_packages!(CSharp, CSHARP, CSHARP_MANIFEST);
impl_language_support_full!(Cpp, CPP, CPP_MANIFEST);
impl_language_support_full_with_imports!(Dart, DART, DART_MANIFEST);
impl_language_support_full!(Kotlin, KOTLIN, KOTLIN_MANIFEST);
impl_language_support_full!(Swift, SWIFT, SWIFT_MANIFEST);
impl_language_support_full!(Ruby, RUBY, RUBY_MANIFEST);
impl_language_support_full!(Perl, PERL, PERL_MANIFEST);
impl_language_support_full!(Lua, LUA, LUA_MANIFEST);
impl_language_support_full!(Php, PHP, PHP_MANIFEST);
impl_language_support_full!(Elixir, ELIXIR, ELIXIR_MANIFEST);
impl_language_support_full!(Erlang, ERLANG, ERLANG_MANIFEST);

// C shares CPP's complexity profile but is its own language. The CPP struct
// in braced_static.rs already owns `BracedStatic` semantics; we expose `C`
// here as its own zero-size struct so the registry can distinguish it.
pub struct C;
impl LanguageSupport for C {
    fn id(&self) -> LanguageId {
        manifest::C
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &C_MANIFEST
    }
    fn complexity(&self) -> &dyn ComplexityModel {
        &CComplexity
    }
}

pub struct CComplexity;
impl ComplexityModel for CComplexity {
    fn family(&self) -> crate::traits::complexity::LanguageFamily {
        crate::traits::complexity::LanguageFamily::BracedStatic
    }
    fn nesting_mode(&self) -> crate::traits::complexity::NestingMode {
        crate::traits::complexity::NestingMode::Brace
    }
}

// Declarative markup / config languages: carry a family tag for nesting &
// function-length gating, no visibility or liveness.
impl_language_support_declarative!(JsonSupport, JSON, JSON_MANIFEST);
impl_language_support_declarative!(YamlSupport, YAML, YAML_MANIFEST);
impl_language_support_declarative!(XmlSupport, XML, XML_MANIFEST);
impl_language_support_declarative!(TomlSupport, TOML, TOML_MANIFEST);
// SQL: declarative markup + fallback-only imports. Since the declarative
// macro writes a LanguageSupport impl we can't add `imports()` through a
// second impl block, so we write the struct + impl manually.
pub struct SqlSupport;
impl LanguageSupport for SqlSupport {
    fn id(&self) -> LanguageId {
        SQL
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &SQL_MANIFEST
    }
    fn complexity(&self) -> &dyn ComplexityModel {
        &DeclarativeMarkup
    }
    fn imports(&self) -> &dyn ImportResolution {
        &FALLBACK_ONLY_IMPORTS
    }
}
impl_language_support_declarative!(BladeSupport, BLADE, BLADE_MANIFEST);
impl_language_support_declarative!(RazorSupport, RAZOR, RAZOR_MANIFEST);

// Identity-only shells for languages that don't (yet) participate in
// agent-readiness: CSS family, Bash, Asm, EJS, AL, Markdown, Tree-sitter-query,
// Fortran, Gleam, Nix, Verse, Vue.
impl_language_support_identity!(CssSupport, CSS, CSS_MANIFEST);
impl_language_support_identity!(ScssSupport, SCSS, SCSS_MANIFEST);
impl_language_support_identity!(LessSupport, LESS, LESS_MANIFEST);
impl_language_support_identity!(SassSupport, SASS, SASS_MANIFEST);
impl_language_support_identity!(StylSupport, STYL, STYL_MANIFEST);
impl_language_support_identity!(BashSupport, BASH, BASH_MANIFEST);
impl_language_support_identity!(AsmSupport, ASM, ASM_MANIFEST);
impl_language_support_identity!(EjsSupport, EJS, EJS_MANIFEST);
impl_language_support_identity!(AlSupport, AL, AL_MANIFEST);
impl_language_support_identity!(MarkdownSupport, MARKDOWN, MARKDOWN_MANIFEST);
impl_language_support_identity!(
    TreeSitterQuerySupport,
    TREE_SITTER_QUERY,
    TREE_SITTER_QUERY_MANIFEST
);
impl_language_support_identity!(FortranSupport, FORTRAN, FORTRAN_MANIFEST);
impl_language_support_identity!(GleamSupport, GLEAM, GLEAM_MANIFEST);
impl_language_support_identity!(NixSupport, NIX, NIX_MANIFEST);
impl_language_support_identity!(VerseSupport, VERSE, VERSE_MANIFEST);
// Vue: wrapper language, carries fallback-only import extraction. Separate
// from the identity-only macro because we override `imports()`.
pub struct VueSupport;
impl LanguageSupport for VueSupport {
    fn id(&self) -> LanguageId {
        VUE
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &VUE_MANIFEST
    }
    fn imports(&self) -> &dyn ImportResolution {
        &FALLBACK_ONLY_IMPORTS
    }
}
impl_language_support_identity!(LuauSupport, LUAU, LUAU_MANIFEST);

/// Shared zero-size adapter used by languages whose import extraction is
/// line-scanner-only (`FallbackOnly`): Vue, SQL. Avoids duplicating the tiny
/// `ImportResolution` impl in each language file.
pub struct FallbackOnlyImports;

impl ImportResolution for FallbackOnlyImports {
    fn extraction_mode(&self) -> Option<ImportExtractionMode> {
        Some(ImportExtractionMode::FallbackOnly)
    }
}

pub static FALLBACK_ONLY_IMPORTS: FallbackOnlyImports = FallbackOnlyImports;
