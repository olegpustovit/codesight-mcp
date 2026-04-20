//! Builtin language manifests.
//!
//! Phase C wires `GrammarSpec` into every curated manifest. Grammar
//! statics live in `crate::grammars`; manifests reference them as
//! `grammar: Some(&RUST_GRAMMAR)`. Residual languages (blade, razor,
//! verse, …) that rely on hand-rolled extractors keep `grammar: None`.

use std::sync::Arc;

use crate::grammars::{
    BASH_GRAMMAR, CPP_GRAMMAR, CSHARP_GRAMMAR, CSS_GRAMMAR, DART_GRAMMAR, EJS_GRAMMAR,
    ELIXIR_GRAMMAR, ERLANG_GRAMMAR, FORTRAN_GRAMMAR, GLEAM_GRAMMAR, GO_GRAMMAR, JAVASCRIPT_GRAMMAR,
    JAVA_GRAMMAR, JSON_GRAMMAR, KOTLIN_GRAMMAR, LUAU_GRAMMAR, LUA_GRAMMAR, NIX_GRAMMAR,
    PERL_GRAMMAR, PHP_GRAMMAR, PYTHON_GRAMMAR, RUBY_GRAMMAR, RUST_GRAMMAR, SCSS_GRAMMAR,
    SQL_GRAMMAR, SWIFT_GRAMMAR, TSX_GRAMMAR, TYPESCRIPT_GRAMMAR, XML_GRAMMAR, YAML_GRAMMAR,
};
use crate::languages::braced_static::{CSharp, Cpp, Dart, Java, Kotlin, Swift};
use crate::languages::cpp::cpp_header_content_probe;
use crate::languages::functional::{Elixir, Erlang};
use crate::languages::go::Go;
use crate::languages::python::Python;
use crate::languages::rust::Rust;
use crate::languages::scripted_dynamic::{Lua, Perl, Php, Ruby};
use crate::languages::support_glue::{
    AlSupport, AsmSupport, BashSupport, BladeSupport, C, CssSupport, EjsSupport, FortranSupport,
    GleamSupport, JsonSupport, LessSupport, LuauSupport, MarkdownSupport, NixSupport, RazorSupport,
    SassSupport, ScssSupport, SqlSupport, StylSupport, TomlSupport, TreeSitterQuerySupport,
    VerseSupport, VueSupport, XmlSupport, YamlSupport,
};
use crate::languages::typescript::{JavaScript, Jsx, Tsx, TypeScript};
use crate::languages::yaml::{JSON_PROFILES, YAML_PROFILES};
use crate::manifest::{
    BASH, CPP, CSHARP, CSS, DART, ELIXIR, GO, JAVA, JAVASCRIPT, JSON, KOTLIN, LUA, LUAU,
    BackendIds, LanguageManifest, PERL, PHP, PYTHON, RUBY, RUST, SCSS, SQL, SWIFT, TSX,
    TYPESCRIPT, XML, YAML,
};
use crate::registry::LanguageRegistry;

/// One manifest per builtin language. Order is not significant — the
/// registry indexes by identifier, extension, filename, and compound
/// extension.
macro_rules! manifest {
    (
        $name:ident,
        display = $display:expr,
        aliases = $aliases:expr,
        ext = $ext:expr,
        filenames = $fnames:expr,
        compound = $compound:expr,
        grammar = $grammar:expr,
        lsp = $lsp:expr,
        profiles = $profiles:expr,
        probe = $probe:expr $(,)?
    ) => {
        pub(crate) static $name: LanguageManifest = LanguageManifest {
            display_name: $display,
            aliases: $aliases,
            extensions: $ext,
            filename_overrides: $fnames,
            compound_extensions: $compound,
            grammar: $grammar,
            backend_ids: BackendIds { lsp: $lsp },
            profiles: $profiles,
            content_probe: $probe,
        };
    };
}

manifest!(RUST_MANIFEST,
    display = "Rust", aliases = &["rs"], ext = &["rs"],
    filenames = &[], compound = &[],
    grammar = Some(&RUST_GRAMMAR),
    lsp = Some("rust"), profiles = &[], probe = None);

manifest!(TYPESCRIPT_MANIFEST,
    display = "TypeScript", aliases = &["ts"],
    ext = &["ts", "mts", "cts"], filenames = &[], compound = &["d.ts"],
    grammar = Some(&TYPESCRIPT_GRAMMAR),
    lsp = Some("typescript"), profiles = &[], probe = None);

manifest!(TSX_MANIFEST,
    display = "TSX", aliases = &[], ext = &["tsx"],
    filenames = &[], compound = &[],
    grammar = Some(&TSX_GRAMMAR),
    lsp = Some("tsx"), profiles = &[], probe = None);

manifest!(JAVASCRIPT_MANIFEST,
    display = "JavaScript", aliases = &["js"],
    ext = &["js", "mjs", "cjs"], filenames = &[], compound = &[],
    grammar = Some(&JAVASCRIPT_GRAMMAR),
    lsp = Some("javascript"), profiles = &[], probe = None);

manifest!(JSX_MANIFEST,
    display = "JSX", aliases = &[], ext = &["jsx"],
    filenames = &[], compound = &[],
    grammar = None,
    lsp = None, profiles = &[], probe = None);

manifest!(PYTHON_MANIFEST,
    display = "Python", aliases = &["py"], ext = &["py", "pyi"],
    filenames = &[], compound = &[],
    grammar = Some(&PYTHON_GRAMMAR),
    lsp = Some("python"), profiles = &[], probe = None);

manifest!(GO_MANIFEST,
    display = "Go", aliases = &["golang"], ext = &["go"],
    filenames = &[], compound = &[],
    grammar = Some(&GO_GRAMMAR),
    lsp = Some("go"), profiles = &[], probe = None);

manifest!(JAVA_MANIFEST,
    display = "Java", aliases = &[], ext = &["java"],
    filenames = &[], compound = &[],
    grammar = Some(&JAVA_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(RUBY_MANIFEST,
    display = "Ruby", aliases = &["rb"], ext = &["rb"],
    filenames = &[], compound = &[],
    grammar = Some(&RUBY_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(PHP_MANIFEST,
    display = "PHP", aliases = &[], ext = &["php"],
    filenames = &[], compound = &[],
    grammar = Some(&PHP_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(CSHARP_MANIFEST,
    display = "C#", aliases = &["c_sharp", "cs"], ext = &["cs"],
    filenames = &[], compound = &[],
    grammar = Some(&CSHARP_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(CPP_MANIFEST,
    display = "C++", aliases = &[],
    ext = &["cc", "cpp", "cxx", "hpp", "hh", "hxx", "h"],
    filenames = &[], compound = &[],
    grammar = Some(&CPP_GRAMMAR),
    lsp = None, profiles = &[],
    probe = Some(cpp_header_content_probe));

manifest!(C_MANIFEST,
    display = "C", aliases = &[], ext = &["c"],
    filenames = &[], compound = &[],
    grammar = None,
    lsp = None, profiles = &[], probe = None);

manifest!(SWIFT_MANIFEST,
    display = "Swift", aliases = &[], ext = &["swift"],
    filenames = &[], compound = &[],
    grammar = Some(&SWIFT_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(KOTLIN_MANIFEST,
    display = "Kotlin", aliases = &[], ext = &["kt", "kts"],
    filenames = &[], compound = &[],
    grammar = Some(&KOTLIN_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(DART_MANIFEST,
    display = "Dart", aliases = &[], ext = &["dart"],
    filenames = &[], compound = &[],
    grammar = Some(&DART_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(BASH_MANIFEST,
    display = "Bash", aliases = &["sh"], ext = &["sh", "bash"],
    filenames = &[], compound = &[],
    grammar = Some(&BASH_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(SQL_MANIFEST,
    display = "SQL", aliases = &[], ext = &["sql"],
    filenames = &[], compound = &[],
    grammar = Some(&SQL_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(YAML_MANIFEST,
    display = "YAML", aliases = &["yml"], ext = &["yaml", "yml"],
    filenames = &[], compound = &[],
    grammar = Some(&YAML_GRAMMAR),
    lsp = Some("yaml"), profiles = YAML_PROFILES, probe = None);

manifest!(JSON_MANIFEST,
    display = "JSON", aliases = &[], ext = &["json"],
    filenames = &[], compound = &[],
    grammar = Some(&JSON_GRAMMAR),
    lsp = None, profiles = JSON_PROFILES, probe = None);

manifest!(XML_MANIFEST,
    display = "XML", aliases = &[], ext = &["xml", "svg"],
    filenames = &[], compound = &[],
    grammar = Some(&XML_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(CSS_MANIFEST,
    display = "CSS", aliases = &[], ext = &["css"],
    filenames = &[], compound = &[],
    grammar = Some(&CSS_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(SCSS_MANIFEST,
    display = "SCSS", aliases = &[], ext = &["scss"],
    filenames = &[], compound = &[],
    grammar = Some(&SCSS_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(LESS_MANIFEST,
    display = "Less", aliases = &[], ext = &["less"],
    filenames = &[], compound = &[],
    grammar = None,
    lsp = None, profiles = &[], probe = None);

manifest!(SASS_MANIFEST,
    display = "Sass", aliases = &[], ext = &["sass"],
    filenames = &[], compound = &[],
    grammar = None,
    lsp = None, profiles = &[], probe = None);

manifest!(STYL_MANIFEST,
    display = "Stylus", aliases = &[], ext = &["styl"],
    filenames = &[], compound = &[],
    grammar = None,
    lsp = None, profiles = &[], probe = None);

manifest!(LUA_MANIFEST,
    display = "Lua", aliases = &[], ext = &["lua"],
    filenames = &[], compound = &[],
    grammar = Some(&LUA_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(LUAU_MANIFEST,
    display = "Luau", aliases = &[], ext = &["luau"],
    filenames = &[], compound = &[],
    grammar = Some(&LUAU_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(NIX_MANIFEST,
    display = "Nix", aliases = &[], ext = &["nix"],
    filenames = &[], compound = &[],
    grammar = Some(&NIX_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(PERL_MANIFEST,
    display = "Perl", aliases = &[], ext = &["pl", "pm", "psgi", "cgi"],
    filenames = &[], compound = &[],
    grammar = Some(&PERL_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(ASM_MANIFEST,
    display = "Assembly", aliases = &[],
    ext = &["asm", "s", "65816", "inc", "z80", "spc"],
    filenames = &[], compound = &[],
    grammar = None,
    lsp = None, profiles = &[], probe = None);

manifest!(BLADE_MANIFEST,
    display = "Blade", aliases = &[], ext = &[],
    filenames = &[], compound = &["blade.php"],
    grammar = None,
    lsp = None, profiles = &[], probe = None);

manifest!(RAZOR_MANIFEST,
    display = "Razor", aliases = &[], ext = &["cshtml", "razor"],
    filenames = &[], compound = &[],
    grammar = None,
    lsp = None, profiles = &[], probe = None);

manifest!(EJS_MANIFEST,
    display = "EJS", aliases = &["embedded_template"], ext = &["ejs"],
    filenames = &[], compound = &[],
    grammar = Some(&EJS_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(VERSE_MANIFEST,
    display = "Verse", aliases = &[], ext = &["verse"],
    filenames = &[], compound = &[],
    grammar = None,
    lsp = None, profiles = &[], probe = None);

manifest!(ELIXIR_MANIFEST,
    display = "Elixir", aliases = &[], ext = &["ex", "exs"],
    filenames = &[], compound = &[],
    grammar = Some(&ELIXIR_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(ERLANG_MANIFEST,
    display = "Erlang", aliases = &[], ext = &["erl", "hrl"],
    filenames = &[], compound = &[],
    grammar = Some(&ERLANG_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(FORTRAN_MANIFEST,
    display = "Fortran", aliases = &[],
    ext = &["f", "f90", "f95", "f03", "f08"],
    filenames = &[], compound = &[],
    grammar = Some(&FORTRAN_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(GLEAM_MANIFEST,
    display = "Gleam", aliases = &[], ext = &["gleam"],
    filenames = &[], compound = &[],
    grammar = Some(&GLEAM_GRAMMAR),
    lsp = None, profiles = &[], probe = None);

manifest!(VUE_MANIFEST,
    display = "Vue", aliases = &[], ext = &["vue"],
    filenames = &[], compound = &[],
    grammar = None,
    lsp = None, profiles = &[], probe = None);

manifest!(AL_MANIFEST,
    display = "AL", aliases = &[], ext = &["al"],
    filenames = &[], compound = &[],
    grammar = None,
    lsp = None, profiles = &[], probe = None);

manifest!(MARKDOWN_MANIFEST,
    display = "Markdown", aliases = &[], ext = &["md", "markdown"],
    filenames = &[], compound = &[],
    grammar = None,
    lsp = None, profiles = &[], probe = None);

manifest!(TOML_MANIFEST,
    display = "TOML", aliases = &[], ext = &["toml"],
    filenames = &[], compound = &[],
    grammar = None,
    lsp = None, profiles = &[], probe = None);

manifest!(TREE_SITTER_QUERY_MANIFEST,
    display = "Tree-sitter query", aliases = &[], ext = &["scm"],
    filenames = &[], compound = &[],
    grammar = None,
    lsp = None, profiles = &[], probe = None);

const CURATED: &[crate::manifest::LanguageId] = &[
    RUST, TYPESCRIPT, TSX, JAVASCRIPT, PYTHON, GO, JAVA, RUBY, PHP, CSHARP, CPP, SWIFT, KOTLIN,
    DART, BASH, SQL, YAML, JSON, XML, CSS, SCSS, LUA, LUAU, PERL, ELIXIR,
];

/// Register every builtin language in `registry`, wiring each `LanguageId`
/// to its per-language `LanguageSupport` impl. Curated languages are those
/// with a maintained query pack under `runtime/queries/<lang>/`; the registry
/// exposes that set via `curated_languages()` so §7.2.3 curated-language
/// parity stays stable as extractor tiers move.
pub fn register_builtins(registry: &mut LanguageRegistry) {
    registry.register(Arc::new(Rust));
    registry.register(Arc::new(TypeScript));
    registry.register(Arc::new(Tsx));
    registry.register(Arc::new(JavaScript));
    registry.register(Arc::new(Jsx));
    registry.register(Arc::new(Python));
    registry.register(Arc::new(Go));
    registry.register(Arc::new(Java));
    registry.register(Arc::new(Ruby));
    registry.register(Arc::new(Php));
    registry.register(Arc::new(CSharp));
    registry.register(Arc::new(Cpp));
    registry.register(Arc::new(C));
    registry.register(Arc::new(Swift));
    registry.register(Arc::new(Kotlin));
    registry.register(Arc::new(Dart));
    registry.register(Arc::new(BashSupport));
    registry.register(Arc::new(SqlSupport));
    registry.register(Arc::new(YamlSupport));
    registry.register(Arc::new(JsonSupport));
    registry.register(Arc::new(XmlSupport));
    registry.register(Arc::new(CssSupport));
    registry.register(Arc::new(ScssSupport));
    registry.register(Arc::new(LessSupport));
    registry.register(Arc::new(SassSupport));
    registry.register(Arc::new(StylSupport));
    registry.register(Arc::new(Lua));
    registry.register(Arc::new(LuauSupport));
    registry.register(Arc::new(NixSupport));
    registry.register(Arc::new(Perl));
    registry.register(Arc::new(AsmSupport));
    registry.register(Arc::new(BladeSupport));
    registry.register(Arc::new(RazorSupport));
    registry.register(Arc::new(EjsSupport));
    registry.register(Arc::new(VerseSupport));
    registry.register(Arc::new(Elixir));
    registry.register(Arc::new(Erlang));
    registry.register(Arc::new(FortranSupport));
    registry.register(Arc::new(GleamSupport));
    registry.register(Arc::new(VueSupport));
    registry.register(Arc::new(AlSupport));
    registry.register(Arc::new(MarkdownSupport));
    registry.register(Arc::new(TomlSupport));
    registry.register(Arc::new(TreeSitterQuerySupport));
    for id in CURATED {
        registry.mark_curated(*id);
    }
}
